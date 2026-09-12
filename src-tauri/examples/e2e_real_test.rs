//! Full REAL end-to-end: real LLM (whatever `llm.json` points at) → real Doubao
//! TTS per line → real ffmpeg concat, producing a listenable mp3.
//!
//! Unlike `smoke_test`, nothing here is mocked. Secrets are never printed.
//!
//! Run: cargo run --example e2e_real_test

use podcastfyui_lib::audio;
use podcastfyui_lib::config::{ConversationConfig, LlmConfig};
use podcastfyui_lib::generator::{Generator, GeneratorConfig, Provider};
use podcastfyui_lib::queue::split_transcript;
use podcastfyui_lib::tts::{doubao::DoubaoTts, TtsProvider};
use std::path::PathBuf;

/// 与流水线中豆包音色留空时的回退保持一致（两位主持人不同嗓子）。
const FALLBACK_VOICE_1: &str = "zh_female_wanwanxiaohe_moon_bigtts";
const FALLBACK_VOICE_2: &str = "zh_male_wennuanahu_moon_bigtts";

fn app_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local/share/com.podcastfy.ui")
}

/// 与 `queue::pipeline::voice_for_role` 同一规则（该函数是私有的，此处对齐复刻）。
fn voice_for_role<'a>(question: &'a str, answer: &'a str, role: &str) -> &'a str {
    if role == "PERSON_2" {
        answer
    } else {
        question
    }
}

fn ffprobe_duration(path: &std::path::Path) -> String {
    match std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "?".into(),
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let dir = app_dir();
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap_or_default();

    let llm: LlmConfig =
        serde_json::from_str(&read("llm.json")).map_err(|e| format!("llm.json: {e}"))?;
    let keys: serde_json::Value =
        serde_json::from_str(&read("api_keys.json")).map_err(|e| format!("api_keys.json: {e}"))?;
    let conv: ConversationConfig =
        serde_yaml::from_str(&read("conversation.yaml")).map_err(|e| format!("conversation.yaml: {e}"))?;

    let api_key = keys["openai"].as_str().unwrap_or("").trim().to_string();
    println!(
        "[cfg] LLM provider={} model={} base_url={} key={}chars | 输出语言={}",
        llm.provider,
        llm.model,
        llm.base_url,
        api_key.chars().count(),
        conv.output_language
    );

    // 素材可用 EP_SOURCE 覆盖，省得为换个主题重新改代码；默认用项目自身的技术介绍。
    let source = std::env::var("EP_SOURCE").unwrap_or_else(|_| {
        "Rust 的所有权系统在编译期消除数据竞争：每个值有唯一所有者，离开作用域即释放；\
借用分为可变与不可变，可变借用是独占的。Tauri 用 Rust 做桌面后端，前端只能通过命令与事件与它通信。"
            .to_string()
    });
    println!("[cfg] 素材 {} 字符（EP_SOURCE {}）", source.chars().count(), if std::env::var("EP_SOURCE").is_ok() { "已覆盖" } else { "未设置，用默认" });

    let work = PathBuf::from("/tmp/e2e-real");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("parts")).map_err(|e| e.to_string())?;

    // ---- 1) 真实 LLM -----------------------------------------------------
    let provider_kind = match llm.provider.as_str() {
        "openai" => Provider::OpenAi,
        other => return Err(format!("example 只接了 openai 兼容通路，当前 provider={other}")),
    };
    let gen = Generator::new(GeneratorConfig {
        provider: provider_kind,
        model: llm.model.clone(),
        api_key: Some(api_key),
        base_url: Some(llm.base_url.clone()),
        temperature: llm.temperature,
        max_tokens: llm.max_tokens,
    });
    let transcript_path = work.join("transcript.txt");
    let gen_progress = |_stage: &str, _pct: u8| {};
    let t0 = std::time::Instant::now();
    let transcript = gen
        .generate_qa(&source, &conv, false, &transcript_path, &gen_progress)
        .await
        .map_err(|e| format!("generation failed: {e}"))?;
    println!(
        "[1] LLM OK: {} 字符 / {} 行 / {:.1}s",
        transcript.chars().count(),
        transcript.lines().count(),
        t0.elapsed().as_secs_f32()
    );

    // ---- 2) 真实豆包 TTS（每行一个 mp3）---------------------------------
    let app_id = keys["doubao_app_id"].as_str().unwrap_or("").trim().to_string();
    let token = keys["doubao_access_token"].as_str().unwrap_or("").trim().to_string();
    let resource_id = keys["doubao_resource_id"].as_str().unwrap_or("").trim().to_string();
    let api_version = keys["doubao_api_version"].as_str().unwrap_or("").trim().to_string();
    if app_id.is_empty() || token.is_empty() {
        return Err("豆包凭证未配置，无法做真实 TTS".into());
    }

    let d = &conv.text_to_speech.doubao;
    let v1 = if d.question.trim().is_empty() {
        FALLBACK_VOICE_1.to_string()
    } else {
        d.question.clone()
    };
    let v2 = if d.answer.trim().is_empty() {
        FALLBACK_VOICE_2.to_string()
    } else {
        d.answer.clone()
    };
    println!("[2] 主持人音色: PERSON_1={v1} | PERSON_2={v2}");

    let tts = DoubaoTts::new(app_id, token, "volcano_tts".into(), v1.clone(), resource_id, api_version);
    let lines = split_transcript(&transcript);
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut failed = 0usize;
    let t1 = std::time::Instant::now();
    for (i, (text, role)) in lines.iter().enumerate() {
        let voice = voice_for_role(&v1, &v2, role);
        let out = work.join("parts").join(format!("{i:04}.mp3"));
        match tts.synthesize_line(text, voice, &out).await {
            Ok(()) => parts.push(out),
            Err(e) => {
                failed += 1;
                println!("    line {i} ({role}) FAIL: {e}");
            }
        }
    }
    println!(
        "[3] TTS OK: {}/{} 行 → mp3，用时 {:.1}s",
        parts.len(),
        lines.len(),
        t1.elapsed().as_secs_f32()
    );
    if parts.is_empty() {
        return Err("没有任何一行合成成功".into());
    }

    // ---- 3) 真实 ffmpeg 拼接 --------------------------------------------
    let final_path = work.join("podcast.mp3");
    audio::concatenate_parts(&parts, &final_path)
        .await
        .map_err(|e| format!("concat failed: {e}"))?;
    let size = std::fs::metadata(&final_path).map_err(|e| e.to_string())?.len();
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries",
               "stream=codec_name,sample_rate,channels", "-of", "csv=p=0"])
        .arg(&final_path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!(
        "[4] concat OK: {} ({} bytes, {} 秒, {}){}",
        final_path.display(),
        size,
        ffprobe_duration(&final_path),
        probe,
        if failed > 0 { format!("  ⚠ {failed} 行失败") } else { String::new() }
    );
    println!("RESULT: 真实端到端完成，成品在 {}", final_path.display());
    Ok(())
}
