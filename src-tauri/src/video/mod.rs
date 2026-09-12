//! 视频导出（L1 静态封面 / L2 波形）。
//!
//! 职责很窄：拼 ffmpeg 滤镜图 → 用随包中文字体把标题/副标题画上去 → 编码成 mp4。
//! 素材全部来自上游产物（成品 mp3 + 视频配置 + 可选封面图），不碰生成逻辑。
//!
//! 四种组合（横版/竖版 × 波形/封面）已在真机上逐个跑通，下述参数即当时的验证值；
//! 改动这里请同步跑 `cargo run --example video_export_test`。

use crate::audio::{find_ffmpeg, tail};
use crate::config::VideoConfig;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// 随包中文字体文件名（来源与许可见 `src-tauri/fonts/README.md`）。
const BUNDLED_FONT: &str = "DroidSansFallbackFull.ttf";

/// 波形配色（品牌青两档）。
const WAVE_COLORS: &str = "0x22D3EE|0x67E8F9";
/// 自绘封面渐变：深底 → 品牌青。
const GRADIENT_FROM: &str = "0x0B1220";
const GRADIENT_TO: &str = "0x0E7490";

/// 进度回调：`(0-100, 阶段描述)`。
pub type Progress<'a> = &'a (dyn Fn(u8, String) + Send + Sync);

/// 导出结果。
#[derive(Debug, Clone)]
pub struct ExportResult {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    /// 产物时长（秒）；ffprobe 不可用时为 `None`，不影响导出成功。
    pub duration_secs: Option<f64>,
}

/// 定位中文字体：自定义路径 → 打包资源目录 → 仓库内（开发期与 example）。
///
/// 三级都找不到时**提前报错**，而不是让 ffmpeg 画出一屏方块。
pub fn resolve_font(explicit: &str, resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    let explicit = explicit.trim();
    if !explicit.is_empty() {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("指定的字体文件不存在：{}", p.display()));
    }
    if let Some(dir) = resource_dir {
        let p = dir.join("fonts").join(BUNDLED_FONT);
        if p.is_file() {
            return Ok(p);
        }
    }
    // 开发期直接跑（dev / example）时没有资源目录，回落到仓库内的字体。
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fonts")
        .join(BUNDLED_FONT);
    if dev.is_file() {
        return Ok(dev);
    }
    Err(format!(
        "找不到中文字体 {BUNDLED_FONT}（随包资源缺失）；可在设置页指定自定义字体路径"
    ))
}

