//! Text-to-speech providers.
//!
//! Factory + trait design mirrors upstream `tts/base.py` + `tts/factory.py`.
//! Providers ship in this build: OpenAI (tts-1-hd) and Edge TTS (free, no key).
//! ElevenLabs / Gemini multi-speaker land later.

use crate::config::{ApiKeys, ConversationConfig};
use async_trait::async_trait;
use base64::Engine as _;
use std::path::Path;
use std::time::Duration;

/// One TTS backend.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Synthesize one line of dialogue into `out_path` (mp3).
    async fn synthesize_line(&self, text: &str, voice: &str, out_path: &Path) -> Result<(), String>;
}

/// Build the configured provider.
pub fn create_provider(
    model: &str,
    keys: &ApiKeys,
    conv: &ConversationConfig,
) -> Result<Box<dyn TtsProvider>, String> {
    match model.to_lowercase().as_str() {
        "openai" => {
            let key = keys.openai.clone();
            if key.is_empty() {
                return Err("OpenAI TTS needs an API key (Settings → API keys)".into());
            }
            Ok(Box::new(OpenAiTts::new(
                key,
                conv.text_to_speech.openai.model.clone().unwrap_or_else(|| "tts-1-hd".into()),
            )))
        }
        "edge" => Ok(Box::new(EdgeTts::new(
            &conv.text_to_speech.edge.question,
            &conv.text_to_speech.edge.answer,
        ))),
        other => Err(format!(
            "TTS provider '{other}' not available in this build (openai/edge supported)"
        )),
    }
}

// ---------------------------------------------------------------------------
// OpenAI TTS
// ---------------------------------------------------------------------------

pub struct OpenAiTts {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl OpenAiTts {
    pub fn new(api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("http client");
        Self { http, api_key, model }
    }
}

#[async_trait]
impl TtsProvider for OpenAiTts {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn synthesize_line(&self, text: &str, voice: &str, out_path: &Path) -> Result<(), String> {
        let resp = self
            .http
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "voice": voice,
                "input": text,
                "response_format": "mp3",
            }))
            .send()
            .await
            .map_err(|e| format!("openai tts request: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("openai tts {status}: {body}"));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        tokio::fs::write(out_path, &bytes)
            .await
            .map_err(|e| format!("write {}: {e}", out_path.display()))
    }
}

// ---------------------------------------------------------------------------
// Edge TTS (free, no key)
// ---------------------------------------------------------------------------

/// Minimal port of the edge-tts protocol:
/// 1. POST the text to `speech.platform.bing.com/consumer/speech/synthesize/readaloud/...`
///    with the Sec-MS-GEC token dance (the current public protocol is a
///    WebSocket upgrade; we speak plain HTTP JSON which the service accepts).
pub struct EdgeTts {
    http: reqwest::Client,
    default_voice: String,
    answer_voice: String,
}

impl EdgeTts {
    pub fn new(question_voice: &str, answer_voice: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("http client");
        Self {
            http,
            default_voice: question_voice.to_string(),
            answer_voice: answer_voice.to_string(),
        }
    }

    pub fn answer_voice(&self) -> &str {
        &self.answer_voice
    }
}

#[async_trait]
impl TtsProvider for EdgeTts {
    fn name(&self) -> &'static str {
        "edge"
    }

    async fn synthesize_line(&self, text: &str, voice: &str, out_path: &Path) -> Result<(), String> {
        let voice = if voice.is_empty() {
            &self.default_voice
        } else {
            voice
        };
        let url = format!(
            "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/merge/all/?trustedclienttoken=6A5AA1D4EAFF4E9FB37E23D68491D6F4&config={}",
            edge_config(voice)
        );
        let body = edge_ssml(voice, text);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/ssml+xml")
            .header("User-Agent", "Mozilla/5.0")
            .header("X-Microsoft-OutputFormat", "audio-24khz-96kbitrate-mono-mp3")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("edge tts request: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "edge tts {status} — the protocol may have changed; try OpenAI TTS. ({body})"
            ));
        }
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
        if bytes.is_empty() {
            return Err("edge tts returned empty audio".into());
        }
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        tokio::fs::write(out_path, &bytes)
            .await
            .map_err(|e| format!("write {}: {e}", out_path.display()))
    }
}

fn edge_config(voice: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(serde_json::json!({
        "context": { "synthesis": { "audio": {
            "metadata": { "contentCategory": "social", "experienceCategory": "social.casual" }
        }}},
        "data": "",
        "request": {
            "action": "exec",
            "configs": [
                {
                    "configuration": {
                        "requestId": uuid::Uuid::new_v4().to_string(),
                        "context": {
                            "synthesis": {
                                "audio": {
                                    "metadata": {
                                        "contentCategory": "social",
                                        "experienceCategory": "social.casual"
                                    },
                                    "metadataOptions": [
                                        {"name": "EngineManifestId"},
                                        {"name": "VoiceInfo"}
                                    ]
                                }
                            }
                        },
                        "synthesisRequest": {
                            "payload": {
                                "text": "",
                                "boundary": "sentenceBoundary"
                            },
                            "context": {
                                "synthesis": {
                                    "telephonyRequest": {"value": "1234"},
                                    "textToSpeech": {
                                        "trustedClientToken": "6A5AA1D4EAFF4E9FB37E23D68491D6F4",
                                        "parameters": {
                                            "engine": {
                                                "type": "default",
                                                "telemetry": { "tracing": {
                                                    "async": {"mode": "disabled"}
                                                }},
                                                "voice": {
                                                    "name": voice,
                                                    "personalization": {
                                                        "mode": "default"
                                                    }
                                                },
                                                "audio": {
                                                    "outputFormat": "audio-24khz-96kbitrate-mono-mp3"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            ],
            "options": {
                "storage": {"blobStorageEndpoint": ""},
                "requestResponse": {
                    "async": {"mode": "disabled"},
                    "output": {"format": "raw"}
                }
            }
        }
    }).to_string())
}

fn edge_ssml(voice: &str, text: &str) -> String {
    let safe = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    // 由音色名前缀推导 xml:lang，避免中文/日文等非英文音色被当成 en-US。
    let xml_lang = if voice.starts_with("zh-CN-") {
        "zh-CN"
    } else if voice.starts_with("zh-TW-") {
        "zh-TW"
    } else if voice.starts_with("ja-JP-") {
        "ja-JP"
    } else {
        "en-US"
    };
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' \
         xmlns:mstts='http://www.w3.org/2001/mstts' xml:lang='{xml_lang}'>\
         <voice name='{voice}'>{safe}</voice></speak>"
    )
}
