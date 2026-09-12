//! End-to-end smoke test (no Tauri window, no real API keys).
//!
//! Exercises the FULL production pipeline:
//!   1. Real web extraction (https://example.com) via `Extractor`
//!   2. Real dialogue generation via `Generator` against a **local mock
//!      OpenAI-compatible server** (spawned in-process)
//!   3. TTS via a mock `TtsProvider` (real ffmpeg produces each mp3 line)
//!   4. Real ffmpeg concat (`audio::concatenate_parts`)
//!
//! Run: cargo run --example smoke_test
//!
//! NOTE: the in-app `pipeline::run` orchestrates these same calls but needs
//! a Tauri AppHandle (events + app data dir), so this example reproduces the
//! stage sequence 1:1 with real components and prints each stage result.

use async_trait::async_trait;
use futures::future::join_all;
use podcastfyui_lib::audio;
use podcastfyui_lib::config::ConversationConfig;
use podcastfyui_lib::extractor::Extractor;
use podcastfyui_lib::generator::{Generator, GeneratorConfig, Provider};
use podcastfyui_lib::queue::split_transcript;
use podcastfyui_lib::tts::TtsProvider;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A TTS provider that produces REAL mp3s with ffmpeg (sine wave), standing
/// in for OpenAI/Edge while the cloud endpoints are unreachable from this
/// network. It implements the exact production trait, so the pipeline code
/// path (line splitting → per-line files → batch → concat) is identical.
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

/// Minimal OpenAI-compatible /v1/chat/completions mock server.
/// Always answers with a short two-host dialogue that references the
/// user prompt (so extraction failures would be visible in the transcript).
struct MockLlm {
    port: u16,
    shutdown: Arc<tokio::sync::Notify>,
}

impl MockLlm {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let sh = Arc::clone(&shutdown);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sh.notified() => break,
                    accepted = listener.accept() => {
                        let (mut stream, _) = match accepted {
                            Ok(x) => x,
                            Err(_) => continue,
                        };
                        tokio::spawn(async move {
                            let _ = handle_one(&mut stream).await;
                        });
                    }
                }
            }
        });
        Self { port, shutdown }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn stop(self) {
        self.shutdown.notify_one();
    }
}

