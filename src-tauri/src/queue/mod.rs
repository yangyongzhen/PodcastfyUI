//! Task queue, task state and transcript storage.
//!
//! Tasks run on a tokio task; progress is pushed to the frontend via
//! `task-update` events (see `pipeline.rs` for the real flow).

mod pipeline;
pub use pipeline::split_transcript;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// 可选第五阶段：把已完成的 mp3 导出成视频（失败不回退 mp3 产物）。
    Exporting,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    /// 是否处于「进程还在推进」的状态；进程退出后这些状态不可能再继续。
    pub fn is_running(self) -> bool {
        matches!(
            self,
            TaskStatus::Pending
                | TaskStatus::Extracting
                | TaskStatus::Generating
                | TaskStatus::Synthesizing
                | TaskStatus::Muxing
                | TaskStatus::Exporting
        )
    }
}

/// 任务索引文件名；放 AppData 根，与产物目录 `tasks/` 区分开。
const TASK_INDEX: &str = "task_index.json";
/// 产物目录名；值与 `pipeline.rs` 的 `WORKSPACE` 保持一致。
const WORKSPACE: &str = "tasks";

/// 索引文件载荷。带 `version` 便于以后调整格式。
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct TaskIndex {
    version: u32,
    tasks: Vec<Task>,
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
    /// 已导出的视频路径（横版 `video.mp4` / 竖版 `video-portrait.mp4`）。
    pub video_path: Option<String>,
    /// 上一次视频导出失败的原因；与 `error` 分开，避免污染已成功的音频产物。
    pub video_error: Option<String>,
}

pub struct QueueManager {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    /// 索引文件路径；`None` 表示不持久化（单元测试用的 `new()`）。
    store: Option<PathBuf>,
}

impl Clone for QueueManager {
    fn clone(&self) -> Self {
        Self {
            tasks: Arc::clone(&self.tasks),
            store: self.store.clone(),
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
            store: None,
        }
    }

