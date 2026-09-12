//! The real generation pipeline:
//! extract sources → generate Q&A transcript → synthesize lines → mux with ffmpeg.
//!
//! Progress is pushed to the frontend via `task-update` events.

use super::{QueueManager, TaskInput, TaskStatus};
use crate::audio;
use crate::config::{self, load_settings};
use crate::extractor::Extractor;
use crate::generator::{Generator, GeneratorConfig, Provider};
use crate::tts;
use futures::future::join_all;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const WORKSPACE: &str = "tasks";

pub(super) async fn run(
    queue: &QueueManager,
    app: &AppHandle,
    id: &str,
    input: TaskInput,
) {
    if let Err(e) = execute(queue, app, id, input).await {
        queue.update(id, app, |t| {
            t.status = TaskStatus::Failed;
            t.error = Some(e);
            t.stage = "failed".into();
        });
    }
}

async fn execute(
    queue: &QueueManager,
    app: &AppHandle,
    id: &str,
    input: TaskInput,
) -> Result<(), String> {
    let settings = load_settings(app);
    let conv = settings.conversation;

    let task_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(WORKSPACE)
        .join(id);
    let _ = tokio::fs::create_dir_all(&task_dir).await;
    let transcript_path = task_dir.join("transcript.txt");
    let audio_path = task_dir.join("podcast.mp3");

    // ------------------------------------------------------------------
    // 1. Extraction (progress 0-15)
    // ------------------------------------------------------------------
    queue.update(id, app, |t| {
        t.status = TaskStatus::Extracting;
        t.progress = 2;
        t.stage = "extracting sources…".into();
    });

    let extractor = Extractor::new();
    let mut sources: Vec<String> = Vec::new();

    let total_units = input.urls.len()
        + input.pdfs.len()
        + usize::from(!input.text.trim().is_empty())
        + usize::from(!input.topic.trim().is_empty());
    let done_units = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // URLs (parallel, but a few at a time to be polite).
    for chunk in input.urls.chunks(4) {
        let lang: &str = &conv.output_language;
        let results = join_all(chunk.iter().map(|u| {
            let u = u.clone();
            let ex = &extractor;
            async move { ex.extract_url(&u, lang).await }
        }))
        .await;
        for (u, r) in chunk.iter().zip(results.iter()) {
            match r {
                Ok(text) => sources.push(text.clone()),
                Err(e) => {
                    tracing::warn!("url {u} failed: {e}");
                }
            }
            bump(queue, app, id, &done_units, total_units, 2, 15);
        }
    }

    // PDFs (sequential — CPU bound).
    for p in &input.pdfs {
        match extractor.extract_pdf(p).await {
            Ok(text) => sources.push(text),
            Err(e) => return Err(format!("pdf {p}: {e}")),
        }
        bump(queue, app, id, &done_units, total_units, 2, 15);
    }

    // Raw text.
    if !input.text.trim().is_empty() {
        sources.push(format!("User-provided text:\n{}", input.text.trim()));
        bump(queue, app, id, &done_units, total_units, 2, 15);
    }

    // Topic expansion.
    if !input.topic.trim().is_empty() {
        queue.update(id, app, |t| t.stage = "searching web for topic…".into());
        let topic_material = extractor
            .search_topic(
                input.topic.trim(),
                (!settings.keys.serper.is_empty()).then(|| settings.keys.serper.as_str()),
            )
            .await
            .map_err(|e| format!("topic search failed: {e}"))?;
        sources.push(format!("Topic: {}\n\n{}", input.topic, topic_material));
        bump(queue, app, id, &done_units, total_units, 2, 15);
    }

    if sources.is_empty() {
        return Err("no source content could be extracted — check URLs / files".into());
    }
    let content: String = sources.join("\n\n---\n\n");

    // ------------------------------------------------------------------
    // 2. Dialogue generation (progress 15-50)
    // ------------------------------------------------------------------
    queue.update(id, app, |t| {
        t.status = TaskStatus::Generating;
        t.progress = 15;
        t.stage = "starting dialogue generation…".into();
    });

    let provider = Provider::parse(&settings.llm.provider)
        .ok_or_else(|| format!("unknown LLM provider '{}'", settings.llm.provider))?;
    let api_key = match provider {
        Provider::OpenAi => {
            let k = settings.keys.openai.clone();
            if k.is_empty() {
                return Err("OpenAI API key missing — set it in Settings".into());
            }
            Some(k)
        }
        Provider::Anthropic => {
            let k = settings.keys.anthropic.clone();
            if k.is_empty() {
                return Err("Anthropic API key missing — set it in Settings".into());
            }
            Some(k)
        }
        Provider::Gemini => {
            let k = settings.keys.gemini.clone();
            if k.is_empty() {
                return Err("Gemini API key missing — set it in Settings".into());
            }
            Some(k)
        }
        Provider::Ollama => None,
    };

    let gen = Generator::new(GeneratorConfig {
        provider,
        model: settings.llm.model.clone(),
        api_key,
        base_url: if settings.llm.base_url.is_empty() {
            None
        } else {
            Some(settings.llm.base_url.clone())
        },
        temperature: settings.llm.temperature,
        max_tokens: settings.llm.max_tokens,
    });

    let app_cb = app.clone();
    let id_cb = id.to_string();
    let queue_cb = queue.clone();
    let progress_cb: Box<dyn Fn(&str, u8) + Send + Sync> =
        Box::new(move |stage, pct| {
            queue_cb.update(&id_cb, &app_cb, |t| {
                let mapped = 15 + (pct * 35) / 100; // 15..50
                t.progress = mapped.min(50).max(15);
                t.stage = stage.to_string();
            });
        });

    let transcript = gen
        .generate_qa(
            &content,
            &conv,
            input.longform,
            &transcript_path,
            &progress_cb,
        )
        .await?;

    queue.update(id, app, |t| {
        t.transcript_path = Some(transcript_path.to_string_lossy().to_string());
        t.progress = 50;
        t.stage = "transcript ready, starting TTS…".into();
    });

    // ------------------------------------------------------------------
    // 3. TTS line synthesis (progress 50-90)
    // ------------------------------------------------------------------
    queue.update(id, app, |t| {
        t.status = TaskStatus::Synthesizing;
        t.progress = 50;
        t.stage = "preparing voices…".into();
    });

    let tts_model = conv.text_to_speech.default_tts_model.clone();
    let provider = tts::create_provider(&tts_model, &settings.keys, &conv)?;

    let question_voice = voice_for(&conv, &tts_model, "question");
    let answer_voice = voice_for(&conv, &tts_model, "answer");

    let lines = split_transcript(&transcript);
    if lines.is_empty() {
        return Err("transcript has no speakable dialogue lines".into());
    }
    tracing::info!("synthesizing {} lines via {}", lines.len(), tts_model);

    let tmp_dir = task_dir.join("parts");
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;

    // Synthesize in small batches to keep progress responsive.
    let mut parts: Vec<PathBuf> = Vec::with_capacity(lines.len());
    let batch = 3usize;
    for (start, batch_lines) in lines.chunks(batch).enumerate() {
        let base = start * batch;
        let futures: Vec<_> = batch_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let (text, voice) = line;
                let idx = base + i;
                let out = tmp_dir.join(format!("{idx:04}.mp3"));
                let provider = &provider;
                async move {
                    let r = provider
                        .synthesize_line(text, voice, &out)
                        .await
                        .map(|_| out);
                    r
                }
            })
            .collect();
        let results = join_all(futures).await;
        for (i, r) in results.iter().enumerate() {
            match r {
                Ok(p) => parts.push(p.clone()),
                Err(e) => {
                    return Err(format!("TTS line {} failed: {e}", base + i));
                }
            }
        }
        let pct = 50 + (80 * parts.len()) / lines.len();
        queue.update(id, app, |t| {
            t.progress = pct.min(90) as u8;
            t.stage = format!("synthesizing audio {}/{}", parts.len(), lines.len());
        });
    }

    // ------------------------------------------------------------------
    // 4. Mux (progress 90-100)
    // ------------------------------------------------------------------
    queue.update(id, app, |t| {
        t.status = TaskStatus::Muxing;
        t.progress = 90;
        t.stage = "muxing final audio…".into();
    });
    audio::concatenate_parts(&parts, &audio_path).await?;

    queue.update(id, app, |t| {
        t.status = TaskStatus::Completed;
        t.progress = 100;
        t.stage = "done".into();
        t.audio_path = Some(audio_path.to_string_lossy().to_string());
    });
    Ok(())
}

