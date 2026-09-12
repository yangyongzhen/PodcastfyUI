//! LLM dialogue (Q&A transcript) generation.
//!
//! Providers: OpenAI-compatible chat (covers OpenAI + any compatible server),
//! Anthropic, Gemini, and Ollama. Prompt design mirrors the upstream
//! `content_generator.py`: two hosts with fixed roles discussing source content.

use crate::config::ConversationConfig;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "openai-compatible" | "openai_compatible" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            "ollama" => Some(Self::Ollama),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    pub provider: Provider,
    pub model: String,
    pub api_key: Option<String>,
    /// Custom base URL for OpenAI-compatible servers / Ollama / Gemini.
    pub base_url: Option<String>,
    /// Sampling temperature (upstream `creativity` is 0..=1).
    pub temperature: f32,
    /// Max output tokens per request.
    pub max_tokens: u32,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            provider: Provider::OpenAi,
            model: "gpt-4o-mini".into(),
            api_key: None,
            base_url: None,
            temperature: 0.8,
            max_tokens: 4096,
        }
    }
}

/// Progress callback: (stage, percent 0..=100).
pub type ProgressFn = dyn Fn(&str, u8) + Send + Sync;

pub struct Generator {
    cfg: GeneratorConfig,
    http: reqwest::Client,
}