    /// 带持久化的管理器：索引放 `<app_dir>/task_index.json`，并扫描
    /// `<app_dir>/tasks/*/` 把本改动之前的历史任务也恢复出来。
    pub fn with_store(app_dir: &Path) -> Self {
        let q = Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            store: Some(app_dir.join(TASK_INDEX)),
        };
        q.load(app_dir);
        q
    }

    /// 产物目录根：`<AppData>/tasks`（自定义输出目录由产物路径反推，不走这里）。
    fn workspace_dir(&self) -> Option<PathBuf> {
        Some(self.store.as_ref()?.parent()?.join(WORKSPACE))
    }

    /// 启动时恢复任务表。
    ///
    /// 两路来源缺一不可：索引只覆盖本改动之后创建的任务，历史任务在磁盘上只有产物目录；
    /// 而任何「进行中」的状态在进程退出后都不可能再推进，统一归一为失败。
    fn load(&self, app_dir: &Path) {
        let mut restored: HashMap<String, Task> = HashMap::new();

        if let Some(p) = &self.store {
            match std::fs::read_to_string(p) {
                Ok(s) => match serde_json::from_str::<TaskIndex>(&s) {
                    Ok(idx) => {
                        for t in idx.tasks {
                            restored.insert(t.id.clone(), t);
                        }
                    }
                    Err(e) => tracing::warn!("task index parse failed ({}): {e}", p.display()),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("task index read failed ({}): {e}", p.display()),
            }
        }

        let ws = app_dir.join(WORKSPACE);
        let mut scanned = 0usize;
        if let Ok(rd) = std::fs::read_dir(&ws) {
            for entry in rd.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().to_string();
                if restored.contains_key(&id) {
                    continue;
                }
                if let Some(t) = task_from_dir(&id, &entry.path()) {
                    restored.insert(id, t);
                    scanned += 1;
                }
            }
        }

        // 进程退出时「进行中」的状态不可能再推进，载入时必须按产物归一出真实结果。
        // 要复核两种情形：
        //   1. 进程退出时还在「进行中」的状态；
        //   2. 旧版本把「进行中」一律判失败时留下的 `interrupted` 标记——它只是
        //      「没走完」的猜测，产物证据可以推翻，否则一张产物齐全的任务会永久
        //      卡在「失败」上（本机就撞到过）。
        // 判定依据是产物本身：`podcast.mp3` 在盘上说明生成阶段确实走完了，中断
        // 只可能落在收尾的可选视频导出上；这与 pipeline「视频导出失败仍算完成、
        // 原因记在 `video_error`」的约定一致。
        let mut reconciled = 0usize;
        for t in restored.values_mut() {
            let was_running = t.status.is_running();
            let stale_interrupted = t.status == TaskStatus::Failed && t.stage == "interrupted";
            if !was_running && !stale_interrupted {
                continue;
            }
            let has_audio = t
                .audio_path
                .as_deref()
                .map(|p| Path::new(p).exists())
                .unwrap_or(false);
            if has_audio {
                let was_exporting = was_running && t.status == TaskStatus::Exporting;
                t.status = TaskStatus::Completed;
                t.progress = 100;
                t.error = None;
                if was_exporting {
                    // 视频导出直写目标文件（无原子改名），中断时无法担保文件完好，
                    // 如实标注并给出补救动作，但不动路径——原文件也许仍是完好产物。
                    t.stage = "完成（视频导出被中断）".into();
                    t.video_error =
                        Some("上次导出被中断，如需视频请重新导出（音频产物完好）".into());
                } else {
                    t.stage = "completed".into();
                }
                reconciled += 1;
            } else if was_running {
                t.status = TaskStatus::Failed;
                t.stage = "interrupted".into();
                t.error = Some("任务在应用退出时被中断，可重新发起".into());
                reconciled += 1;
            }
            // 无产物 + 已有 interrupted 标记：原本就如实，保持不动。
        }

        let total = restored.len();
        *self.lock_tasks() = restored;
        tracing::info!(
            "task index: restored {total} task(s) ({scanned} from workspace, {reconciled} reconciled after restart) @ {}",
            ws.display()
        );
        // 索引缺失时会就此生成，后续启动走快路径。
        self.persist();
    }

    /// 原子写回索引文件。失败只告警：落盘是附加保障，绝不能影响任务本身。
    fn persist(&self) {
        let Some(path) = self.store.as_ref() else {
            return;
        };
        let mut tasks: Vec<Task> = self.lock_tasks().values().cloned().collect();
        tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let payload = TaskIndex { version: 1, tasks };
        let json = match serde_json::to_vec_pretty(&payload) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("task index serialize failed: {e}");
                return;
            }
        };
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!("task index dir {} failed: {e}", dir.display());
                return;
            }
        }
        // 先写临时文件再 rename：中途崩溃不会留下半截 JSON 让下次启动解析失败。
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &json) {
            tracing::warn!("task index write {} failed: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            tracing::warn!("task index rename {} failed: {e}", path.display());
        }
    }

    /// 取任务表锁。
    ///
    /// 后台 task 线程 panic 会毒化这把 Mutex；此处若直接 `unwrap()`，主线程
    /// （GTK 事件循环，例如 `list()`）就会踩 `PoisonError` 再 panic 一次，而 GTK
    /// 回调不能 unwind → 整个进程 abort、窗口消失。任务表的数据本身仍然一致，
    /// 取内层数据继续用即可。
    fn lock_tasks(&self) -> std::sync::MutexGuard<'_, HashMap<String, Task>> {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, id: &str, app: &AppHandle, f: impl FnOnce(&mut Task)) {
        let clone = {
            let mut m = self.lock_tasks();
            m.get_mut(id).map(|t| {
                f(t);
                t.clone()
            })
        };
        if let Some(t) = clone {
            let _ = app.emit("task-update", &t);
            self.persist();
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
            video_path: None,
            video_error: None,
        };
        self.lock_tasks().insert(id.clone(), task.clone());
        let _ = app.emit("task-update", &task);
        self.persist();

        let queue = self.clone();
        let app2 = app.clone();
        let id2 = id.clone();
        // 必须用 tauri 的运行时句柄，不能用裸 `tokio::spawn`：
        // 同步命令跑在主线程（GTK 事件循环）上，那里没有 Tokio reactor，
        // `tokio::spawn` 会 panic，而 GTK 回调不能 unwind，会直接把进程 abort。
        tauri::async_runtime::spawn(async move {
            pipeline::run(&queue, &app2, &id2, input).await;
        });
        Ok(id)
    }

    pub fn list(&self) -> Vec<Task> {
        let mut v: Vec<Task> = self.lock_tasks().values().cloned().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.lock_tasks().get(id).cloned()
    }

    pub fn cancel(&self, id: &str) -> bool {
        let changed = {
            let mut m = self.lock_tasks();
            match m.get_mut(id) {
                Some(t)
                    if !matches!(
                        t.status,
                        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                    ) =>
                {
                    t.status = TaskStatus::Cancelled;
                    t.stage = "cancelled".into();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.persist();
        }
        changed
    }

    /// 删除任务并清掉它的产物文件。
    ///
    /// 必须连产物一起删：否则下次启动扫描 `tasks/*/` 会把「已删除」的任务又找回来。
    /// 只删本任务记录过的已知产物（不整目录递归），索引万一被改坏也不会误删别的东西。
    pub fn delete(&self, id: &str) -> bool {
        let task = match self.lock_tasks().remove(id) {
            Some(t) => t,
            None => return false,
        };
        let mut dir: Option<PathBuf> = None;
        for f in [
            task.audio_path.clone(),
            task.transcript_path.clone(),
            task.video_path.clone(),
        ]
        .into_iter()
        .flatten()
        {
            let p = PathBuf::from(&f);
            if dir.is_none() {
                dir = p.parent().map(|d| d.to_path_buf());
            }
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("remove {} failed: {e}", p.display()),
            }
        }
        if dir.is_none() {
            // 索引里的任务可能还没产出任何文件（例如建了任务就退出），
            // 但目录里可能留着 parts/ 之类残料，按 id 兜底找一次。
            dir = self.workspace_dir().map(|w| w.join(id));
        }
        if let Some(d) = dir {
            for name in [
                "video.mp4",
                "video-portrait.mp4",
                "video-title.txt",
                "video-subtitle.txt",
            ] {
                let _ = std::fs::remove_file(d.join(name));
            }
            let parts = d.join("parts");
            if parts.is_dir() {
                let _ = std::fs::remove_dir_all(&parts);
            }
            // 目录只剩空壳时顺手收掉；非空（用户自己放的文件）就留着。
            let _ = std::fs::remove_dir(&d);
        }
        self.persist();
        true
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
        self.lock_tasks().insert(id.to_string(), t);
        self.persist();
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
    iso_from_secs(secs)
}

/// Unix 秒 → ISO-8601（秒精度，UTC）。
fn iso_from_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3600, (rem / 60) % 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// 历史任务没有索引记录，只能拿目录 mtime 当创建时间。
fn dir_mtime_iso(dir: &Path) -> Option<String> {
    let modified = std::fs::metadata(dir).ok()?.modified().ok()?;
    let secs = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    Some(iso_from_secs(secs))
}

/// 从产物目录重建一个历史任务（本改动之前的任务在磁盘上只有产物）。
///
/// 三个产物都没有的目录不算任务——那是「建了目录但没跑完」的空壳，列出来只会干扰；
/// 而只有转录稿、没有 mp3 的目录保留为失败态，方便用户看到"那次没跑完"。
fn task_from_dir(id: &str, dir: &Path) -> Option<Task> {
    let transcript = dir.join("transcript.txt");
    let audio = dir.join("podcast.mp3");
    let video = ["video.mp4", "video-portrait.mp4"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.exists());

    if !audio.exists() && !transcript.exists() && video.is_none() {
        return None;
    }
    let has_audio = audio.exists();
    Some(Task {
        id: id.to_string(),
        title: title_from_dir(id, &transcript),
        input: TaskInput::default(),
        status: if has_audio {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        },
        progress: if has_audio { 100 } else { 0 },
        stage: if has_audio {
            "completed".into()
        } else {
            "interrupted".into()
        },
        error: if has_audio {
            None
        } else {
            Some("产物不完整：没有找到 podcast.mp3".into())
        },
        created_at: dir_mtime_iso(dir).unwrap_or_else(now_iso),
        transcript_path: transcript
            .exists()
            .then(|| transcript.to_string_lossy().to_string()),
        audio_path: audio.exists().then(|| audio.to_string_lossy().to_string()),
        video_path: video.map(|p| p.to_string_lossy().to_string()),
        video_error: None,
    })
}

/// 历史任务的标题：目录名是 `日期-可读标题-短id`（见 `pipeline::task_dir_name`）时取中间那段，
/// 纯 uuid 的旧目录退而取转录稿第一句。
fn title_from_dir(id: &str, transcript: &Path) -> String {
    let parts: Vec<&str> = id.split('-').collect();
    let two = |s: &&str| s.len() == 2 && s.chars().all(|c| c.is_ascii_digit());
    let dated = parts.len() >= 4
        && parts[0].len() == 4
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && two(&parts[1])
        && two(&parts[2]);
    if dated && parts.len() > 4 {
        let topic = parts[3..parts.len() - 1].join("-");
        if !topic.trim().is_empty() {
            return topic;
        }
    }
    if let Ok(s) = std::fs::read_to_string(transcript) {
        for line in s.lines() {
            let raw = line.trim();
            if raw.is_empty()
                || raw.starts_with("PODCASTIFY")
                || raw.to_uppercase().starts_with("YOUR PERSONAL")
            {
                continue;
            }
            let body = raw
                .strip_prefix("PERSON_1:")
                .or_else(|| raw.strip_prefix("PERSON_2:"))
                .unwrap_or(raw)
                .trim();
            if body.is_empty() {
                continue;
            }
            let short: String = body.chars().take(42).collect();
            return if body.chars().count() > 42 {
                format!("{short}…")
            } else {
                short
            };
        }
    }
    format!("历史任务 {}", id.chars().take(8).collect::<String>())
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

/// 打开产物文件（优先在文件管理器中定位），**始终把绝对路径回传**给前端。
///
/// 无桌面环境（没有文件管理器、没有 mime handler）时 `xdg-open` 会静默失败且不报错，
/// 界面表现只是「点了没反应」；有了路径，前端至少能把文件位置告诉用户。
fn reveal_or_open(path: Option<String>) -> Result<String, String> {
    let path = path.ok_or_else(|| "no output file yet".to_string())?;
    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("file not found: {path}"));
    }
    if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(&p)
        .or_else(|_| tauri_plugin_opener::open_path(&p, None::<&str>))
    {
        tracing::warn!("reveal/open failed for {path}: {e}");
    }
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_audio_file(state: State<'_, Arc<QueueManager>>, id: String) -> Result<String, String> {
    let t = state.get(&id).ok_or_else(|| format!("task not found: {id}"))?;
    reveal_or_open(t.audio_path.clone())
}

/// 导出视频（可选第五阶段）。返回更新后的任务，前端可直接刷新卡片。
///
/// 失败时音频产物保持不变：错误记在 `video_error`，任务状态仍是 `completed`。
#[tauri::command]
pub async fn export_video_task(
    app: AppHandle,
    state: State<'_, Arc<QueueManager>>,
    id: String,
) -> Result<Task, String> {
    let queue: &Arc<QueueManager> = state.inner();
    pipeline::export_video(queue, &app, &id).await?;
    state.get(&id).ok_or_else(|| format!("task not found: {id}"))
}

#[tauri::command]
pub fn open_video_file(state: State<'_, Arc<QueueManager>>, id: String) -> Result<String, String> {
    let t = state.get(&id).ok_or_else(|| format!("task not found: {id}"))?;
    reveal_or_open(t.video_path.clone())
}

#[cfg(test)]
mod spawn_tests {
    use std::sync::mpsc;
    use std::time::Duration;

    /// 回归测试：在没有 Tokio reactor 的线程上派生异步任务。
    ///
    /// 这正是 Tauri **同步**命令的执行环境（GTK 主线程）。此处若用裸 `tokio::spawn`
    /// 会 panic "there is no reactor running"，而 GTK 回调不能 unwind，第二次 panic
    /// 会把整个进程 abort（窗口直接消失）。必须走 `tauri::async_runtime`。
    #[test]
    fn async_runtime_spawn_works_off_runtime_thread() {
        let h = std::thread::spawn(|| {
            let (tx, rx) = mpsc::channel();
            tauri::async_runtime::spawn(async move {
                let _ = tx.send(7u8);
            });
            rx.recv_timeout(Duration::from_secs(10))
        });
        let got = h.join().expect("派生线程自身 panic 了");
        assert!(matches!(got, Ok(7)), "非运行时线程上的 spawn 没有跑起来: {got:?}");
    }

    /// 回归测试：后台 task 线程 panic 毒化队列锁后，主线程读数不能跟着 panic。
    ///
    /// 之前 `list()` 等是 `lock().unwrap()`：毒化后在 GTK 主线程再 panic 一次，
    /// 而 GTK 回调不能 unwind → 整个进程 abort（用户看到的就是「窗口消失」）。
    #[test]
    fn queue_survives_poisoned_lock() {
        let q = super::QueueManager::new();
        let holder = q.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = holder.tasks.lock().unwrap();
            panic!("模拟 task 线程持锁时 panic");
        }));
        assert!(q.tasks.is_poisoned(), "测试前提不成立：锁没有被毒化");
        assert!(q.list().is_empty(), "毒化后 list() 仍必须可用");
        assert!(q.get("missing").is_none(), "毒化后 get() 仍必须可用");
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;

    fn temp_app_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pfy-index-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 历史任务在磁盘上只有产物目录，必须能从目录扫描恢复出来（标题取可读目录名）。
    #[test]
    fn restores_task_from_workspace_dir() {
        let app = temp_app_dir("scan");
        let dir = app.join(WORKSPACE).join("2026-09-13-我的播客-90a5b181");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("podcast.mp3"), b"fake").unwrap();
        std::fs::write(dir.join("transcript.txt"), "PERSON_1: 你好").unwrap();
        // 空壳目录不算任务
        std::fs::create_dir_all(app.join(WORKSPACE).join("empty-dir")).unwrap();

        let q = QueueManager::with_store(&app);
        let list = q.list();
        assert_eq!(list.len(), 1, "空壳目录不该出现在列表里: {list:?}");
        let t = &list[0];
        assert_eq!(t.title, "我的播客", "可读目录名里的标题没取到");
        assert_eq!(t.status, TaskStatus::Completed);
        assert!(t.audio_path.is_some() && t.transcript_path.is_some());
        assert!(app.join(TASK_INDEX).exists(), "载入后应生成索引文件");
        let _ = std::fs::remove_dir_all(&app);
    }

    /// 纯 uuid 的旧目录名没有可读标题，退化为转录稿首句。
    #[test]
    fn old_uuid_dir_takes_title_from_transcript() {
        let app = temp_app_dir("uuid");
        let dir = app.join(WORKSPACE).join("90a5b181-16be-416f-bed9-b0a4ffbbfee0");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("podcast.mp3"), b"fake").unwrap();
        std::fs::write(
            dir.join("transcript.txt"),
            "PODCASTIFY\nYour Personal Generative AI Podcast\n\nPERSON_1: 欢迎回到播客\n",
        )
        .unwrap();

        let q = QueueManager::with_store(&app);
        let t = &q.list()[0];
        assert_eq!(
            t.title, "欢迎回到播客",
            "uuid 目录应回落到转录稿首句: {}",
            t.title
        );
        let _ = std::fs::remove_dir_all(&app);
    }

    /// 进程退出时还在「进行中」的状态不可能再推进：没有产物才判失败。
    #[test]
    fn stale_running_status_becomes_interrupted() {
        let app = temp_app_dir("stale");
        let stale = Task {
            id: "aaaa1111".into(),
            title: "跑一半".into(),
            input: TaskInput::default(),
            status: TaskStatus::Generating,
            progress: 42,
            stage: "generating".into(),
            error: None,
            created_at: "2026-09-13T00:00:00Z".into(),
            transcript_path: None,
            audio_path: None,
            video_path: None,
            video_error: None,
        };
        let idx = TaskIndex {
            version: 1,
            tasks: vec![stale],
        };
        std::fs::write(app.join(TASK_INDEX), serde_json::to_vec_pretty(&idx).unwrap()).unwrap();

        let q = QueueManager::with_store(&app);
        let t = &q.list()[0];
        assert_eq!(t.status, TaskStatus::Failed, "无产物的进行中状态应归一为失败");
        assert_eq!(t.stage, "interrupted");
        assert!(t.error.is_some());
        let _ = std::fs::remove_dir_all(&app);
    }

    /// 反过来：`podcast.mp3` 已在盘上就说明生成阶段走完了，中断只落在可选视频导出上，
    /// 判失败会让用户以为白跑一场（正是这张卡报「失败」的由来）。
    #[test]
    fn running_task_with_audio_restores_as_completed() {
        let app = temp_app_dir("resume-audio");
        let dir = app.join(WORKSPACE).join("bbbb2222");
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        let audio = dir.join("podcast.mp3");
        std::fs::write(&audio, b"fake").unwrap();

        let idx = TaskIndex {
            version: 1,
            tasks: vec![Task {
                id: "bbbb2222".into(),
                title: "已出音频".into(),
                input: TaskInput::default(),
                status: TaskStatus::Exporting,
                progress: 30,
                stage: "导出视频：横版 16:9".into(),
                error: None,
                created_at: "2026-09-13T00:00:00Z".into(),
                transcript_path: None,
                audio_path: Some(audio.to_string_lossy().to_string()),
                video_path: None,
                video_error: None,
            }],
        };
        std::fs::write(app.join(TASK_INDEX), serde_json::to_vec_pretty(&idx).unwrap()).unwrap();

        let q = QueueManager::with_store(&app);
        let t = &q.list()[0];
        assert_eq!(t.status, TaskStatus::Completed, "音频产物齐全不应判失败");
        assert_eq!(t.progress, 100);
        assert_eq!(t.stage, "完成（视频导出被中断）");
        assert!(t.video_error.is_some(), "中断原因要如实交代");
        assert!(t.error.is_none());
        let _ = std::fs::remove_dir_all(&app);
    }

    /// 旧版本已经写进索引的「interrupted 失败」也要能被产物推翻，否则那张卡会
    /// 永久卡在「失败」上——只复核「运行中」状态是不够的。
    #[test]
    fn stale_interrupted_marker_is_healed_from_products() {
        let app = temp_app_dir("heal-marker");
        let dir = app.join(WORKSPACE).join("cccc3333");
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("podcast.mp3");
        std::fs::write(&audio, b"fake").unwrap();

        let idx = TaskIndex {
            version: 1,
            tasks: vec![Task {
                id: "cccc3333".into(),
                title: "被旧版本误判".into(),
                input: TaskInput::default(),
                status: TaskStatus::Failed,
                progress: 30,
                stage: "interrupted".into(),
                error: Some("任务在应用退出时被中断，可重新发起".into()),
                created_at: "2026-09-13T00:00:00Z".into(),
                transcript_path: None,
                audio_path: Some(audio.to_string_lossy().to_string()),
                video_path: None,
                video_error: None,
            }],
        };
        std::fs::write(app.join(TASK_INDEX), serde_json::to_vec_pretty(&idx).unwrap()).unwrap();

        let q = QueueManager::with_store(&app);
        let t = &q.list()[0];
        assert_eq!(t.status, TaskStatus::Completed, "产物齐全应洗掉误判的失败标记");
        assert_eq!(t.progress, 100);
        assert_eq!(t.stage, "completed");
        assert!(t.error.is_none());

        // 无产物时不该被洗白：如实保持失败。
        let idx = TaskIndex {
            version: 1,
            tasks: vec![Task {
                id: "cccc3333".into(),
                title: "确实没产物".into(),
                input: TaskInput::default(),
                status: TaskStatus::Failed,
                progress: 30,
                stage: "interrupted".into(),
                error: Some("任务在应用退出时被中断，可重新发起".into()),
                created_at: "2026-09-13T00:00:00Z".into(),
                transcript_path: None,
                audio_path: Some(dir.join("missing.mp3").to_string_lossy().to_string()),
                video_path: None,
                video_error: None,
            }],
        };
        std::fs::write(app.join(TASK_INDEX), serde_json::to_vec_pretty(&idx).unwrap()).unwrap();
        let q2 = QueueManager::with_store(&app);
        assert_eq!(q2.list()[0].status, TaskStatus::Failed);
        assert_eq!(q2.list()[0].stage, "interrupted");
        let _ = std::fs::remove_dir_all(&app);
    }

    /// 删除任务要连产物一起清，否则重启扫描会把它找回来。
    #[test]
    fn delete_removes_products_so_it_does_not_resurrect() {
        let app = temp_app_dir("delete");
        let id = "2026-09-13-待删-aaaa1111";
        let dir = app.join(WORKSPACE).join(id);
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join("podcast.mp3"), b"fake").unwrap();
        std::fs::write(dir.join("transcript.txt"), "PERSON_1: 你好").unwrap();

        let q = QueueManager::with_store(&app);
        assert_eq!(q.list().len(), 1);
        assert!(q.delete(id));
        assert!(!dir.join("podcast.mp3").exists(), "产物没被删掉");

        let again = QueueManager::with_store(&app);
        assert!(
            again.list().is_empty(),
            "删除后重启不该复活: {:?}",
            again.list()
        );
        let _ = std::fs::remove_dir_all(&app);
    }
}