/// (text, voice) for each dialogue line, in order.
/// Public so examples/tests can reuse the production line parser.
/// TTS + 音频拼接两阶段（进度 50→100）。
/// 完整管道（`run`）与「仅重新合成音频」都走这里，保证语音/拼接行为一致。
pub async fn synthesize_and_stitch(
    queue: &QueueManager,
    app: &AppHandle,
    id: &str,
    settings: &crate::config::Settings,
    transcript: &str,
    task_dir: &std::path::Path,
    audio_path: &std::path::Path,
) -> Result<(), String> {
    let conv = &settings.conversation;
    let tts_model = conv.text_to_speech.default_tts_model.clone();

    let lines = split_transcript(transcript);
    if lines.is_empty() {
        return Err(
            "转录稿里没有可合成的对白行（每行需以 PERSON_1: / PERSON_2: 开头）".into(),
        );
    }

    let provider = tts::create_provider(&tts_model, &settings.keys, conv)?;

    queue.update(id, app, |t| {
        t.status = TaskStatus::Synthesizing;
        t.stage = format!("语音合成：{} 行 / {}", lines.len(), tts_model);
        t.progress = 52;
        t.error = None;
    });

    let tmp_dir = task_dir.join("parts");
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;

    let total = lines.len();
    let mut parts: Vec<std::path::PathBuf> = Vec::with_capacity(total);
    for (i, (text, voice)) in lines.iter().enumerate() {
        let out = tmp_dir.join(format!("{i:04}.mp3"));
        provider
            .synthesize_line(text, voice, &out)
            .await
            .map_err(|e| format!("第 {} 行语音合成失败：{e}", i + 1))?;
        parts.push(out);
        let pct = 52 + ((38 * (i + 1)) / total) as u8;
        queue.update(id, app, |t| t.progress = pct);
    }

    queue.update(id, app, |t| {
        t.status = TaskStatus::Muxing;
        t.stage = "拼接音频".into();
        t.progress = 94;
    });
    audio::concatenate_parts(&parts, audio_path).await?;

    queue.update(id, app, |t| {
        t.status = TaskStatus::Completed;
        t.stage = "完成".into();
        t.progress = 100;
        t.audio_path = Some(audio_path.to_string_lossy().to_string());
    });
    Ok(())
}

