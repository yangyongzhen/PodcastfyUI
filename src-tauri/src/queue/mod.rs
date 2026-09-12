//! Task queue, task state and transcript storage.
//!
//! Tasks run on a tokio task; progress is pushed to the frontend via
//! `task-update` events (see `pipeline.rs` for the real flow).

mod pipeline;
pub use pipeline::split_transcript;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, State};

/// Task status lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Extracting,
    Generating,
    Synthesizing,
    Muxing,
    Completed,
    Failed,
    Cancelled,
}

/// Input sources for a task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TaskInput {
    /// Web pages / YouTube links.
    pub urls: Vec<String>,
    /// Local PDF files.
    pub pdfs: Vec<String>,
    /// Raw text.
    pub text: String,
    /// Optional topic (web-search expansion).
    pub topic: String,
    pub longform: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub input: TaskInput,
    pub status: TaskStatus,
    /// 0..=100
    pub progress: u8,
    /// Human-readable stage message.
    pub stage: String,
    pub error: Option<String>,
    pub created_at: String,
    pub transcript_path: Option<String>,
    pub audio_path: Option<String>,
}

pub struct QueueManager {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
}

impl Clone for QueueManager {
    fn clone(&self) -> Self {
        Self {
            tasks: Arc::clone(&self.tasks),
        }
    }
}

impl Default for QueueManager {
    fn default() -> Self {
        Self::new()
    }
}

impl QueueManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn update(&self, id: &str, app: &AppHandle, f: impl FnOnce(&mut Task)) {
        let clone = {
            let mut m = self.tasks.lock().unwrap();
            m.get_mut(id).map(|t| {
                f(t);
                t.clone()
            })
        };
        if let Some(t) = clone {
            let _ = app.emit("task-update", &t);
        }
    }

    /// Register and start a generation task.
    pub fn start(
        &self,
        app: &AppHandle,
        title: String,
        input: TaskInput,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let task = Task {
            id: id.clone(),
            title,
            input: input.clone(),
            status: TaskStatus::Pending,
            progress: 0,
            stage: "queued".into(),
            error: None,
            created_at: now_iso(),
            transcript_path: None,
            audio_path: None,
        };
        self.tasks.lock().unwrap().insert(id.clone(), task.clone());
        let _ = app.emit("task-update", &task);

        let queue = self.clone();
        let app2 = app.clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            pipeline::run(&queue, &app2, &id2, input).await;
        });
        Ok(id)
    }

    pub fn list(&self) -> Vec<Task> {
        let mut v: Vec<Task> = self.tasks.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().get(id).cloned()
    }

    pub fn cancel(&self, id: &str) -> bool {
        let mut m = self.tasks.lock().unwrap();
        if let Some(t) = m.get_mut(id) {
            if !matches!(
                t.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                t.status = TaskStatus::Cancelled;
                t.stage = "cancelled".into();
                return true;
            }
        }
        false
    }

    pub fn delete(&self, id: &str) -> bool {
        self.tasks.lock().unwrap().remove(id).is_some()
    }

    pub fn get_transcript(&self, id: &str) -> Result<Option<String>, String> {
        let t = self.get(id).ok_or("task not found")?;
        let p = t.transcript_path.as_ref().ok_or("no transcript yet")?;
        Ok(Some(std::fs::read_to_string(p).map_err(|e| e.to_string())?))
    }

    pub fn save_transcript(&self, id: &str, content: String) -> Result<(), String> {
        let mut t = self.get(id).ok_or("task not found")?;
        let p = t
            .transcript_path
            .as_ref()
            .ok_or("no transcript yet")?
            .clone();
        std::fs::write(&p, content).map_err(|e| e.to_string())?;
        t.transcript_path = Some(p);
        self.tasks.lock().unwrap().insert(id.to_string(), t);
        Ok(())
    }

    /// 复用任务已保存的转录稿，只重跑 TTS + 拼接（不调用 LLM）。
    pub fn resynthesize(&self, app: &AppHandle, id: &str) -> Result<(), String> {
        let task = self
            .get(id)
            .ok_or_else(|| format!("task not found: {id}"))?;
        let transcript_path = task
            .transcript_path
            .ok_or("this task has no transcript yet — generate it first")?;
        let transcript =
            std::fs::read_to_string(&transcript_path).map_err(|e| e.to_string())?;
        if pipeline::split_transcript(&transcript).is_empty() {
            return Err("转录稿里没有可合成的对白行（每行需以 PERSON_1: / PERSON_2: 开头）".into());
        }

        let audio_path = std::path::PathBuf::from(&transcript_path)
            .parent()
            .ok_or("bad transcript path")?
            .join("podcast.mp3");
        let task_dir = audio_path
            .parent()
            .ok_or("bad audio path")?
            .to_path_buf();

        self.update(id, app, |t| {
            t.status = TaskStatus::Synthesizing;
            t.stage = "re-synthesizing audio…".into();
            t.progress = 50;
            t.error = None;
        });

        let queue = self.clone();
        let app = app.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            let settings = crate::config::load_settings(&app);
            let r = pipeline::synthesize_and_stitch(
                &queue,
                &app,
                &id,
                &settings,
                &transcript,
                &task_dir,
                &audio_path,
            )
            .await;
            if let Err(e) = r {
                queue.update(&id, &app, |t| {
                    t.status = TaskStatus::Failed;
                    t.stage = "failed".into();
                    t.progress = 0;
                    t.error = Some(e);
                });
            }
        });
        Ok(())
    }
}

/// ISO-8601 timestamp, seconds precision, UTC.
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = ((doe - doe / 1460 + doe / 36524 - doe / 146096) / 365) as i64;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe as u64 + yoe as u64 / 4 - yoe as u64 / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_task(
    state: State<'_, Arc<QueueManager>>,
    app: AppHandle,
    title: String,
    input: TaskInput,
) -> Result<String, String> {
    state.start(&app, title, input)
}

#[tauri::command]
pub fn list_tasks(state: State<'_, Arc<QueueManager>>) -> Vec<Task> {
    state.list()
}

#[tauri::command]
pub fn get_task(state: State<'_, Arc<QueueManager>>, id: String) -> Result<Task, String> {
    state
        .get(&id)
        .ok_or_else(|| format!("task not found: {id}"))
}

#[tauri::command]
pub fn cancel_task(state: State<'_, Arc<QueueManager>>, id: String) -> Result<bool, String> {
    Ok(state.cancel(&id))
}

#[tauri::command]
pub fn delete_task(state: State<'_, Arc<QueueManager>>, id: String) -> Result<bool, String> {
    Ok(state.delete(&id))
}

#[tauri::command]
pub fn resynthesize_task(
    state: State<'_, Arc<QueueManager>>,
    app: AppHandle,
    id: String,
) -> Result<(), String> {
    state.resynthesize(&app, &id)
}

#[tauri::command]
pub fn get_transcript(
    state: State<'_, Arc<QueueManager>>,
    id: String,
) -> Result<Option<String>, String> {
    state.get_transcript(&id)
}

#[tauri::command]
pub fn save_transcript(
    state: State<'_, Arc<QueueManager>>,
    id: String,
    content: String,
) -> Result<(), String> {
    state.save_transcript(&id, content)
}

#[tauri::command]
pub fn open_audio_file(state: State<'_, Arc<QueueManager>>, id: String) -> Result<(), String> {
    let t = state.get(&id).ok_or_else(|| format!("task not found: {id}"))?;
    let p = t.audio_path.clone().ok_or("no audio file yet")?;
    tauri_plugin_opener::open_path(&p, None::<&str>).map_err(|e| e.to_string())
}
