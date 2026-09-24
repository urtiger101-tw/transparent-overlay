use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result, bail};
use image::ImageFormat;

use crate::{
    core::{self, render_frame},
    project::{OutputFormat, Project},
};

pub struct RenderResult {
    pub master: PathBuf,
    pub preview: Option<PathBuf>,
    pub frame_count: usize,
    pub duration_seconds: f32,
    pub warnings: Vec<String>,
}

pub fn render_master<F>(project: &Project, mut report: F) -> Result<RenderResult>
where
    F: FnMut(f32, String),
{
    project.validate()?;
    let ffmpeg = ffmpeg_path();
    let required = if project.make_preview {
        vec![project.format.encoder(), "libx264"]
    } else {
        vec![project.format.encoder()]
    };
    ensure_encoders(&ffmpeg, &required)?;

    let (assets, warnings) = core::load_assets(project)?;
    fs::create_dir_all(&project.output_dir)
        .with_context(|| format!("無法建立輸出資料夾：{}", project.output_dir.display()))?;
    let frames_dir = project.output_dir.join("frames");
    if project.keep_frames {
        fs::create_dir(&frames_dir)
            .with_context(|| format!("無法建立逐格輸出資料夾：{}", frames_dir.display()))?;
    }

    let total = project.total_frames()?;
    let duration = total as f32 / project.fps as f32;
    let master = project.output_dir.join(project.format.filename());
    report(0.0, format!("準備輸出 {total} 幀，約 {duration:.1} 秒"));

    let mut args = vec![
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-nostdin".to_owned(),
        "-n".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-video_size".to_owned(),
        format!("{}x{}", project.width, project.height),
        "-framerate".to_owned(),
        project.fps.to_string(),
        "-i".to_owned(),
        "pipe:0".to_owned(),
        "-frames:v".to_owned(),
        total.to_string(),
        "-an".to_owned(),
    ];
    append_encoder_args(&mut args, project.format);
    args.push(master.to_string_lossy().into_owned());

    let mut child = Command::new(&ffmpeg)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("無法啟動 FFmpeg：{}", ffmpeg.display()))?;
    let mut stdin = child.stdin.take().context("FFmpeg 輸入管線不存在")?;
    let stderr = child.stderr.take().context("FFmpeg 診斷管線不存在")?;
    let stderr_reader = thread::spawn(move || {
        let mut message = String::new();
        let _ = stderr.take(256 * 1024).read_to_string(&mut message);
        message
    });

    let slot_frames = project.slot_frames()?;
    let sample_frame = (slot_frames - project.transition_frames()?.max(1) / 2).min(total - 1);
    let mut sample_saved = false;

    let frame_result = (|| -> Result<()> {
        for frame_number in 0..total {
            let frame = render_frame(frame_number, &assets, project)?;
            stdin
                .write_all(frame.as_raw())
                .with_context(|| format!("傳送第 {frame_number} 幀至 FFmpeg 失敗"))?;

            if project.keep_frames {
                let path = frames_dir.join(format!("frame_{frame_number:05}.png"));
                frame
                    .save_with_format(&path, ImageFormat::Png)
                    .with_context(|| format!("無法儲存影格：{}", path.display()))?;
            }
            if frame_number == sample_frame {
                frame
                    .save(project.output_dir.join("transparent_sample.png"))
                    .context("無法儲存透明度抽樣圖")?;
                sample_saved = true;
            }

            if frame_number % project.fps.max(1) as usize == 0 || frame_number + 1 == total {
                let fraction = (frame_number + 1) as f32 / total as f32;
                report(
                    fraction * 0.82,
                    format!("合成影格 {}/{total}", frame_number + 1),
                );
            }
        }
        Ok(())
    })();
    drop(stdin);
    let status = child.wait().context("等待 FFmpeg 結束失敗")?;
    let diagnostic = stderr_reader
        .join()
        .unwrap_or_else(|_| "無法讀取 FFmpeg 診斷訊息".to_owned());
    frame_result?;
    if !status.success() {
        bail!("FFmpeg 編碼失敗：{}", diagnostic.trim());
    }
    if !sample_saved {
        bail!("動畫中沒有成功儲存透明影格抽樣。");
    }

    let preview = if project.make_preview {
        report(0.84, "建立棋盤格檢視影片".to_owned());
        Some(make_preview(project, &master, &ffmpeg, duration)?)
    } else {
        None
    };

    let manifest = serde_json::json!({
        "kit_version": env!("CARGO_PKG_VERSION"),
        "width": project.width,
        "height": project.height,
        "fps": project.fps,
        "frame_count": total,
        "duration_seconds": duration,
        "mode": format!("{:?}", project.mode).to_lowercase(),
        "seconds_per_slot_including_transition": project.slot_frames()? as f32 / project.fps as f32,
        "transition_seconds": if project.mode == crate::project::EffectMode::Carousel {
            project.transition_frames()? as f32 / project.fps as f32
        } else {
            0.0
        },
        "loopable": true,
        "alpha_mode": "straight",
        "source_precision": "8-bit RGBA",
        "outputs": [project.format.filename()],
        "png_frames_retained": project.keep_frames,
        "source_images": project.images.iter().map(|path| path.file_name().unwrap_or_default().to_string_lossy().to_string()).collect::<Vec<_>>(),
        "note": "Preview MP4 is opaque and not a transparent asset. No endpoint frame is duplicated."
    });
    fs::write(
        project.output_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .context("無法寫入輸出資訊 manifest.json")?;

    report(1.0, "透明動畫輸出完成".to_owned());
    Ok(RenderResult {
        master,
        preview,
        frame_count: total,
        duration_seconds: duration,
        warnings,
    })
}