async fn handle_one(stream: &mut tokio::net::TcpStream) {
    let mut buf = [0u8; 8192];
    let mut raw = Vec::new();
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.windows(4).any(|w| w == b"\r\n\r\n")
                    || raw.len() > 1_000_000
                {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&raw).to_string();

    let user_text: String = {
        let key = "\"content\"";
        head.split(key)
            .nth(1)
            .map(|rest| {
                let s = rest.find('"').map(|i| i + 1).unwrap_or(0);
                let e = rest[s..].find('"').map(|i| s + i).unwrap_or(rest.len());
                rest[s..e].replace("\\n", "\n")
            })
            .unwrap_or_default()
    };
    // Keep the mock transcript short; echo the first 40 chars of the prompt.
    let snippet: String = user_text.chars().take(40).collect();
    let transcript = format!(
        "PERSON_1: Welcome to the smoke test podcast. The source material started with: {snippet}\n\
         PERSON_2: Nice. This line comes from the mock model, so we can verify the full chain.\n\
         PERSON_1: Extraction, generation, synthesis and muxing are all production code paths here.\n\
         PERSON_2: Exactly. See you next time!"
    );
    let json = format!(
        "{{\"id\":\"smoke\",\"object\":\"chat.completion\",\"choices\":[{{\"index\":0, \
         \"message\":{{\"role\":\"assistant\",\"content\":{}}},\"finish_reason\":\"stop\"}}]}}",
        serde_json::to_string(&transcript).unwrap()
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        json.len(),
        json
    );
    let _ = stream.write_all(resp.as_bytes()).await;
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), String> {
    let work = std::env::temp_dir().join(format!("podcastfy_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let conv = ConversationConfig::default();
    let mut passed = 0usize;

    // ---- Stage 0: mock LLM server -------------------------------------
    let llm = MockLlm::start().await;
    println!("[0] mock LLM at {}", llm.base_url());

    // ---- Stage 1: real web extraction ----------------------------------
    let extractor = Extractor::new();
    let source = extractor
        .extract_url("https://example.com", "English")
        .await
        .map_err(|e| format!("extraction failed: {e}"))?;
    assert!(source.contains("Example Domain"), "unexpected extraction output");
    // example.com is intentionally tiny; only assert we got real content.
    assert!(source.chars().count() > 50, "extraction too short");
    println!(
        "[1] extraction OK ({} chars): \"{}...\"",
        source.chars().count(),
        &source[..source.find('\n').map(|i| i.min(60)).unwrap_or(60)]
    );
    passed += 1;

    // ---- Stage 2: real generation against mock LLM ---------------------
    let gen = Generator::new(GeneratorConfig {
        provider: Provider::OpenAi,
        model: "mock-model".into(),
        api_key: Some("mock".into()),
        base_url: Some(llm.base_url()),
        temperature: 0.8,
        max_tokens: 2048,
    });
    let transcript_path = work.join("transcript.txt");
    let gen_progress = |_stage: &str, _pct: u8| {};
    let transcript = gen
        .generate_qa(&source, &conv, false, &transcript_path, &gen_progress)
        .await
        .map_err(|e| format!("generation failed: {e}"))?;
    let on_disk = std::fs::read_to_string(&transcript_path).unwrap();
    assert_eq!(transcript, on_disk, "transcript file mismatch");
    assert!(transcript.contains("PERSON_1:") && transcript.contains("PERSON_2:"));
    println!(
        "[2] generation OK ({} chars, {} lines)",
        transcript.chars().count(),
        transcript.lines().count()
    );
    passed += 1;

    // ---- Stage 3: TTS line synthesis (mock provider, real ffmpeg) ------
    let ffmpeg = audio::find_ffmpeg().map_err(|e| format!("ffmpeg: {e}"))?;
    println!("[3] ffmpeg: {}", ffmpeg.display());
    let lines = split_transcript(&transcript);
    assert!(lines.len() >= 2, "split_transcript produced no lines");
    let tts = FfmpegMockTts {
        ffmpeg: ffmpeg.clone(),
    };
    let parts_dir = work.join("parts");
    std::fs::create_dir_all(&parts_dir).unwrap();
    let mut parts: Vec<PathBuf> = Vec::new();
    for chunk in lines.chunks(3) {
        let base = parts.len();
        let futures: Vec<_> = chunk
            .iter()
            .enumerate()
            .map(|(i, (text, voice))| {
                let idx = base + i;
                let out = parts_dir.join(format!("{idx:04}.mp3"));
                let tts = &tts;
                async move {
                    tts.synthesize_line(text, voice, &out)
                        .await
                        .map(|_| out)
                }
            })
            .collect();
        let results = join_all(futures).await;
        for (i, r) in results.into_iter().enumerate() {
            let p = r.map_err(|e| format!("tts line {} failed: {e}", base + i))?;
            parts.push(p);
        }
    }
    assert_eq!(parts.len(), lines.len(), "missing audio parts");
    for p in &parts {
        assert!(p.metadata().unwrap().len() > 100, "part too small: {}", p.display());
    }
    println!("[3] tts OK ({} lines -> {} mp3 parts)", lines.len(), parts.len());
    passed += 1;

    // ---- Stage 4: real ffmpeg concat ------------------------------------
    let final_path = work.join("podcast.mp3");
    audio::concatenate_parts(&parts, &final_path)
        .await
        .map_err(|e| format!("concat failed: {e}"))?;
    let size = std::fs::metadata(&final_path).map_err(|e| e.to_string())?.len();
    assert!(size > 2000, "final audio suspiciously small ({size} bytes)");
    println!("[4] concat OK -> {} ({} bytes)", final_path.display(), size);
    passed += 1;

    llm.stop();

    println!(
        "\nSMOKE PASS — {passed}/4 stages green\nartifacts: {}",
        work.display()
    );
    Ok(())
}
