//! Doubao (Volcengine) TTS real-synthesis probe — uses the PRODUCTION
//! `DoubaoTts` provider with the credentials already stored in the app data dir,
//! so a pass here means the app's TTS stage really reaches the service.
//!
//! Secrets are never printed — only their lengths.
//!
//! Run: cargo run --example doubao_tts_test

use podcastfyui_lib::tts::{doubao::DoubaoTts, TtsProvider};
use std::path::PathBuf;

fn app_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local/share/com.podcastfy.ui")
}

fn ffprobe_duration(path: &std::path::Path) -> String {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "?".into(),
    }
}

#[tokio::main]
async fn main() {
    let dir = app_dir();
    let keys_raw = std::fs::read_to_string(dir.join("api_keys.json")).unwrap_or_default();
    let conv_raw = std::fs::read_to_string(dir.join("conversation.yaml")).unwrap_or_default();

    let keys: serde_json::Value = serde_json::from_str(&keys_raw).unwrap_or_default();
    let conv: serde_yaml::Value = serde_yaml::from_str(&conv_raw).unwrap_or_default();

    let app_id = keys["doubao_app_id"].as_str().unwrap_or("").trim().to_string();
    let token = keys["doubao_access_token"].as_str().unwrap_or("").trim().to_string();
    let resource_id = keys["doubao_resource_id"].as_str().unwrap_or("").trim().to_string();
    let api_version = keys["doubao_api_version"].as_str().unwrap_or("").trim().to_string();

    let d = &conv["text_to_speech"]["doubao"];
    let cluster = d["model"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("volcano_tts")
        .to_string();
    let v1 = d["question"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("zh_female_wanwanxiaohe_moon_bigtts")
        .to_string();
    let v2 = d["answer"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("zh_male_wennuanahu_moon_bigtts")
        .to_string();

    // 可选：命令行覆盖音色，用来验证某个 voice_type 是否已授权：
    //   cargo run --example doubao_tts_test -- <voice1> [voice2]
    let args: Vec<String> = std::env::args().skip(1).collect();
    let v1 = args.first().cloned().unwrap_or(v1);
    let v2 = args.get(1).cloned().unwrap_or(v2);

    println!(
        "config: app_id={} chars, token={} chars, resource_id='{}', api_version='{}', cluster='{}'",
        app_id.chars().count(),
        token.chars().count(),
        if resource_id.is_empty() { "(空→默认 volc.service_type.10029)" } else { &resource_id },
        if api_version.is_empty() { "(空→默认 v1)" } else { &api_version },
        cluster
    );

    if app_id.is_empty() || token.is_empty() {
        println!("RESULT 0/2 — 凭证未配置，无法测试");
        return;
    }

    let provider = DoubaoTts::new(app_id, token, cluster, v1.clone(), resource_id, api_version);
    let out_dir = PathBuf::from("/tmp/doubao-tts-test");

    // 极短文本（约 20 字），只为验证协议与鉴权，尽量少耗额度。
    let cases: [(&str, &str, &str); 2] = [
        ("你好，这是一次豆包语音合成的连通性测试。", &v1, "p1.mp3"),
        ("收到，协议与鉴权都通了。", &v2, "p2.mp3"),
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
                    "OK   {name} voice={voice} bytes={size} head={head:02x?} secs={}",
                    ffprobe_duration(&path)
                );
                ok += 1;
            }
            Err(e) => println!("FAIL {name} voice={voice}: {e}"),
        }
    }
    println!("RESULT {ok}/2 lines -> {}", out_dir.display());
}