fn append_encoder_args(args: &mut Vec<String>, format: OutputFormat) {
    match format {
        OutputFormat::ProRes => args.extend(
            [
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4",
                "-pix_fmt",
                "yuva444p10le",
                "-alpha_bits",
                "16",
                "-qscale:v",
                "4",
                "-threads",
                "2",
            ]
            .into_iter()
            .map(str::to_owned),
        ),
        OutputFormat::Qtrle => args.extend(
            ["-c:v", "qtrle", "-pix_fmt", "argb"]
                .into_iter()
                .map(str::to_owned),
        ),
        OutputFormat::Webm => args.extend(
            [
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-b:v",
                "0",
                "-crf",
                "25",
                "-auto-alt-ref",
                "0",
                "-deadline",
                "good",
                "-cpu-used",
                "4",
                "-threads",
                "2",
            ]
            .into_iter()
            .map(str::to_owned),
        ),
    }
}

fn make_preview(project: &Project, master: &Path, ffmpeg: &Path, duration: f32) -> Result<PathBuf> {
    let checker_path = project
        .output_dir
        .join("preview_background_CHECKERBOARD.png");
    core::checkerboard(project.width, project.height)
        .save(&checker_path)
        .context("無法建立棋盤格背景")?;
    let preview = project
        .output_dir
        .join("preview_checkerboard_NOT_TRANSPARENT.mp4");

    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-filter_complex_threads",
            "1",
            "-loop",
            "1",
            "-framerate",
        ])
        .arg(project.fps.to_string())
        .arg("-i")
        .arg(&checker_path)
        .args(["-stream_loop", "-1"]);
    if project.format.is_webm() {
        command.args(["-c:v", "libvpx-vp9"]);
    }
    let result = command
        .arg("-i")
        .arg(master)
        .args([
            "-filter_complex",
            "[0:v]setpts=PTS-STARTPTS[b];[1:v]setpts=PTS-STARTPTS[o];[b][o]overlay=alpha=straight:format=auto,format=yuv420p[v]",
            "-map",
            "[v]",
            "-an",
            "-r",
        ])
        .arg(project.fps.to_string())
        .args(["-fps_mode", "cfr", "-t"])
        .arg(format!("{:.6}", duration * 2.0))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-crf",
            "20",
            "-threads",
            "2",
            "-movflags",
            "+faststart",
        ])
        .arg(&preview)
        .output()
        .context("無法啟動 FFmpeg 建立棋盤格預覽")?;
    if !result.status.success() {
        bail!(
            "建立棋盤格預覽失敗：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(preview)
}

pub fn overlay_on_video<F>(
    base: &Path,
    overlay: &Path,
    output: &Path,
    x: i32,
    y: i32,
    width: Option<u32>,
    start: f64,
    end: Option<f64>,
    fps_override: Option<&str>,
    mut report: F,
) -> Result<()>
where
    F: FnMut(f32, String),
{
    if !base.is_file() || !overlay.is_file() {
        bail!("主影片與透明素材都必須是存在的檔案。");
    }
    if output.exists() {
        bail!("輸出影片已存在，請選擇其他檔名以保護既有檔案。");
    }
    if output
        .extension()
        .is_none_or(|ext| !ext.eq_ignore_ascii_case("mp4"))
    {
        bail!("最終影片輸出格式需使用 .mp4。");
    }
    if !start.is_finite() || start < 0.0 {
        bail!("開始時間需為 0 以上的有效數值。");
    }
    if let Some(end) = end
        && (!end.is_finite() || end <= start)
    {
        bail!("結束時間需大於開始時間。");
    }
    if width.is_some_and(|value| value < 2 || value % 2 != 0) {
        bail!("縮放寬度需是正偶數。");
    }

    let ffmpeg = ffmpeg_path();
    ensure_encoders(&ffmpeg, &["libx264", "aac"])?;
    let ffprobe = sibling_ffprobe(&ffmpeg);
    let fps = match fps_override {
        Some(value) => validate_fps(value.to_owned())?,
        None => probe_fps(&ffprobe, base)?,
    };
    report(0.05, format!("以 {fps} fps 讀取主影片"));

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).context("無法建立影片輸出資料夾")?;
    }
    let mut command = Command::new(&ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-filter_complex_threads",
            "1",
        ])
        .arg("-i")
        .arg(base)
        .args(["-stream_loop", "-1"]);
    if overlay
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
    {
        command.args(["-c:v", "libvpx-vp9"]);
    }
    command.arg("-i").arg(overlay).arg("-filter_complex");

    let mut asset_filter = "format=rgba".to_owned();
    if let Some(width) = width {
        asset_filter.push_str(&format!(",scale={width}:-2"));
    }
    asset_filter.push_str(&format!(",setpts=PTS-STARTPTS+{start:.9}/TB"));
    let mut enable = format!("gte(t,{start:.9})");
    if let Some(end) = end {
        enable.push_str(&format!("*lt(t,{end:.9})"));
    }
    let graph = format!(
        "[0:v]setpts=PTS-STARTPTS[b];[1:v]{asset_filter}[o];[b][o]overlay=x={x}:y={y}:alpha=straight:format=auto:shortest=1:enable='{enable}',pad=ceil(iw/2)*2:ceil(ih/2)*2,format=yuv420p[v]"
    );
    let result = command
        .arg(graph)
        .args([
            "-map",
            "[v]",
            "-map",
            "0:a?",
            "-af",
            "asetpts=PTS-STARTPTS",
            "-r",
        ])
        .arg(&fps)
        .args([
            "-fps_mode",
            "cfr",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-threads",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ])
        .arg(output)
        .output()
        .context("無法啟動 FFmpeg 合成主影片")?;
    if !result.status.success() {
        bail!(
            "主影片合成失敗：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    report(1.0, format!("主影片輸出完成：{}", output.display()));
    Ok(())
}

fn ensure_encoders(ffmpeg: &Path, required: &[&str]) -> Result<()> {
    let result = Command::new(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .with_context(|| format!("無法查詢 FFmpeg：{}", ffmpeg.display()))?;
    if !result.status.success() {
        bail!(
            "FFmpeg 無法列出 encoder：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    let listing = String::from_utf8_lossy(&result.stdout);
    let available: Vec<&str> = listing
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|encoder| !available.contains(encoder))
        .collect();
    if !missing.is_empty() {
        bail!("此 FFmpeg 缺少必要 encoder：{}", missing.join("、"));
    }
    Ok(())
}

fn probe_fps(ffprobe: &Path, video: &Path) -> Result<String> {
    let result = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate,r_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(video)
        .output()
        .with_context(|| format!("無法啟動 ffprobe：{}", ffprobe.display()))?;
    if !result.status.success() {
        bail!(
            "無法讀取主影片 fps：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    let output = String::from_utf8_lossy(&result.stdout);
    let rates: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0/0" && *value != "N/A")
        .collect();
    let rate = rates.first().context("主影片沒有可用的影格率。")?;
    validate_fps((*rate).to_owned())
}

fn validate_fps(fps: String) -> Result<String> {
    let value = if let Some((numerator, denominator)) = fps.split_once('/') {
        let numerator = numerator.parse::<f64>().context("主影片 fps 格式錯誤")?;
        let denominator = denominator.parse::<f64>().context("主影片 fps 格式錯誤")?;
        numerator / denominator
    } else {
        fps.parse::<f64>().context("主影片 fps 格式錯誤")?
    };
    if !value.is_finite() || !(0.0..=240.0).contains(&value) || value == 0.0 {
        bail!("影片影格率必須大於 0 且不超過 240。");
    }
    Ok(fps)
}

fn ffmpeg_path() -> PathBuf {
    std::env::var_os("FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn sibling_ffprobe(ffmpeg: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    if ffmpeg.components().count() == 1 {
        return PathBuf::from(name);
    }
    ffmpeg.parent().unwrap_or_else(|| Path::new(".")).join(name)
}
