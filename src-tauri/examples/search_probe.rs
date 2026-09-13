//! 主题搜索探针：不读应用配置、不提供任何 key，直接验证 `SearchCfg` 的兜底链
//! （`auto` 模式 → Exa 托管 MCP）是否真能拿到可用于生成播客的素材。
//!
//! 跑法：
//! ```bash
//! cd src-tauri && cargo run --example search_probe
//! cargo run --example search_probe -- "鸿蒙 HarmonyOS 开发入门" bocha
//! ```

use podcastfyui_lib::extractor::{Extractor, SearchCfg};

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let topic = args
        .next()
        .unwrap_or_else(|| "纯 Rust 重写播客生成工具的可行性".to_string());
    let provider = args.next().unwrap_or_else(|| "auto".to_string());

    // 刻意留空所有 key：auto 模式下只剩 Exa（免密钥）与 DuckDuckGo 可试。
    let cfg = SearchCfg {
        provider: provider.clone(),
        num_results: 5,
        ..Default::default()
    };

    println!("== search_probe ==");
    println!("  topic    : {topic}");
    println!("  provider : {provider}（所有 key 均为空）");

    let extractor = Extractor::new();
    let started = std::time::Instant::now();
    let material = match extractor.search_topic(&topic, &cfg).await {
        Ok(m) => {
            println!(
                "  status   : OK in {:.1}s, {} chars",
                started.elapsed().as_secs_f64(),
                m.chars().count()
            );
            m
        }
        Err(e) => {
            println!(
                "  status   : FAILED in {:.1}s — {e}",
                started.elapsed().as_secs_f64()
            );
            return Err(e.to_string());
        }
    };

    if material.trim().is_empty() {
        return Err("搜索结果为空".into());
    }
    let head: String = material.chars().take(400).collect();
    println!("  --- 前 400 字 ---\n{head}\n  ------------------");
    println!("SEARCH PROBE PASS");
    Ok(())
}
