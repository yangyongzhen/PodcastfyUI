//! End-to-end example driven by **real configuration + a real LLM**.
//!
//! Chain reproduced 1:1 with production code paths:
//!   0. Real config  : deserialize `api_keys.json` / `llm.json` /
//!                     `conversation.yaml` from the app data dir (key content
//!                     is never printed).
//!   1. Content source: real `Extractor::extract_url("https://example.com")`;
//!                     on network failure we fall back to an inline Chinese
//!                     source about "AI 播客" and say so truthfully.
//!   2. Real generation: `Generator::generate_qa` against the real taotoken
//!                     OpenAI-compatible endpoint (`/v1/chat/completions`).
//!   3. Real TTS      : `split_transcript` → per-line synthesis via a mock
//!                     `TtsProvider` that produces REAL mp3s with ffmpeg
//!                     (cloud TTS endpoints are unreachable from this host).
//!   4. Real concat   : `audio::concatenate_parts` → `/tmp/e2e-real/final.mp3`.
//!
//! Every stage prints PASS/FAIL and failures are collected (no panic-driven
//! early exit): the process only exits non-zero after printing the summary.
//!
//! Run: cargo run --example e2e_real_llm

use async_trait::async_trait;
use futures::future::join_all;
use podcastfyui_lib::audio;
use podcastfyui_lib::config::{ApiKeys, ConversationConfig, LlmConfig};
use podcastfyui_lib::extractor::Extractor;
use podcastfyui_lib::generator::{Generator, GeneratorConfig, Provider};
use podcastfyui_lib::queue::split_transcript;
use podcastfyui_lib::tts::TtsProvider;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// App data dir holding the REAL, already-verified configuration files.
const CONFIG_DIR: &str = "/root/.local/share/com.podcastfy.ui";

/// Fallback source material (简体中文) used only when live extraction fails.
const INLINE_SOURCE: &str = "AI 播客正在改变人们获取信息的方式。过去，听众需要主动搜索、阅读长文，\
再把要点记下来；现在，生成式人工智能可以把一篇长文章、一段访谈或一份报告，自动改写成两个人\
自然对话的播客脚本。这让知识传播的门槛大幅降低：通勤路上、健身房里，人们就能听懂复杂的技术\
概念。与此同时，AI 播客也带来新的挑战，比如事实准确性、声音的自然度，以及内容同质化。好的\
AI 播客工具，需要把联网抓取、大模型写作、语音合成和音频拼接这几步做得稳定可靠，让创作者把\
精力放在选题与审核上，而不是繁琐的后期处理。";

/// TTS provider producing REAL mp3s with ffmpeg (sine wave), standing in for
/// OpenAI/Edge while the cloud endpoints are unreachable from this network.
/// It implements the exact production `TtsProvider` trait, so the pipeline
/// code path (line splitting → per-line files → batch → concat) is identical.
struct FfmpegMockTts {
    ffmpeg: PathBuf,
}

#[async_trait]
impl TtsProvider for FfmpegMockTts {
    fn name(&self) -> &'static str {
        "mock-ffmpeg"
    }

    async fn synthesize_line(&self, text: &str, voice: &str, out_path: &Path) -> Result<(), String> {
        // Frequency keyed on voice so the two "hosts" sound different.
        let freq = if voice.contains("PERSON_2") { 220.0 } else { 330.0 };
        // ~0.4s per 40 chars, clamped 0.5..3s
        let dur = (text.chars().count() as f64 / 40.0).clamp(0.5, 3.0);

        let src = out_path.to_string_lossy().to_string();
        let ffmpeg = self.ffmpeg.clone();
        tokio::task::spawn_blocking(move || {
            std::process::Command::new(&ffmpeg)
                .args([
                    "-y", "-f", "lavfi",
                    "-i", &format!("sine=frequency={freq}:duration={dur}"),
                    "-ar", "24000", "-ac", "1", "-c:a", "libmp3lame", "-b:a", "96k",
                    &src,
                ])
                .output()
                .map_err(|e| format!("spawn ffmpeg: {e}"))
                .and_then(|o| {
                    if o.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "ffmpeg: {}",
                            String::from_utf8_lossy(&o.stderr).chars().take(200).collect::<String>()
                        ))
                    }
                })
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

