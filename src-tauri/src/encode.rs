//! 编码/分离执行层：构造 ffmpeg 命令、跑进度、支持取消与批量。
//!
//! 输出设计：
//! - 音频：FLAC（无损）或 mp3 / wav；声道三档 native / stereo / center
//! - 视频预览版：H.264 mp4，可调分辨率，短 GOP，无音轨，faststart
//! - 视频高质量版：H.264 mp4，原分辨率，CRF 可控，无音轨
//! - 原样 remux：`-c copy`
//!
//! 输出目录三种模式（out_mode）：app = 程序目录下 TJ-Output；source = 源文件所在目录下的
//! TJ-Output；custom = 用户指定目录。

use crate::tools;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
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
    /// 音频轨序号（0 起），同时用于 ffmpeg `-map 0:a:N` 与 mpv `--aid=N+1`
    pub audio_ordinal: i64,
    pub channel_layout: String,
    pub base_name: String,
    pub duration: f64,

    /// 输出位置：app | source | custom
    pub out_mode: String,
    pub custom_dir: String,

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
            base_name: String::new(),
            duration: 0.0,
            out_mode: "app".into(),
            custom_dir: String::new(),
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

/// 解析实际输出目录
pub fn resolve_out_dir(req: &EncodeRequest) -> PathBuf {
    match req.out_mode.as_str() {
        "source" => Path::new(&req.video)
            .parent()
            .map(|d| d.join(tools::OUTPUT_FOLDER))
            .unwrap_or_else(tools::default_output_root),
        "custom" => {
            let d = req.custom_dir.trim();
            if d.is_empty() {
                tools::default_output_root()
            } else {
                PathBuf::from(d)
            }
        }
        _ => tools::default_output_root(),
    }
}

/// 声音滤镜链：处理时间轴延迟 + 中置声道提取
fn audio_filter_chain(req: &EncodeRequest) -> String {
    let mut parts: Vec<String> = Vec::new();

    if req.apply_align {
        if req.delay_ms > 0 {
            parts.push(format!("adelay={}:all=1", req.delay_ms));
        } else if req.delay_ms < 0 {
            let start = (-req.delay_ms) as f64 / 1000.0;
            parts.push(format!("atrim=start={start:.6},asetpts=PTS-STARTPTS"));
        }
    }

    if req.audio_mode == "center" {
        let layout = req.channel_layout.to_lowercase();
        if layout.contains("5.1") || layout.contains("7.1") || layout.contains('6') {
            parts.push("pan=mono|c0=FC".into());
        } else {
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

fn base_name(req: &EncodeRequest) -> String {
    if !req.base_name.trim().is_empty() {
        return req.base_name.trim().to_string();
    }
    Path::new(&req.video)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string())
}

/// 所有步骤共用的全局参数（必须出现在 `-i` 之前）
fn global_args() -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "error".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
    ]
}

/// 把输入文件放到参数最前面：ffmpeg 要求 `-i` 出现在所有输出选项之前。
///
/// 这里与 `describe_plan` 必须使用同一份参数。曾经 `-i` 只写在预览的模板里、
/// 没进真正的参数表，结果是"预览正确、执行缺少输入"，ffmpeg 立即以 EINVAL(-22) 退出。
fn with_input(req: &EncodeRequest, args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 2);
    out.push("-i".to_string());
    out.push(req.video.clone());
    out.extend(args);
    out
}

fn quote_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect()
}

/// 一次任务的步骤列表（含每步的时长，供进度换算）
struct Step {
    label: String,
    args: Vec<String>,
    duration: f64,
}

fn build_steps(req: &EncodeRequest) -> Vec<Step> {
    let dir = resolve_out_dir(req);
    let base = base_name(req);
    let mut steps: Vec<Step> = Vec::new();

    if req.want_audio {
        let ext = match req.audio_codec.as_str() {
            "mp3" => "mp3",
            "wav" => "wav",
            _ => "flac",
        };
        let out = dir.join(format!("{base}.audio.{ext}"));
        steps.push(Step {
            label: format!(
                "音频 {} / {}",
                req.audio_codec.to_uppercase(),
                req.audio_mode
            ),
            args: with_input(req, audio_args(req, &out.to_string_lossy())),
            // 音频转码很快，进度按比例给一个近似值时长为原时长
            duration: req.duration,
        });
    }

    if req.want_preview {
        let out = dir.join(format!("{base}.preview.{}p.mp4", req.preview_height));
        steps.push(Step {
            label: format!("预览 {}p / CRF {}", req.preview_height, req.preview_crf),
            args: with_input(req, video_preview_args(req, &out.to_string_lossy())),
            duration: req.duration,
        });
    }

    if req.want_high {
        let out = dir.join(format!("{base}.high.mp4"));
        steps.push(Step {
            label: format!("高质量 CRF {}", req.high_crf),
            args: with_input(req, video_high_args(req, &out.to_string_lossy())),
            duration: req.duration,
        });
    }

    if req.want_remux {
        let out = dir.join(format!("{base}.remux.mkv"));
        steps.push(Step {
            label: "原样封装".into(),
            args: with_input(req, remux_args(&out.to_string_lossy())),
            duration: req.duration,
        });
    }

    steps
}

