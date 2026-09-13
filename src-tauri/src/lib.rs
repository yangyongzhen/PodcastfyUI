pub mod audio;
pub mod config;
pub mod error;
pub mod extractor;
pub mod generator;
pub mod health;
pub mod queue;
pub mod tts;
pub mod video;

use queue::QueueManager;
use std::sync::Arc;
use tauri::Manager;

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// 放行 asset 协议读取某个目录（空串 = 放行默认的 AppData 根）。
///
/// 自定义输出目录一旦不在 AppData 下，`convertFileSrc` 生成的地会被协议拒绝，
/// 用户看到的就是「点播放/看视频没反应」；所以保存设置时和启动时都要同步放行。
pub fn allow_asset_dir(app: &tauri::AppHandle, dir: &str) {
    let target = if dir.trim().is_empty() {
        app.path().app_data_dir().ok()
    } else {
        Some(std::path::PathBuf::from(dir.trim()))
    };
    if let Some(p) = target {
        if let Err(e) = app.asset_protocol_scope().allow_directory(&p, true) {
            tracing::warn!("allow asset dir {} failed: {e}", p.display());
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "podcastfyui_lib=info,tauri=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 任务表持久化：索引和产物都在 AppData 下，启动时恢复上次的任务列表。
            match app.path().app_data_dir() {
                Ok(dir) => {
                    app.manage(Arc::new(QueueManager::with_store(&dir)));
                }
                Err(e) => {
                    tracing::warn!("app data dir unavailable, task list will not persist: {e}");
                    app.manage(Arc::new(QueueManager::new()));
                }
            }
            // 启动就放行已保存的自定义输出目录，否则本次运行里预览会失效。
            let dir = config::load_settings(app.handle()).output.dir.clone();
            allow_asset_dir(app.handle(), &dir);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            config::get_api_keys,
            config::save_api_keys,
            config::get_llm_config,
            config::save_llm_config,
            config::get_conversation_config,
            config::save_conversation_config,
            config::get_video_config,
            config::save_video_config,
            config::get_search_config,
            config::save_search_config,
            config::get_output_config,
            config::save_output_config,
            config::get_default_output_dir,
            video::video_font_status,
            queue::export_video_task,
            queue::open_video_file,
            health::test_connection,
            queue::start_task,
            queue::list_tasks,
            queue::get_task,
            queue::cancel_task,
            queue::delete_task,
            queue::get_transcript,
            queue::save_transcript,
            queue::resynthesize_task,
            queue::open_audio_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
