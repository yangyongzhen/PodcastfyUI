//! Audio assembly via ffmpeg (sidecar binary or PATH).

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Locate the ffmpeg binary: bundled (next to the executable) or on PATH.
pub fn find_ffmpeg() -> Result<PathBuf, String> {
    // 1) Next to the running executable (distribution bundling).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["ffmpeg", "ffmpeg.exe"] {
                let p = dir.join(name);
                if p.exists() {
                    return Ok(p);
                }
            }
        }
    }
    // 2) On PATH.
    let path_var = std::env::var("PATH").map_err(|_| "PATH not set".to_string())?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(sep) {
        let p = Path::new(dir).join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
        if p.exists() {
            return Ok(p);
        }
    }
    Err("ffmpeg not found (install it or bundle it next to the executable)".to_string())
}

/// Concatenate audio parts into `out`, normalizing to 24 kHz mono mp3 so
/// mixed provider outputs (edge 24kHz / openai 24kHz) line up cleanly.
pub async fn concatenate_parts(parts: &[PathBuf], out: &Path) -> Result<(), String> {
    if parts.is_empty() {
        return Err("no audio parts to concatenate".into());
    }
    if parts.len() == 1 {
        // Re-encode the single part so the output format is consistent.
        return reencode(&parts[0], out).await;
    }

    let ffmpeg = find_ffmpeg()?;
    // Build an ffmpeg concat demuxer list file.
    let list_path = out.with_extension("concat.txt");
    let mut list = String::new();
    for p in parts {
        let escaped = p.to_string_lossy().replace('\'', "'\\''");
        list.push_str(&format!("file '{}'\n", escaped));
    }
    tokio::fs::write(&list_path, list).await.map_err(|e| e.to_string())?;

    let out_str = out.to_string_lossy().to_string();
    let list_str = list_path.to_string_lossy().to_string();
    let result = tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                &list_str,
                "-ar",
                "24000",
                "-ac",
                "1",
                "-b:a",
                "96k",
                "-c:a",
                "libmp3lame",
                &out_str,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("spawn ffmpeg: {e}"))?;
        std::fs::remove_file(&list_str).ok();
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ffmpeg failed: {}", tail(&err, 800)));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

/// Re-encode a single audio file to a normalized mp3.
pub async fn reencode(src: &Path, out: &Path) -> Result<(), String> {
    let ffmpeg = find_ffmpeg()?;
    let src_str = src.to_string_lossy().to_string();
    let out_str = out.to_string_lossy().to_string();
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-i",
                &src_str,
                "-ar",
                "24000",
                "-ac",
                "1",
                "-b:a",
                "96k",
                "-c:a",
                "libmp3lame",
                &out_str,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("spawn ffmpeg: {e}"))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ffmpeg reencode failed: {}", tail(&err, 800)));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Last `n` chars of a string (for error excerpts).
pub(crate) fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}
