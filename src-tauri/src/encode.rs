//! 编码/分离执行层：构造 ffmpeg 命令、跑进度、支持取消。
//!
//! 输出设计（对应"最恰当格式 + 更完整的输出方式"）：
//! - 音频：FLAC（无损，波形/转写最优）或 mp3；声道三档 native / stereo / center
//! - 视频预览版：H.264 mp4，720p，短 GOP，无音轨，faststart —— 供打轴时看画面
//! - 视频高质量版：H.264 mp4，原分辨率，CRF 可控，无音轨
//! - 原样 remux：`-c copy`，零损失快速归档

use crate::tools;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

static CANCEL: AtomicBool = AtomicBool::new(false);

pub fn request_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

pub fn reset_cancel() {
    CANCEL.store(false, Ordering::SeqCst);
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct EncodeRequest {
    pub video: String,
    pub audio_ordinal: i64,
    pub channel_layout: String,
    pub out_dir: String,
    pub base_name: String,
    pub duration: f64,

    pub want_audio: bool,
    pub audio_codec: String,
    pub audio_mode: String,
    pub audio_bitrate: String,
    /// 音轨相对视频的偏移（毫秒）；apply_align 为真时用于补偿
    pub delay_ms: i64,
    pub apply_align: bool,

    pub want_preview: bool,
    pub preview_height: u32,
    pub preview_crf: u32,

    pub want_high: bool,
    pub high_crf: u32,

    pub want_remux: bool,
}

impl Default for EncodeRequest {
    fn default() -> Self {
        Self {
            video: String::new(),
            audio_ordinal: 0,
            channel_layout: String::new(),
            out_dir: String::new(),
            base_name: String::new(),
            duration: 0.0,
            want_audio: true,
            audio_codec: "flac".into(),
            audio_mode: "stereo".into(),
            audio_bitrate: "320k".into(),
            delay_ms: 0,
            apply_align: true,
            want_preview: true,
            preview_height: 720,
            preview_crf: 30,
            want_high: false,
            high_crf: 18,
            want_remux: false,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct JobEvent {
    pub job_id: String,
    pub stage: String,
    pub percent: f32,
    pub message: String,
    pub done: bool,
    pub error: bool,
    pub outputs: Vec<String>,
}

/// 声音滤镜链：处理时间轴延迟 + 中置声道提取
fn audio_filter_chain(req: &EncodeRequest) -> String {
    let mut parts: Vec<String> = Vec::new();

    if req.apply_align {
        if req.delay_ms > 0 {
            // 音轨晚于视频：头部补静音
            parts.push(format!("adelay={}:all=1", req.delay_ms));
        } else if req.delay_ms < 0 {
            // 音轨早于视频：裁掉开头
            let start = (-req.delay_ms) as f64 / 1000.0;
            parts.push(format!("atrim=start={start:.6},asetpts=PTS-STARTPTS"));
        }
    }

    if req.audio_mode == "center" {
        let layout = req.channel_layout.to_lowercase();
        if layout.contains("5.1") || layout.contains("7.1") || layout.contains('6') {
            parts.push("pan=mono|c0=FC".into());
        } else {
            // 无中置声道可提取，退化为标准单声道下混
            parts.push("aformat=channel_layouts=mono".into());
        }
    }

    parts.join(",")
}

fn audio_args(req: &EncodeRequest, out: &str) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-map".into(),
        format!("0:a:{}", req.audio_ordinal.max(0)),
        "-vn".into(),
        "-sn".into(),
        "-dn".into(),
    ];

    let chain = audio_filter_chain(req);
    if !chain.is_empty() {
        a.push("-af".into());
        a.push(chain);
    }

    if req.audio_mode == "stereo" {
        a.push("-ac".into());
        a.push("2".into());
    }

    match req.audio_codec.as_str() {
        "mp3" => {
            a.push("-c:a".into());
            a.push("libmp3lame".into());
            a.push("-b:a".into());
            a.push(req.audio_bitrate.clone());
            a.push("-ar".into());
            a.push("44100".into());
        }
        "wav" => {
            a.push("-c:a".into());
            a.push("pcm_s16le".into());
        }
        _ => {
            a.push("-c:a".into());
            a.push("flac".into());
            a.push("-compression_level".into());
            a.push("8".into());
        }
    }

    a.push("-y".into());
    a.push(out.to_string());
    a
}

fn video_preview_args(req: &EncodeRequest, out: &str) -> Vec<String> {
    vec![
        "-map".into(),
        "0:v:0".into(),
        "-vf".into(),
        format!("scale=-2:{}", req.preview_height.max(144)),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        req.preview_crf.max(1).to_string(),
        "-g".into(),
        "24".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-y".into(),
        out.to_string(),
    ]
}

fn video_high_args(req: &EncodeRequest, out: &str) -> Vec<String> {
    vec![
        "-map".into(),
        "0:v:0".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "medium".into(),
        "-crf".into(),
        req.high_crf.max(1).to_string(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-y".into(),
        out.to_string(),
    ]
}

fn remux_args(out: &str) -> Vec<String> {
    vec![
        "-map".into(),
        "0".into(),
        "-c".into(),
        "copy".into(),
        "-map_metadata".into(),
        "0".into(),
        "-y".into(),
        out.to_string(),
    ]
}

/// 组装一次完整任务：音频 → 预览 → 高质量 → remux
pub fn plan(req: &EncodeRequest) -> Vec<(String, Vec<String>)> {
    let dir = Path::new(&req.out_dir);
    let base = if req.base_name.trim().is_empty() {
        "output".to_string()
    } else {
        req.base_name.clone()
    };
    let mut steps: Vec<(String, Vec<String>)> = Vec::new();

    if req.want_audio {
        let ext = match req.audio_codec.as_str() {
            "mp3" => "mp3",
            "wav" => "wav",
            _ => "flac",
        };
        let out = dir.join(format!("{base}.audio.{ext}"));
        steps.push((
            format!("音频（{} / {}）", req.audio_codec.to_uppercase(), req.audio_mode),
            audio_args(req, &out.to_string_lossy()),
        ));
    }

    if req.want_preview {
        let out = dir.join(format!("{base}.preview.{}p.mp4", req.preview_height));
        steps.push((
            format!("视频预览（{}p / CRF {}）", req.preview_height, req.preview_crf),
            video_preview_args(req, &out.to_string_lossy()),
        ));
    }

    if req.want_high {
        let out = dir.join(format!("{base}.high.mp4"));
        steps.push((
            format!("视频高质量（原分辨率 / CRF {}）", req.high_crf),
            video_high_args(req, &out.to_string_lossy()),
        ));
    }

    if req.want_remux {
        let out = dir.join(format!("{base}.remux.mkv"));
        steps.push(("原样封装（无损 / -c copy）".into(), remux_args(&out.to_string_lossy())));
    }

    steps
}

/// 执行一个 ffmpeg 步骤，边跑边回调进度
fn run_step(
    ffmpeg: &Path,
    args: &[String],
    total_duration: f64,
    stage_idx: usize,
    stage_count: usize,
    on_progress: &mut dyn FnMut(f32, String),
) -> Result<(), String> {
    let mut cmd = tools::hidden_command(&ffmpeg.to_string_lossy());
    cmd.arg("-hide_banner")
        .arg("-nostdin")
        .arg("-loglevel")
        .arg("error")
        .arg("-progress")
        .arg("pipe:1")
        .arg("-nostats")
        .args(args);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取 ffmpeg 输出")?;
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        if CANCEL.load(Ordering::SeqCst) {
            let _ = child.kill();
            return Err("已取消".into());
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if let Some(v) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = v.trim().parse::<f64>() {
                let secs = us / 1_000_000.0;
                let stage_frac = if total_duration > 0.0 {
                    (secs / total_duration).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let overall =
                    (stage_idx as f32 + stage_frac) / (stage_count.max(1) as f32) * 100.0;
                on_progress(overall, format!("{:.1}s / {:.1}s", secs, total_duration));
            }
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("ffmpeg 退出码 {:?}", status.code()));
    }
    Ok(())
}

pub struct EncodeOutcome {
    pub outputs: Vec<String>,
}

/// 依次执行全部步骤
pub fn execute(
    req: &EncodeRequest,
    ffmpeg: &Path,
    on_progress: &mut dyn FnMut(f32, String),
) -> Result<EncodeOutcome, String> {
    reset_cancel();
    let steps = plan(req);
    if steps.is_empty() {
        return Err("没有选择任何输出项".into());
    }
    std::fs::create_dir_all(&req.out_dir).map_err(|e| format!("无法创建输出目录: {e}"))?;

    let count = steps.len();
    let mut outputs: Vec<String> = Vec::new();

    for (idx, (label, args)) in steps.iter().enumerate() {
        on_progress(
            (idx as f32 / count as f32) * 100.0,
            format!("{label} …"),
        );
        run_step(ffmpeg, args, req.duration, idx, count, on_progress)
            .map_err(|e| format!("{label} 失败: {e}"))?;
        // 记录产物路径（args 最后一项即输出文件）
        if let Some(last) = args.last() {
            outputs.push(last.clone());
        }
    }

    on_progress(100.0, "全部完成".into());
    Ok(EncodeOutcome { outputs })
}

/// 供 UI 展示"将要执行的命令"
pub fn describe_plan(req: &EncodeRequest, ffmpeg: &Path) -> Vec<String> {
    plan(req)
        .into_iter()
        .map(|(label, args)| {
            let quoted: Vec<String> = args
                .iter()
                .map(|a| {
                    if a.contains(' ') {
                        format!("\"{a}\"")
                    } else {
                        a.clone()
                    }
                })
                .collect();
            format!(
                "{label}\n{} {} -i \"{}\" {}",
                ffmpeg.to_string_lossy(),
                "-hide_banner -nostdin -progress pipe:1 -nostats",
                req.video,
                quoted.join(" ")
            )
        })
        .collect()
}