pub fn split_transcript(transcript: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for line in transcript.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((speaker, text)) = line.split_once(':') {
            let speaker = speaker.trim();
            if speaker == "PERSON_1" || speaker == "PERSON_2" {
                let text = text.trim();
                if !text.is_empty() {
                    out.push((text, speaker));
                }
            }
        } else if line.starts_with(&['P', 'A', 'Q'])
            && (line.contains("PERSON_1:") || line.contains("PERSON_2:"))
        {
            // Fallback: allow lower/other-case variants like "Person 1:".
            if let Some((speaker, text)) = line
                .split_once(":")
                .map(|(s, t)| (s.trim().to_lowercase(), t.trim()))
            {
                if speaker == "person_1" {
                    out.push((text, "PERSON_1"));
                } else if speaker == "person_2" {
                    out.push((text, "PERSON_2"));
                }
            }
        }
    }
    out
}

fn voice_for(conv: &config::ConversationConfig, model: &str, role: &str) -> String {
    let v = match model.to_lowercase().as_str() {
        "openai" => &conv.text_to_speech.openai,
        "elevenlabs" => &conv.text_to_speech.elevenlabs,
        "edge" => &conv.text_to_speech.edge,
        "gemini" => &conv.text_to_speech.gemini,
        _ => &conv.text_to_speech.openai,
    };
    let voice = if role == "answer" {
        v.answer.clone()
    } else {
        v.question.clone()
    };
    // 简体中文 + edge 模型 + 解析出的音色非中文时，回退到默认中文音色。
    let is_edge = model.eq_ignore_ascii_case("edge");
    let is_zh =
        config::normalize_language(&conv.output_language) == "Simplified Chinese (简体中文)";
    if is_edge && is_zh && !voice.starts_with("zh-") {
        return match role {
            "answer" => "zh-CN-YunxiNeural".to_string(),
            _ => "zh-CN-XiaoxiaoNeural".to_string(),
        };
    }
    voice
}

fn bump(
    queue: &QueueManager,
    app: &AppHandle,
    id: &str,
    done: &Arc<std::sync::atomic::AtomicUsize>,
    total: usize,
    lo: u8,
    hi: u8,
) {
    let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if total == 0 {
        return;
    }
    let pct = lo + ((hi - lo) * d as u8) / total as u8;
    queue.update(id, app, |t| t.progress = pct);
}

/// Keep clippy quiet about the unused Duration import pattern.
#[allow(dead_code)]
fn _unused() -> std::time::Duration {
    std::time::Duration::from_millis(1)
}
