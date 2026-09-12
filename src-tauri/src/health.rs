//! Connectivity self-check for the configured LLM / TTS provider.
//!
//! Both probes go through the production code paths (`Generator::ping` →
//! `Generator::chat`, `tts::create_provider` → `synthesize_line`), so a green
//! result means the real pipeline can reach the very same endpoints.

use crate::config::{load_settings, ConversationConfig};
use crate::generator::{Generator, GeneratorConfig, Provider};
use crate::tts;
use tauri::AppHandle;

/// Probe the configured provider. `kind` is `"llm"`, `"tts"` or `"ffmpeg"`.
#[tauri::command]
pub async fn test_connection(app: AppHandle, kind: String) -> Result<String, String> {
    let settings = load_settings(&app);
    let started = std::time::Instant::now();

    match kind.as_str() {
        "llm" => {
            let provider = Provider::parse(&settings.llm.provider)
                .ok_or_else(|| format!("未知 LLM provider：{}", settings.llm.provider))?;
            let api_key = match provider {
                Provider::OpenAi => require_key(settings.keys.openai.clone(), "OpenAI")?,
                Provider::Anthropic => require_key(settings.keys.anthropic.clone(), "Anthropic")?,
                Provider::Gemini => require_key(settings.keys.gemini.clone(), "Gemini")?,
                Provider::Ollama => None,
            };
            let model = settings.llm.model.clone();
            let gen = Generator::new(GeneratorConfig {
                provider,
                model: model.clone(),
                api_key,
                base_url: if settings.llm.base_url.is_empty() {
                    None
                } else {
                    Some(settings.llm.base_url.clone())
                },
                temperature: settings.llm.temperature,
                max_tokens: settings.llm.max_tokens,
            });

            let reply = gen
                .ping()
                .await
                .map_err(|e| format!("LLM 连接失败：{e}"))?;
            Ok(format!(
                "LLM 连通正常 · {model} · {} ms · 回显「{}」",
                started.elapsed().as_millis(),
                truncate(reply.trim(), 40)
            ))
        }
        "tts" => {
            let conv = &settings.conversation;
            let model = conv.text_to_speech.default_tts_model.clone();
            let provider = tts::create_provider(&model, &settings.keys, conv)?;
            let voice = probe_voice(conv, &model);
            let out = std::env::temp_dir().join("podcastfyui-tts-probe.mp3");

            provider
                .synthesize_line("连接测试，一二三四五。", &voice, &out)
                .await
                .map_err(|e| format!("TTS 连接失败：{e}"))?;

            let bytes = tokio::fs::metadata(&out).await.map(|m| m.len()).unwrap_or(0);
            let _ = tokio::fs::remove_file(&out).await;
            Ok(format!(
                "TTS 连通正常 · {model} · {} ms · 试听样本 {bytes} 字节",
                started.elapsed().as_millis()
            ))
        }
        "ffmpeg" => {
            // Reuse the pipeline's own locator so the probe fails exactly where
            // audio assembly would; only the error text is made user-actionable.
            let path = crate::audio::find_ffmpeg().map_err(|_| {
                "未找到 ffmpeg —— Linux 请用包管理器安装（如 apt install ffmpeg / dnf install ffmpeg）；Windows 请下载后放到应用所在目录，或将其加入 PATH".to_string()
            })?;
            let path_str = path.to_string_lossy().to_string();
            let version = tokio::task::spawn_blocking(move || {
                std::process::Command::new(&path).arg("-version").output()
            })
            .await
            .map_err(|e| format!("执行 ffmpeg 失败：{e}"))?
            .map_err(|e| format!("执行 ffmpeg 失败：{e}"))?;

            if !version.status.success() {
                return Err(format!("ffmpeg 退出码异常：{:?}", version.status.code()));
            }
            let stdout = String::from_utf8_lossy(&version.stdout);
            let first = stdout.lines().next().unwrap_or("").trim();
            Ok(format!(
                "ffmpeg 可用 · {path_str} · {} ms · {}",
                started.elapsed().as_millis(),
                truncate(first, 80)
            ))
        }
        other => Err(format!("未知测试目标「{other}」（可选 llm / tts / ffmpeg）")),
    }
}

/// Mirrors the pipeline's key check so the probe fails the same way it would.
fn require_key(key: String, name: &str) -> Result<Option<String>, String> {
    if key.is_empty() {
        Err(format!("{name} API key 未填写 —— 请先保存设置"))
    } else {
        Ok(Some(key))
    }
}

/// The pipeline's voice selection, reduced to the question voice.
fn probe_voice(conv: &ConversationConfig, model: &str) -> String {
    match model.to_lowercase().as_str() {
        "edge" => conv.text_to_speech.edge.question.clone(),
        "elevenlabs" => conv.text_to_speech.elevenlabs.question.clone(),
        "gemini" => conv.text_to_speech.gemini.question.clone(),
        _ => conv.text_to_speech.openai.question.clone(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}