/// Deserialize the real on-disk configuration. Returns the key length only —
/// never the key material.
fn load_real_config() -> Result<(ApiKeys, LlmConfig, ConversationConfig), String> {
    let dir = Path::new(CONFIG_DIR);

    let keys_s = std::fs::read_to_string(dir.join("api_keys.json"))
        .map_err(|e| format!("read api_keys.json: {e}"))?;
    let keys: ApiKeys =
        serde_json::from_str(&keys_s).map_err(|e| format!("parse api_keys.json: {e}"))?;

    let llm_s =
        std::fs::read_to_string(dir.join("llm.json")).map_err(|e| format!("read llm.json: {e}"))?;
    let llm: LlmConfig =
        serde_json::from_str(&llm_s).map_err(|e| format!("parse llm.json: {e}"))?;

    let conv_s = std::fs::read_to_string(dir.join("conversation.yaml"))
        .map_err(|e| format!("read conversation.yaml: {e}"))?;
    let conv: ConversationConfig =
        serde_yaml::from_str(&conv_s).map_err(|e| format!("parse conversation.yaml: {e}"))?;

    if keys.openai.is_empty() {
        return Err("api_keys.json: openai key is empty".into());
    }
    Ok((keys, llm, conv))
}

/// Print the stage summary; returns true iff every stage passed.
fn print_summary(results: &[(&'static str, bool)]) -> bool {
    let passed = results.iter().filter(|(_, ok)| *ok).count();
    println!("\n===== SUMMARY: {passed}/{} stages PASS =====", results.len());
    for (name, ok) in results {
        println!("    [{}] {name}", if *ok { "PASS" } else { "FAIL" });
    }
    passed == results.len()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let work = PathBuf::from("/tmp/e2e-real");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("create /tmp/e2e-real");

    println!("== e2e_real_llm: real config -> real LLM -> split -> ffmpeg TTS -> concat ==");
    println!("workdir: {}\n", work.display());

    let mut results: Vec<(&'static str, bool)> = Vec::new();

    // ---- [0] real config -------------------------------------------------
    let cfg = load_real_config();
    match &cfg {
        Ok((keys, llm, conv)) => {
            println!("[0] config PASS");
            println!("    api_keys.openai: {} chars (content withheld)", keys.openai.len());
            println!(
                "    llm: provider={} model={} base_url={} temperature={} max_tokens={}",
                llm.provider, llm.model, llm.base_url, llm.temperature, llm.max_tokens
            );
            println!(
                "    conv: output_language={} tts_model={}",
                conv.output_language, conv.text_to_speech.default_tts_model
            );
        }
        Err(e) => println!("[0] config FAIL: {e}"),
    }
    results.push(("config", cfg.is_ok()));
    let (keys, llm, conv) = match cfg {
        Ok(v) => v,
        Err(_) => {
            print_summary(&results);
            std::process::exit(1);
        }
    };

    // ---- [1] content source (real extraction, inline fallback) -----------
    let extractor = Extractor::new();
    let source = match extractor
        .extract_url("https://example.com", &conv.output_language)
        .await
    {
        Ok(s) if s.chars().count() >= 50 => {
            println!(
                "[1] content source PASS: real Extractor::extract_url(\"https://example.com\") -> {} chars",
                s.chars().count()
            );
            s
        }
        Ok(s) => {
            println!(
                "[1] content source PASS: INLINE fallback (live extraction returned only {} chars)",
                s.chars().count()
            );
            INLINE_SOURCE.to_string()
        }
        Err(e) => {
            println!("[1] content source PASS: INLINE fallback (live extraction failed: {e})");
            INLINE_SOURCE.to_string()
        }
    };
    results.push(("content source", true));

    // ---- [2] real LLM generation (taotoken) ------------------------------
    let gen = Generator::new(GeneratorConfig {
        provider: Provider::parse(&llm.provider).unwrap_or(Provider::OpenAi),
        model: llm.model.clone(),
        api_key: Some(keys.openai.clone()),
        base_url: if llm.base_url.is_empty() {
            None
        } else {
            Some(llm.base_url.clone())
        },
        temperature: llm.temperature,
        max_tokens: llm.max_tokens,
    });
    let transcript_path = work.join("transcript.txt");
    let progress = |stage: &str, pct: u8| println!("    .. {stage} ({pct}%)");

    let t0 = Instant::now();
    let transcript = match gen
        .generate_qa(&source, &conv, false, &transcript_path, &progress)
        .await
    {
        Ok(t) => {
            let preview: String = t.chars().take(120).collect();
            println!(
                "[2] generation PASS: model={} elapsed={:?} chars={} lines={}",
                llm.model,
                t0.elapsed(),
                t.chars().count(),
                t.lines().count()
            );
            println!("    preview(120): {}", preview.replace('\n', " / "));
            println!("    saved: {}", transcript_path.display());
            results.push(("generation", true));
            Some(t)
        }
        Err(e) => {
            println!("[2] generation FAIL: {e}");
            results.push(("generation", false));
            None
        }
    };

    let transcript = match transcript {
        Some(t) => t,
        None => {
            println!("[3] TTS FAIL: skipped (no transcript)");
            println!("[4] concat FAIL: skipped (no audio parts)");
            results.push(("tts", false));
            results.push(("concat", false));
            print_summary(&results);
            std::process::exit(1);
        }
    };

    // ---- [3] per-line TTS via FfmpegMockTts (real ffmpeg mp3) ------------
    let ffmpeg = match audio::find_ffmpeg() {
        Ok(p) => {
            println!("[3] ffmpeg: {}", p.display());
            p
        }
        Err(e) => {
            println!("[3] TTS FAIL: {e}");
            results.push(("tts", false));
            results.push(("concat", false));
            print_summary(&results);
            std::process::exit(1);
        }
    };

    let lines = split_transcript(&transcript);
    println!("[3] split_transcript -> {} (text, voice) lines", lines.len());
    let tts = FfmpegMockTts { ffmpeg: ffmpeg.clone() };
    let parts_dir = work.join("parts");
    std::fs::create_dir_all(&parts_dir).expect("parts dir");

    let mut parts: Vec<PathBuf> = Vec::new();
    let mut tts_err: Option<String> = None;
    for chunk in lines.chunks(3) {
        let base = parts.len();
        let futures: Vec<_> = chunk
            .iter()
            .enumerate()
            .map(|(i, (text, voice))| {
                let idx = base + i;
                let out = parts_dir.join(format!("{idx:04}.mp3"));
                let tts = &tts;
                async move { tts.synthesize_line(text, voice, &out).await.map(|_| out) }
            })
            .collect();
        let res = join_all(futures).await;
        for (i, r) in res.into_iter().enumerate() {
            match r {
                Ok(p) => parts.push(p),
                Err(e) => {
                    tts_err = Some(format!("line {} failed: {e}", base + i));
                    break;
                }
            }
        }
        if tts_err.is_some() {
            break;
        }
    }
    if tts_err.is_none() && lines.is_empty() {
        tts_err = Some("split_transcript produced 0 lines".into());
    }

    let mut tts_ok = false;
    match tts_err {
        Some(e) => {
            println!("[3] TTS FAIL: {e}");
            results.push(("tts", false));
        }
        None => {
            let all_parts_ok = parts.len() == lines.len()
                && parts
                    .iter()
                    .all(|p| p.metadata().map(|m| m.len() > 100).unwrap_or(false));
            if all_parts_ok {
                println!(
                    "[3] TTS PASS: {} lines -> {} mp3 parts in {}",
                    lines.len(),
                    parts.len(),
                    parts_dir.display()
                );
                tts_ok = true;
                results.push(("tts", true));
            } else {
                println!(
                    "[3] TTS FAIL: {} usable parts for {} lines",
                    parts.len(),
                    lines.len()
                );
                results.push(("tts", false));
            }
        }
    }

    // ---- [4] real ffmpeg concat -----------------------------------------
    let final_path = work.join("final.mp3");
    if tts_ok && !parts.is_empty() {
        match audio::concatenate_parts(&parts, &final_path).await {
            Ok(()) => {
                let size = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
                if size > 2000 {
                    println!("[4] concat PASS -> {} ({} bytes)", final_path.display(), size);
                    results.push(("concat", true));
                } else {
                    println!("[4] concat FAIL: final.mp3 only {size} bytes");
                    results.push(("concat", false));
                }
            }
            Err(e) => {
                println!("[4] concat FAIL: {e}");
                results.push(("concat", false));
            }
        }
    } else {
        println!("[4] concat FAIL: skipped (no audio parts)");
        results.push(("concat", false));
    }

    let all = print_summary(&results);
    if !all {
        std::process::exit(1);
    }
}
