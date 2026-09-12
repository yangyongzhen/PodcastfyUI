//! 视频导出真实机验证（L1 静态封面 / L2 波形）。
//!
//! 走的和应用内「导出视频」按钮**同一条代码路径**（`video::export()`），只是绕开 Tauri
//! 的 AppHandle：字体与 ffmpeg 的定位在 example 下走同一套回落逻辑。
//!
//! 覆盖五件事：
//!   1. 横版波形（16:9）+ 中文标题/副标题
//!   2. 竖版波形（9:16）
//!   3. 静态封面（自绘渐变）
//!   4. 静态封面（自选图，铺满后居中裁切）
//!   5. 失败路径：封面图不存在 / 字体不存在 → 必须提前报错
//!
//! 另外验证两件容易「看起来成功其实不对」的事：
//!   - **文字真的烧进画面**：同配置再出一版无标题的，比较画面顶部亮度，不同才算证成；
//!   - **音频没被碰过**：整轮前后比对 mp3 的大小与 md5。
//!
//! Run: cargo run --example video_export_test
//! 可用 `EP_AUDIO=/path/to.mp3` 换输入。

use podcastfyui_lib::config::VideoConfig;
use podcastfyui_lib::video;
use std::path::{Path, PathBuf};
use std::time::Instant;

const TITLE: &str = "播客自动化：从网页到双人对话音频";
const SUBTITLE: &str = "podcastfyui · 纯 Rust 复刻";

fn cfg(aspect: &str, style: &str, cover: &str, cover_path: &str) -> VideoConfig {
    VideoConfig {
        aspect: aspect.into(),
        style: style.into(),
        cover: cover.into(),
        cover_path: cover_path.into(),
        title: TITLE.into(),
        subtitle: SUBTITLE.into(),
        // 留空 = 用随包中文字体（example 下回落到仓库内 fonts/）。
        font_path: String::new(),
    }
}

fn md5(path: &Path) -> String {
    std::process::Command::new("md5sum")
        .arg(path)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default()
}

fn human(bytes: u64) -> String {
    format!("{:.2} MB", bytes as f64 / 1_048_576.0)
}