/// 供 UI 展示"将要执行的命令"
pub fn describe_plan(req: &EncodeRequest, ffmpeg: &Path) -> Vec<String> {
    let dir = resolve_out_dir(req);
    let head = format!("输出目录：{}", dir.display());
    let mut lines = vec![head];

    for s in build_steps(req) {
        // 与 run_step 用同一份参数，保证"预览所见 = 执行所用"
        let mut full = global_args();
        full.extend(s.args.clone());
        let quoted = quote_args(&full);
        lines.push(format!(
            "【{}】\n{} {}",
            s.label,
            ffmpeg.to_string_lossy(),
            quoted.join(" ")
        ));
    }

    if lines.len() == 1 {
        lines.push("（没有选择任何输出项）".into());
    }
    lines
}

/// 执行一个 ffmpeg 步骤，边跑边回调进度
fn run_step(
    ffmpeg: &Path,
    step: &Step,
    step_index: usize,
    step_total: usize,
    on_progress: &mut dyn FnMut(f32, String),
) -> Result<(), String> {
    let mut cmd = tools::hidden_command(&ffmpeg.to_string_lossy());
    cmd.args(global_args()).args(&step.args);

    // 三个标准流都必须显式处理：
    // - stdin 给 null：本程序是 GUI 进程（没有控制台），子进程继承到的 stdin 句柄可能无效
    // - stderr 必须持续读走：管道写满会让 ffmpeg 阻塞在写日志上，进而卡死或异常退出
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;
    let stdout = child.stdout.take().ok_or("无法读取 ffmpeg 输出")?;

    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut e) = stderr {
            let _ = std::io::Read::read_to_string(&mut e, &mut buf);
        }
        buf
    });

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
                let stage_frac = if step.duration > 0.0 {
                    (secs / step.duration).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let overall =
                    (step_index as f32 + stage_frac) / (step_total.max(1) as f32) * 100.0;
                on_progress(
                    overall,
                    format!("{} · {:.0}s / {:.0}s", step.label, secs, step.duration),
                );
            }
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    let err_text = err_thread.join().unwrap_or_default();

    if !status.success() {
        let detail = tail_lines(&err_text, 12);
        let detail = if detail.trim().is_empty() {
            "(ffmpeg 没有输出错误信息)".to_string()
        } else {
            detail
        };
        return Err(format!("ffmpeg 退出码 {:?}\n{detail}", status.code()));
    }
    Ok(())
}

/// 取文本的最后 n 行（ffmpeg 的报错通常在末尾），去掉空行
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// 批量执行：依次处理每个文件的全部步骤
pub fn execute_batch(
    reqs: &[EncodeRequest],
    ffmpeg: &Path,
    on_progress: &mut dyn FnMut(f32, String),
) -> Result<Vec<String>, String> {
    reset_cancel();

    if reqs.is_empty() {
        return Err("没有待处理的文件".into());
    }

    // 先把所有文件的所有步骤摊平，便于算总进度
    let mut flat: Vec<Step> = Vec::new();
    for req in reqs {
        if req.video.trim().is_empty() {
            continue;
        }
        let dir = resolve_out_dir(req);
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建输出目录 {}: {e}", dir.display()))?;

        let name = Path::new(&req.video)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        for mut s in build_steps(req) {
            s.label = format!("{name} · {}", s.label);
            flat.push(s);
        }
    }

    if flat.is_empty() {
        return Err("没有选择任何输出项".into());
    }

    let total = flat.len();
    let mut outputs: Vec<String> = Vec::new();

    for (idx, step) in flat.iter().enumerate() {
        on_progress(idx as f32 / total as f32 * 100.0, format!("{} …", step.label));
        run_step(ffmpeg, step, idx, total, on_progress)
            .map_err(|e| format!("{} 失败: {e}", step.label))?;
        if let Some(last) = step.args.last() {
            outputs.push(last.clone());
        }
    }

    on_progress(100.0, format!("全部完成（{} 项输出）", outputs.len()));
    Ok(outputs)
}
