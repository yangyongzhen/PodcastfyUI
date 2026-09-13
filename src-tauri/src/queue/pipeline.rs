//! The real generation pipeline:
//! extract sources → generate Q&A transcript → synthesize lines → mux with ffmpeg.
//!
//! Progress is pushed to the frontend via `task-update` events.

use super::{QueueManager, TaskInput, TaskStatus};
use crate::audio;
use crate::config::{self, load_settings};
use crate::extractor::{Extractor, SearchCfg};
use crate::generator::{Generator, GeneratorConfig, Provider};
use crate::tts;
use futures::future::join_all;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const WORKSPACE: &str = "tasks";

/// 组装主题搜索配置：后端与条数来自 `search.json`，凭证来自 `api_keys.json`。
fn search_cfg(settings: &config::Settings) -> SearchCfg {
    SearchCfg {
        provider: settings.search.provider.clone(),
        num_results: settings.search.num_results,
        exa_key: settings.keys.exa.clone(),
        serper_key: settings.keys.serper.clone(),
        bocha_key: settings.keys.bocha.clone(),
        zhipu_key: settings.keys.zhipu.clone(),
        qianfan_key: settings.keys.qianfan.clone(),
    }
}

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
    let conv = settings.conversation.clone();

    let task_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(WORKSPACE)
        .join(id);
    let _ = tokio::fs::create_dir_all(&task_dir).await;
    tracing::info!("task {id}: workspace ready");
    let transcript_path = task_dir.join("transcript.txt");
    let audio_path = task_dir.join("podcast.mp3");

    // ------------------------------------------------------------------
    // 1. Extraction (progress 0-15)
    // ------------------------------------------------------------------
    tracing::info!("task {id}: stage -> extracting");
    queue.update(id, app, |t| {
        t.status = TaskStatus::Extracting;
        t.progress = 2;
        t.stage = "extracting sources…".into();
    });
    tracing::info!("task {id}: stage update returned");

    let extractor = Extractor::new();
    tracing::info!("task {id}: extractor ready");
    let mut sources: Vec<String> = Vec::new();

    let total_units = input.urls.len()
        + input.pdfs.len()
        + usize::from(!input.text.trim().is_empty())
        + usize::from(!input.topic.trim().is_empty());
    tracing::info!("task {id}: {total_units} source unit(s) queued");
    let done_units = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // URLs (parallel, but a few at a time to be polite).
    for chunk in input.urls.chunks(4) {
        tracing::info!("task {id}: fetching {} url(s)", chunk.len());
        let lang: &str = &conv.output_language;
        let results = join_all(chunk.iter().map(|u| {
            let u = u.clone();
            let ex = &extractor;
            async move { ex.extract_url(&u, lang).await }
        }))
        .await;
        tracing::info!("task {id}: url batch returned");
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
        let search = search_cfg(&settings);
        let provider = if search.provider.trim().is_empty() {
            "auto"
        } else {
            search.provider.trim()
        };
        tracing::info!(
            "task {id}: topic search start (provider={provider}, num={})",
            search.num_results
        );
        queue.update(id, app, |t| t.stage = "searching web for topic…".into());
        match extractor.search_topic(input.topic.trim(), &search).await {
            Ok(material) => {
                tracing::info!("task {id}: topic search ok ({} chars)", material.len());
                sources.push(format!("Topic: {}\n\n{material}", input.topic));
            }
            // 所有搜索后端都没答上：按设置决定是「降级继续」还是让任务失败。
            Err(e) => {
                tracing::warn!("task {id}: topic search failed: {e}");
                if !settings.search.degrade_without_search {
                    return Err(format!("topic search failed: {e}"));
                }
                queue.update(id, app, |t| {
                    t.stage = "search unavailable — using model knowledge…".into()
                });
                tracing::warn!(
                    "task {id}: degrading to model knowledge (no live web sources this run)"
                );
                sources.push(format!(
                    "Topic: {}\n\n\
                     NOTE: live web search was unavailable for this episode ({e}).\n\
                     Write it from your own general knowledge instead, and say plainly in the\n\
                     opening that the facts were NOT checked against live web sources — avoid\n\
                     specific fresh figures, quotes, or dates that you cannot back up.\n",
                    input.topic
                ));
            }
        }
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
                // pct 是 u8：`pct * 35` 在 pct >= 8 时就会溢出（debug 构建直接 panic，
                // 会让 task 线程崩溃并毒化队列锁）。先升到 u32 再算。
                let mapped = 15u32 + (u32::from(pct) * 35) / 100; // 15..50
                t.progress = mapped.min(50).max(15) as u8;
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
                let (text, role) = *line;
                let voice = voice_for_role(&question_voice, &answer_voice, role);
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

/// 可选第五阶段：把已完成的 mp3 导出成视频（L1 静态封面 / L2 波形）。
///
/// 与生成管道解耦，**失败不回退音频产物**：错误只记进 `task.video_error`，任务状态回到
/// `Completed`，前端可单独提示。取消沿用既有语义——进入编码前检查一次；ffmpeg 编码过程中
/// 不中断（与「取消不中断已发出的请求」口径一致），所以收尾时再确认一次状态，避免把
/// 已取消的任务又改回完成。
pub async fn export_video(queue: &QueueManager, app: &AppHandle, id: &str) -> Result<(), String> {
    let task = queue.get(id).ok_or_else(|| format!("任务不存在：{id}"))?;
    if task.status == TaskStatus::Cancelled {
        return Err("任务已取消".into());
    }
    let audio_path = task
        .audio_path
        .clone()
        .ok_or_else(|| "任务还没有音频产物，先完成生成再导出视频".to_string())?;
    let audio = PathBuf::from(&audio_path);
    if !audio.is_file() {
        return Err(format!("音频文件不存在：{audio_path}"));
    }

    let settings = load_settings(app);
    let cfg = settings.video.clone();
    let task_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(WORKSPACE)
        .join(id);
    let _ = tokio::fs::create_dir_all(&task_dir).await;
    let out = crate::video::output_path(&task_dir, &cfg);

    // 标题留空回落任务标题；副标题留空回落播客 tagline。两者都空也能出片（纯视觉）。
    let title = if cfg.title.trim().is_empty() {
        task.title.clone()
    } else {
        cfg.title.clone()
    };
    let subtitle = if cfg.subtitle.trim().is_empty() {
        settings.conversation.podcast_tagline.clone()
    } else {
        cfg.subtitle.clone()
    };

    let aspect_label = if cfg.aspect == "portrait" {
        "竖版 9:16"
    } else {
        "横版 16:9"
    };
    queue.update(id, app, |t| {
        t.status = TaskStatus::Exporting;
        t.progress = 2;
        t.stage = format!("导出视频：{aspect_label}");
        t.video_error = None;
    });

    // 进度复用同一套 task-update 事件；100 留给收尾统一落，
    // 免得出现「进度 100 但 video_path 还没写」的闪烁。
    let progress = |pct: u8, stage: String| {
        queue.update(id, app, |t| {
            t.status = TaskStatus::Exporting;
            t.progress = pct.clamp(2, 99);
            t.stage = stage.clone();
        });
    };

    let resource_dir = app.path().resource_dir().ok();
    let result = crate::video::export(
        &audio,
        &out,
        &cfg,
        &title,
        &subtitle,
        resource_dir.as_deref(),
        &progress,
    )
    .await;

    // 收尾前再确认一次取消状态：取消不该被后面的完成覆盖。
    let was_cancelled = queue
        .get(id)
        .map(|t| t.status == TaskStatus::Cancelled)
        .unwrap_or(false);

    match result {
        Ok(res) => {
            if was_cancelled {
                return Err("任务已取消（视频文件已生成但状态保持已取消）".into());
            }
            let dur = res
                .duration_secs
                .map(|d| format!("（{d:.0} 秒）"))
                .unwrap_or_default();
            queue.update(id, app, |t| {
                t.status = TaskStatus::Completed;
                t.progress = 100;
                t.stage = format!("视频已导出{dur}");
                t.video_path = Some(res.output.to_string_lossy().to_string());
                t.video_error = None;
            });
            Ok(())
        }
        Err(e) => {
            let msg = format!("视频导出失败：{e}");
            if !was_cancelled {
                queue.update(id, app, |t| {
                    t.status = TaskStatus::Completed;
                    t.progress = 100;
                    t.stage = "完成（视频导出失败）".into();
                    t.video_error = Some(msg.clone());
                });
            }
            Err(msg)
        }
    }
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
        "doubao" => &conv.text_to_speech.doubao,
        _ => &conv.text_to_speech.openai,
    };
    let voice = if role == "answer" {
        v.answer.clone()
    } else {
        v.question.clone()
    };
    // 豆包：音色留空时给两位主持人不同的 bigtts 默认音色，避免两个人同一个嗓子。
    if model.eq_ignore_ascii_case("doubao") && voice.trim().is_empty() {
        return if role == "answer" {
            "zh_male_wennuanahu_moon_bigtts".to_string()
        } else {
            "zh_female_wanwanxiaohe_moon_bigtts".to_string()
        };
    }
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

/// 把行角色（`PERSON_1` / `PERSON_2`）映射到该主持人配置好的音色。
fn voice_for_role<'a>(question: &'a str, answer: &'a str, role: &str) -> &'a str {
    if role == "PERSON_2" {
        answer
    } else {
        question
    }
}

#[cfg(test)]
mod voice_role_tests {
    use super::voice_for_role;

    #[test]
    fn person2_uses_answer_voice() {
        assert_eq!(voice_for_role("host1", "host2", "PERSON_1"), "host1");
        assert_eq!(voice_for_role("host1", "host2", "PERSON_2"), "host2");
    }
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
    // u8 里算 `(hi - lo) * d as u8`、`/ total as u8` 都会出事：前者乘法溢出，
    // 后者在 total > 255 时截断（多行播客时可能变成 0 → 除零 panic）。升到 usize。
    let pct = lo + (((hi - lo) as usize * d) / total) as u8;
    queue.update(id, app, |t| t.progress = pct);
}

/// Keep clippy quiet about the unused Duration import pattern.
#[allow(dead_code)]
fn _unused() -> std::time::Duration {
    std::time::Duration::from_millis(1)
}
