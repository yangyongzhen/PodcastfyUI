pub mod audio;
pub mod config;
pub mod error;
pub mod extractor;
pub mod generator;
pub mod health;
pub mod queue;
pub mod tts;

use queue::QueueManager;
use std::sync::Arc;

#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "podcastfyui_lib=info,tauri=info".into()),
        )
        .init();

    let queue = Arc::new(QueueManager::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::clone(&queue))
        .invoke_handler(tauri::generate_handler![
            app_info,
            config::get_api_keys,
            config::save_api_keys,
            config::get_llm_config,
            config::save_llm_config,
            config::get_conversation_config,
            config::save_conversation_config,
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