impl Generator {
    pub fn new(cfg: GeneratorConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600)) // long generations
            .build()
            .expect("http client");
        Self { cfg, http }
    }

    /// Minimal real request, used by Settings → 「测试连接」.
    pub async fn ping(&self) -> Result<String, String> {
        self.chat(&[
            ("system", "You are a connectivity probe."),
            ("user", "Reply with the single word: OK"),
        ])
        .await
    }

    /// Generate the full two-host dialogue transcript.
    ///
    /// `content` is the combined, cleaned source material.
    /// Writes the transcript to `output_path` and returns it.
    pub async fn generate_qa(
        &self,
        content: &str,
        conv: &ConversationConfig,
        longform: bool,
        output_path: &std::path::Path,
        progress: &ProgressFn,
    ) -> Result<String, String> {
        let transcript = if longform {
            self.generate_longform(content, conv, progress).await?
        } else {
            self.generate_short(content, conv, progress).await?
        };

        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        tokio::fs::write(output_path, &transcript)
            .await
            .map_err(|e| e.to_string())?;
        Ok(transcript)
    }

    /// Short-form (2–5 min): one request covering the whole source.
    async fn generate_short(
        &self,
        content: &str,
        conv: &ConversationConfig,
        progress: &ProgressFn,
    ) -> Result<String, String> {
        progress("writing dialogue…", 35);
        let prompt = build_prompt(content, conv, false, None);
        let raw = self.chat(&[("system", &system_prompt(conv)), ("user", &prompt)]).await?;
        let transcript = format_transcript(raw, conv);
        progress("dialogue complete", 50);
        Ok(transcript)
    }

    /// Long-form (30+ min): split content into chunks and run a rolling
    /// multi-round discussion, feeding the previous round back as context.
    async fn generate_longform(
        &self,
        content: &str,
        conv: &ConversationConfig,
        progress: &ProgressFn,
    ) -> Result<String, String> {
        let chunks = split_chunks(content, conv.max_num_chunks, conv.min_chunk_size);
        if chunks.is_empty() {
            return Err("content too short for longform generation".into());
        }
        progress(&format!("writing round 1/{total}", total = chunks.len()), 35);

        let mut transcript = String::new();
        let mut prev_round: Option<String> = None;
        for (i, chunk) in chunks.iter().enumerate() {
            let prompt = build_prompt(chunk, conv, true, prev_round.as_deref());
            let raw = self
                .chat(
                    &[("system", &system_prompt(conv)), ("user", &prompt)],
                )
                .await
                .map_err(|e| format!("round {}/{} failed: {e}", i + 1, chunks.len()))?;
            let round = format_transcript(raw, conv);
            transcript.push_str(&round);
            transcript.push('\n');

            // Keep the previous round short so context stays bounded.
            prev_round = Some(tail_chars(&round, 4000));

            let pct = 35u8 + (50 * (i + 1) / chunks.len()) as u8;
            if i + 1 < chunks.len() {
                progress(&format!("writing round {}/{}", i + 2, chunks.len()), pct);
            } else {
                progress("dialogue complete", 50);
            }
        }
        Ok(transcript)
    }

    /// Single chat completion across providers.
    async fn chat(&self, messages: &[(&str, &str)]) -> Result<String, String> {
        let msgs: Vec<Value> = messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();

        match self.cfg.provider {
            Provider::OpenAi | Provider::Ollama => {
                let base = match (self.cfg.provider, self.cfg.base_url.as_deref()) {
                    (Provider::Ollama, Some(b)) => b.trim_end_matches('/').to_string(),
                    (Provider::Ollama, None) => "http://localhost:11434/v1".into(),
                    (Provider::OpenAi, Some(b)) => b.trim_end_matches('/').to_string(),
                    (Provider::OpenAi, None) => "https://api.openai.com/v1".into(),
                    // Anthropic / Gemini don't use this base; unreachable here.
                    _ => "https://api.openai.com/v1".into(),
                };
                let url = format!("{base}/chat/completions");
                let mut req = self.http.post(&url).json(&json!({
                    "model": self.cfg.model,
                    "messages": msgs,
                    "temperature": self.cfg.temperature,
                    "max_tokens": self.cfg.max_tokens,
                }));
                if let Some(key) = &self.cfg.api_key {
                    req = req.header("Authorization", format!("Bearer {key}"));
                }
                let resp = req.send().await.map_err(|e| e.to_string())?;
                let status = resp.status();
                let body: Value = resp.json().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(format!("llm {status}: {}", body.get("error").unwrap_or(&body).to_string()));
                }
                body.pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| "no content in llm response".into())
            }
            Provider::Anthropic => {
                let url = "https://api.anthropic.com/v1/messages";
                let system: String = messages
                    .iter()
                    .filter(|(r, _)| *r == "system")
                    .map(|(_, c)| c.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let rest: Vec<Value> = messages
                    .iter()
                    .filter(|(r, _)| *r != "system")
                    .map(|(r, c)| json!({ "role": r, "content": c }))
                    .collect();
                let mut req = self.http
                    .post(url)
                    .header("x-api-key", self.cfg.api_key.clone().unwrap_or_default())
                    .header("anthropic-version", "2023-06-01")
                    .json(&json!({
                        "model": self.cfg.model,
                        "system": system,
                        "messages": rest,
                        "max_tokens": self.cfg.max_tokens,
                    }));
                req = req.header("User-Agent", "podcastfyui");
                let resp = req.send().await.map_err(|e| e.to_string())?;
                let status = resp.status();
                let body: Value = resp.json().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(format!("anthropic {status}: {}", body.to_string()));
                }
                body.get("content")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|b| b.get("text"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| "no text in anthropic response".into())
            }
            Provider::Gemini => {
                let base = self
                    .cfg
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".into());
                let key = self.cfg.api_key.clone().ok_or("gemini api key missing")?;
                let url = format!(
                    "{}/models/{}/generateContent?key={}",
                    base.trim_end_matches('/'),
                    self.cfg.model,
                    key
                );
                let contents: Vec<Value> = messages
                    .iter()
                    .filter(|(r, _)| *r != "system")
                    .map(|(r, c)| {
                        let role = if *r == "user" { "user" } else { "model" };
                        json!({ "role": role, "parts": [{ "text": c }] })
                    })
                    .collect();
                let system: String = messages
                    .iter()
                    .filter(|(r, _)| *r == "system")
                    .map(|(_, c)| c.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut body = json!({
                    "contents": contents,
                    "generationConfig": {
                        "temperature": self.cfg.temperature,
                        "maxOutputTokens": self.cfg.max_tokens,
                    },
                });
                if !system.is_empty() {
                    body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
                }
                let resp = self
                    .http
                    .post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = resp.status();
                let body: Value = resp.json().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(format!("gemini {status}: {}", body.to_string()));
                }
                body.pointer("/candidates/0/content/parts/0/text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| "no text in gemini response".into())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Prompts (port of upstream prompt templates)
// ---------------------------------------------------------------------------

fn system_prompt(conv: &ConversationConfig) -> String {
    // 归一化输出语言，避免模型把「中文」输出成简繁不明确的表述。
    let lang = crate::config::normalize_language(&conv.output_language);
    format!(
        "You are generating a podcast transcript for '{}'. Tagline: \"{}\".\n\
         Two hosts are speaking:\n\
         - {} (PERSON_1)\n\
         - {} (PERSON_2)\n\
         Style: {}. Dialogue structure: {}.\n\
         Output language: {}.\n\
         Only output the dialogue, no stage directions, no explanations.",
        conv.podcast_name,
        conv.podcast_tagline,
        conv.roles_person1,
        conv.roles_person2,
        conv.conversation_style.join(", "),
        conv.dialogue_structure.join(" -> "),
        lang
    )
}

fn build_prompt(
    content: &str,
    conv: &ConversationConfig,
    longform: bool,
    prev_round: Option<&str>,
) -> String {
    let mut p = String::new();
    p.push_str(&format!(
        "Write a podcast dialogue between {a} and {b} in {lang}.\n",
        a = conv.roles_person1,
        b = conv.roles_person2,
        lang = crate::config::normalize_language(&conv.output_language)
    ));
    p.push_str(&format!(
        "Style: {}. Use engagement techniques: {}. Creativity level: {}.\n",
        conv.conversation_style.join(", "),
        conv.engagement_techniques.join(", "),
        conv.creativity
    ));
    if !conv.user_instructions.trim().is_empty() {
        p.push_str(&format!("Extra instructions: {}\n", conv.user_instructions));
    }
    if longform {
        if let Some(prev) = prev_round {
            p.push_str("\n=== Previous round (context only, do not repeat it) ===\n");
            p.push_str(prev);
            p.push_str("\n=== End previous round ===\n\n");
        }
        p.push_str(
            "This is ONE round of a multi-round long-form podcast. Discuss ONLY the \
             chunk below, in a natural conversational back-and-forth (10-20 speaker \
             turns). Do not conclude the podcast. Format: PERSON_1: … / PERSON_2: …\n\n",
        );
    } else {
        p.push_str(
            "Cover ALL the content below across 25-45 natural speaker turns: start with a \
             short introduction of the podcast, cover the main content, end with a \
             brief conclusion. Format: PERSON_1: … / PERSON_2: …\n\n",
        );
    }
    p.push_str("=== SOURCE MATERIAL ===\n");
    p.push_str(tail_chars(content, 60_000).as_str());
    p
}

/// Format raw LLM output into a transcript with header.
fn format_transcript(raw: String, conv: &ConversationConfig) -> String {
    let cleaned = raw.trim().to_string();
    format!(
        "{name}\n{tagline}\n\n{body}\n\n{ending}",
        name = conv.podcast_name,
        tagline = conv.podcast_tagline,
        body = cleaned,
        ending = conv.text_to_speech.ending_message
    )
}

/// Split content into roughly equal chunks of at least `min_chars`.
fn split_chunks(content: &str, max_chunks: usize, min_chars: usize) -> Vec<String> {
    let content = content.trim();
    if content.is_empty() || max_chunks == 0 {
        return Vec::new();
    }
    let target = std::cmp::max(min_chars, content.len() / max_chunks.max(1));
    let mut chunks = Vec::new();
    let mut rest = content;
    while !rest.is_empty() && chunks.len() < max_chunks {
        if rest.len() <= target {
            chunks.push(rest.to_string());
            break;
        }
        // Find a newline near the target to avoid cutting mid-sentence.
        let window = &rest[..target];
        let cut = window
            .rfind('\n')
            .unwrap_or_else(|| target.min(rest.len()));
        chunks.push(window[..cut].to_string());
        rest = &rest[cut..];
        // Advance past the newline.
        rest = rest.trim_start_matches(['\n', '\r']);
    }
    chunks
}

fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        let start = chars.len() - n;
        String::from("[…] ") + &chars[start..].iter().collect::<String>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_max() {
        let content: String = (0..200).map(|i| format!("Sentence {i}. ")).collect();
        let c = split_chunks(&content, 3, 100);
        assert!(c.len() <= 3);
        assert!(!c.is_empty());
    }

    #[test]
    fn chunks_short_content_single() {
        let c = split_chunks("tiny", 8, 600);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0], "tiny");
    }

    #[test]
    fn provider_parse() {
        assert_eq!(Provider::parse("openai"), Some(Provider::OpenAi));
        assert_eq!(Provider::parse("OLLAMA"), Some(Provider::Ollama));
        assert_eq!(Provider::parse("nope"), None);
    }
}