/// 路径塞进 ffmpeg 滤镜图时需要的转义（反斜杠、冒号、单引号）。
fn escape_filter_path(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// 坍成单行：drawtext 不会自动换行，带换行的标题会溢出画面。
fn single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 用 ffprobe 探测时长（优先取与 ffmpeg 同目录的那个，其次 PATH），失败返回 `None`。
fn probe_duration(file: &Path) -> Option<f64> {
    let sibling = find_ffmpeg()
        .ok()
        .and_then(|f| f.parent().map(|d| d.to_path_buf()))
        .and_then(|d| {
            ["ffprobe", "ffprobe.exe"]
                .iter()
                .map(|n| d.join(n))
                .find(|p| p.exists())
        });
    let probe: PathBuf = sibling.unwrap_or_else(|| PathBuf::from("ffprobe"));
    let out = std::process::Command::new(probe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok()
}

/// 产物路径：横版 `video.mp4`，竖版 `video-portrait.mp4`。
///
/// 用文件名区分画幅，因此同一任务可以先出横版、改设置后再出竖版，两者并存不互相覆盖。
pub fn output_path(task_dir: &Path, config: &VideoConfig) -> PathBuf {
    let mut cfg = config.clone();
    cfg.normalize();
    if cfg.aspect == "portrait" {
        task_dir.join("video-portrait.mp4")
    } else {
        task_dir.join("video.mp4")
    }
}

/// 叠在画面上的文字滤镜链（标题 + 副标题）。
///
/// 字号与位置即真机验证值：横版 64 / 36 号、距顶 8%，竖版 44 / 24 号、距顶 10%。
/// 标题带半透明底衬，压在波形上也能看清；副标题用品牌青。
/// 两者都没有时返回 `null`，让滤镜图仍是合法的直通链。
fn build_text_filters(
    font: &Path,
    title: Option<&Path>,
    subtitle: Option<&Path>,
    portrait: bool,
) -> String {
    let (title_fs, sub_fs, top, gap) = if portrait {
        (44, 24, "h*0.10", 68)
    } else {
        (64, 36, "h*0.08", 92)
    };
    let font_esc = escape_filter_path(font);
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = title {
        parts.push(format!(
            "drawtext=fontfile='{font_esc}':textfile='{}':fontcolor=white:fontsize={title_fs}:x=(w-text_w)/2:y={top}:box=1:boxcolor=0x000000AA:boxborderw=18",
            escape_filter_path(t)
        ));
    }
    if let Some(s) = subtitle {
        parts.push(format!(
            "drawtext=fontfile='{font_esc}':textfile='{}':fontcolor=0xA5F3FC:fontsize={sub_fs}:x=(w-text_w)/2:y={top}+{gap}",
            escape_filter_path(s)
        ));
    }
    if parts.is_empty() {
        return "null".into();
    }
    parts.join(",")
}

/// 导出视频：把成品音频配成一段带中文标题的 mp4。
///
/// - `title` / `subtitle` 都可为空；都空时输出纯视觉视频（不报错）。
/// - `resource_dir` 传 `app.path().resource_dir()`；example 与测试里传 `None`。
/// - 只读音频、只写 `out`：失败不会破坏上游 mp3（调用方据此保证「视频失败不影响音频」）。
pub async fn export(
    audio: &Path,
    out: &Path,
    config: &VideoConfig,
    title: &str,
    subtitle: &str,
    resource_dir: Option<&Path>,
    progress: Progress<'_>,
) -> Result<ExportResult, String> {
    if !audio.is_file() {
        return Err(format!("音频文件不存在：{}", audio.display()));
    }
    let mut cfg = config.clone();
    cfg.normalize();
    let (w, h) = cfg.size();
    let portrait = cfg.aspect == "portrait";

    let font = resolve_font(&cfg.font_path, resource_dir)?;

    // 封面来源校验放在最前面：自选图缺失时给可操作报错，而不是让 ffmpeg 抛难懂的错。
    let cover_image: Option<PathBuf> = if cfg.style == "cover" && cfg.cover == "custom" {
        let p = PathBuf::from(cfg.cover_path.trim());
        if p.as_os_str().is_empty() {
            return Err("已选择「自选图片」作为封面，但还没有选择图片文件".into());
        }
        if !p.is_file() {
            return Err(format!("封面图片不存在：{}", p.display()));
        }
        Some(p)
    } else {
        None
    };

    progress(5, "准备画面…".into());

    // 文字走 textfile 而不是内联 text：中文、引号、冒号、逗号统统不用转义，
    // 这是唯一不会踩 ffmpeg 滤镜转义坑的方式。
    let scratch = out.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let title_file = scratch.join("video-title.txt");
    let subtitle_file = scratch.join("video-subtitle.txt");
    let title_line = single_line(title);
    let subtitle_line = single_line(subtitle);
    let mut text_files: Vec<PathBuf> = Vec::new();
    let title_arg = if title_line.is_empty() {
        None
    } else {
        tokio::fs::write(&title_file, &title_line)
            .await
            .map_err(|e| format!("写入标题失败：{e}"))?;
        text_files.push(title_file.clone());
        Some(title_file.as_path())
    };
    let subtitle_arg = if subtitle_line.is_empty() {
        None
    } else {
        tokio::fs::write(&subtitle_file, &subtitle_line)
            .await
            .map_err(|e| format!("写入副标题失败：{e}"))?;
        text_files.push(subtitle_file.clone());
        Some(subtitle_file.as_path())
    };
    let text_filters = build_text_filters(&font, title_arg, subtitle_arg, portrait);

    let audio_str = audio.to_string_lossy().to_string();
    let mut args: Vec<String> = match (cfg.style.as_str(), cover_image.as_ref()) {
        // L1 + 自选图：铺满目标画幅后居中裁切
        ("cover", Some(img)) => {
            let graph = format!(
                "[0:v]scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},format=yuv420p[bg];[bg]{text_filters}[v]"
            );
            vec![
                "-loop".into(),
                "1".into(),
                "-i".into(),
                img.to_string_lossy().to_string(),
                "-i".into(),
                audio_str.clone(),
                "-filter_complex".into(),
                graph,
                "-map".into(),
                "[v]".into(),
                "-map".into(),
                "1:a".into(),
            ]
        }
        // L1 + 自绘底图：品牌深底 → 青的渐变
        ("cover", None) => {
            let graph = format!("[0:v]format=yuv420p[bg];[bg]{text_filters}[v]");
            vec![
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!("gradients=s={w}x{h}:c0={GRADIENT_FROM}:c1={GRADIENT_TO}:rate=25"),
                "-i".into(),
                audio_str.clone(),
                "-filter_complex".into(),
                graph,
                "-map".into(),
                "[v]".into(),
                "-map".into(),
                "1:a".into(),
            ]
        }
        // L2 波形（默认）
        _ => {
            let graph = format!(
                "[0:a]showwaves=s={w}x{h}:mode=cline:colors={WAVE_COLORS}:rate=25,format=yuv420p[base];[base]{text_filters}[v]"
            );
            vec![
                "-i".into(),
                audio_str,
                "-filter_complex".into(),
                graph,
                "-map".into(),
                "[v]".into(),
                "-map".into(),
                "0:a".into(),
            ]
        }
    };
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "23".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "160k".into(),
        "-shortest".into(),
        out.to_string_lossy().to_string(),
    ]);

    let ffmpeg = find_ffmpeg()?;
    progress(30, "编码视频…".into());
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new(&ffmpeg)
            .args(["-y", "-nostdin", "-v", "error"])
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("spawn ffmpeg: {e}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("视频导出失败：{}", tail(&err, 800)));
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // 字幕临时文件不留痕（它们是中间产物，不属于导出结果）。
    for f in &text_files {
        let _ = tokio::fs::remove_file(f).await;
    }

    if !out.is_file() {
        return Err("ffmpeg 退出成功但没有生成视频文件".into());
    }
    progress(100, "视频已导出".into());
    Ok(ExportResult {
        output: out.to_path_buf(),
        width: w,
        height: h,
        duration_secs: probe_duration(out),
    })
}

/// 探明「实际会用哪个字体」：设置页据此显示状态，而不是让用户猜。
///
/// 返回解析后的字体绝对路径；自定义路径无效或随包字体缺失时返回可操作错误。
#[tauri::command]
pub fn video_font_status(app: tauri::AppHandle, font_path: String) -> Result<String, String> {
    use tauri::Manager;
    let resource_dir = app.path().resource_dir().ok();
    resolve_font(&font_path, resource_dir.as_deref()).map(|p| p.display().to_string())
}
