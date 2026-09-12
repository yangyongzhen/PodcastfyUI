//! Edge TTS real-synthesis probe — uses the PRODUCTION `EdgeTts` provider
//! (same struct the app builds for `default_tts_model: edge`), so a pass here
//! means the app's TTS stage reaches the service with the real protocol.
//!
//! Run: cargo run --example edge_tts_test

use podcastfyui_lib::tts::{EdgeTts, TtsProvider};
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() {
    let out_dir = PathBuf::from("/tmp/edge-tts-test");
    // Voices as configured in conversation.yaml (简体中文 preset).
    let provider = EdgeTts::new("zh-CN-XiaoxiaoNeural", "zh-CN-YunxiNeural");

    let cases: [(&str, &str, &str); 2] = [
        (
            "你好，欢迎收听这档播客节目，今天我们聊聊人工智能。",
            "zh-CN-XiaoxiaoNeural",
            "p1.mp3",
        ),
        (
            "我觉得很有意思，那我们从最基础的概念开始说起吧。",
            "zh-CN-YunxiNeural",
            "p2.mp3",
        ),
    ];

    let mut ok = 0;
    for (text, voice, name) in cases {
        let path: PathBuf = out_dir.join(name);
        match provider.synthesize_line(text, voice, &path).await {
            Ok(()) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let head = std::fs::read(&path)
                    .map(|b| b.iter().take(3).cloned().collect::<Vec<u8>>())
                    .unwrap_or_default();
                println!(
                    "OK   {name} voice={voice} bytes={size} head={head:02x?} text={text}"
                );
                ok += 1;
            }
            Err(e) => println!("FAIL {name} voice={voice}: {e}"),
        }
    }
    println!("RESULT {ok}/2 lines synthesized into {}", Path::new("/tmp/edge-tts-test").display());
}