/// 容器/流的关键参数，用来确认不是「生成了一个坏 mp4」。
fn probe(path: &Path) -> String {
    std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// 画面顶部四分之一区域的平均亮度（YAVG）。有中文标题（白字 + 半透明底衬）时必然不同。
fn top_luma(path: &Path) -> Option<f64> {
    // 注意：metadata=print 是走 ffmpeg 的 info 日志输出的，这里**不能**加 `-v error`，
    // 否则连要解析的 YAVG 一起吞掉（`-nostats` 只关进度行，不影响 metadata）。
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "info", "-nostats", "-ss", "3", "-i"])
        .arg(path)
        .args([
            "-vf",
            "crop=iw:ih/4:0:0,signalstats,metadata=print:key=lavfi.signalstats.YAVG",
            "-frames:v",
            "1",
            "-f",
            "null",
            "-",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let line = text.lines().find(|l| l.contains("YAVG"))?;
    line.split('=').nth(1)?.trim().parse::<f64>().ok()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let audio = PathBuf::from(
        std::env::var("EP_AUDIO").unwrap_or_else(|_| "/tmp/podcastfy-demo.mp3".into()),
    );
    if !audio.is_file() {
        return Err(format!(
            "缺少输入音频 {}；先跑 `cargo run --example e2e_real_test`，或用 EP_AUDIO 指定",
            audio.display()
        ));
    }

    let out_dir = PathBuf::from("/tmp/vtest");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    // 音频完整性基线：整轮结束时必须一字不差。
    let size_before = std::fs::metadata(&audio).map_err(|e| e.to_string())?.len();
    let md5_before = md5(&audio);
    println!(
        "[in] 音频 {} · {} · md5 {}",
        audio.display(),
        human(size_before),
        md5_before
    );
    let font = video::resolve_font("", None)?;
    println!("[in] 随包中文字体 {}", font.display());
    println!("[in] 输出目录 {}", out_dir.display());

    // 自选图封面的素材：现造一张，既不依赖外部图片也没有授权问题。
    let custom_cover = out_dir.join("custom-cover.png");
    if !custom_cover.is_file() {
        let st = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "gradients=s=1600x900:c0=0x1E1B4B:c1=0x0891B2:d=1",
                "-frames:v",
                "1",
            ])
            .arg(&custom_cover)
            .status()
            .map_err(|e| e.to_string())?;
        if !st.success() {
            return Err("生成测试封面图失败".into());
        }
    }

    let cases: Vec<(&str, &str, VideoConfig)> = vec![
        (
            "横版波形 1280x720",
            "landscape-wave",
            cfg("landscape", "wave", "generated", ""),
        ),
        (
            "竖版波形 720x1280",
            "portrait-wave",
            cfg("portrait", "wave", "generated", ""),
        ),
        (
            "横版静态封面（自绘渐变）",
            "cover-generated",
            cfg("landscape", "cover", "generated", ""),
        ),
        (
            "横版静态封面（自选图裁切）",
            "cover-custom",
            cfg(
                "landscape",
                "cover",
                "custom",
                custom_cover.to_string_lossy().as_ref(),
            ),
        ),
    ];

    let progress = |pct: u8, stage: String| {
        if pct == 100 || pct % 30 == 0 {
            println!("      · {pct}% {stage}");
        }
    };

    let mut fails: Vec<String> = Vec::new();
    let mut durations: Vec<(String, f64)> = Vec::new();

    for (label, slug, c) in &cases {
        let dir = out_dir.join(slug);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = video::output_path(&dir, c);
        let _ = std::fs::remove_file(&out);
        println!("\n[case] {label} → {}", out.display());

        let t = Instant::now();
        match video::export(&audio, &out, c, &c.title, &c.subtitle, None, &progress).await {
            Ok(res) => {
                let secs = t.elapsed().as_secs_f64();
                let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                let (want_w, want_h) = if c.aspect == "portrait" {
                    (720, 1280)
                } else {
                    (1280, 720)
                };
                if (res.width, res.height) != (want_w, want_h) {
                    fails.push(format!("{label}: 分辨率 {}x{} != 期望 {want_w}x{want_h}", res.width, res.height));
                }
                match res.duration_secs {
                    Some(d) => durations.push((label.to_string(), d)),
                    None => fails.push(format!("{label}: 拿不到时长")),
                }
                println!(
                    "  OK {}x{} · {:.1} 秒 · {} · 编码耗时 {:.1}s",
                    res.width,
                    res.height,
                    res.duration_secs.unwrap_or(0.0),
                    human(bytes),
                    secs
                );
                println!("  ffprobe: {}", probe(&out));
            }
            Err(e) => {
                fails.push(format!("{label}: {e}"));
                println!("  FAIL {e}");
            }
        }
    }

    // ---- 证明「中文真的烧进画面」，而不是只生成了一个能播的 mp4 ----------------
    println!("\n[proof] 文字烧入：同配置再出一版无标题，比较画面顶部亮度");
    let with_text = out_dir.join("landscape-wave").join("video.mp4");
    let bare_dir = out_dir.join("no-text");
    std::fs::create_dir_all(&bare_dir).map_err(|e| e.to_string())?;
    let bare_out = bare_dir.join("video.mp4");
    let bare = cfg("landscape", "wave", "generated", "");
    match video::export(&audio, &bare_out, &bare, "", "", None, &progress).await {
        Ok(_) => {
            let a = top_luma(&with_text);
            let b = top_luma(&bare_out);
            match (a, b) {
                (Some(x), Some(y)) => {
                    println!("  有字顶部 YAVG={x:.2} · 无字顶部 YAVG={y:.2} · 差值 {:.2}", (x - y).abs());
                    if (x - y).abs() < 1.0 {
                        fails.push("有字与无字的画面几乎一致，无法证明中文烧入".into());
                    }
                }
                _ => fails.push("取样帧亮度失败（ffmpeg signalstats 不可用？）".into()),
            }
        }
        Err(e) => fails.push(format!("对照导出（无标题）失败：{e}")),
    }

    // ---- 失败路径：必须提前报错，而不是产出一个豆腐字/空画面的 mp4 -------------
    println!("\n[case] 失败路径 1：封面图不存在");
    let bad_cfg = cfg("landscape", "cover", "custom", "/tmp/vtest/no-such-cover.png");
    let bad_dir = out_dir.join("bad-cover");
    std::fs::create_dir_all(&bad_dir).map_err(|e| e.to_string())?;
    match video::export(
        &audio,
        &bad_dir.join("video.mp4"),
        &bad_cfg,
        TITLE,
        SUBTITLE,
        None,
        &progress,
    )
    .await
    {
        Ok(_) => fails.push("封面图不存在时居然导出成功".into()),
        Err(e) => println!("  OK 按预期拦截：{e}"),
    }

    println!("\n[case] 失败路径 2：字体文件不存在");
    match video::resolve_font("/tmp/vtest/no-such-font.ttf", None) {
        Ok(p) => fails.push(format!("坏字体路径居然解析成功：{}", p.display())),
        Err(e) => println!("  OK 按预期拦截：{e}"),
    }

    // ---- 音频完整性：导出只读输入，mp3 必须一字未改 ---------------------------
    let size_after = std::fs::metadata(&audio).map_err(|e| e.to_string())?.len();
    let md5_after = md5(&audio);
    println!(
        "\n[in] 音频 {} · {} · md5 {}",
        audio.display(),
        human(size_after),
        md5_after
    );
    if size_after != size_before || md5_after != md5_before {
        fails.push("音频文件被改动了".into());
    } else {
        println!("  OK 音频大小与 md5 完全一致（导出全程只读输入）");
    }

    if durations.is_empty() && !fails.is_empty() {
        println!("\nVIDEO EXPORT FAIL — 没有任何一组导出成功");
        return Err(fails.join("\n"));
    }

    if fails.is_empty() {
        println!(
            "\nVIDEO EXPORT PASS — {} 组导出成功 + 2 条失败路径按预期拦截；文字烧入已证；音频 md5 未变",
            durations.len()
        );
        Ok(())
    } else {
        println!("\nVIDEO EXPORT FAIL — {} 项未通过：", fails.len());
        for f in &fails {
            println!("  · {f}");
        }
        Err("存在未通过项".into())
    }
}
