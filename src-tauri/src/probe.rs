//! ffprobe 音轨分析。
//!
//! 目标：给出每条音轨的可选信息 + 推荐标记，并算出音轨相对视频的起始偏移（时间轴对齐的依据）。

use crate::tools;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone, Default)]
pub struct AudioTrack {
    /// ffprobe 的全局 stream index（用于精确定位）
    pub stream_index: i64,
    /// 音频流内部的序号（0 起），对应 ffmpeg `-map 0:a:N`
    pub ordinal: i64,
    pub codec: String,
    pub language: String,
    pub title: String,
    pub channels: i64,
    pub channel_layout: String,
    pub sample_rate: String,
    pub bit_rate: String,
    pub start_time: f64,
    pub is_default: bool,
    /// 推荐等级：primary / alternate / commentary / unknown
    pub recommendation: String,
    pub note: String,
}

#[derive(Serialize, Clone, Default)]
pub struct VideoInfo {
    pub stream_index: i64,
    pub codec: String,
    pub width: i64,
    pub height: i64,
    pub r_frame_rate: String,
    pub start_time: f64,
    pub pix_fmt: String,
    pub is_hdr: bool,
}

#[derive(Serialize, Clone, Default)]
pub struct ProbeResult {
    pub file_name: String,
    pub file_path: String,
    pub format_name: String,
    pub format_long_name: String,
    pub duration: f64,
    pub size_bytes: u64,
    pub video: VideoInfo,
    pub audios: Vec<AudioTrack>,
    /// 第一条音频相对视频轨的起始差（毫秒）。>0 表示音频晚于视频。
    pub delay_ms: i64,
    pub align_note: String,
    pub raw_summary: String,
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key)
        .and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
        .unwrap_or(0.0)
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(0)
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn tag(v: &Value, key: &str) -> String {
    v.get("tags")
        .and_then(|t| t.get(key))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 依据容器/编码/声道数给出推荐标记
fn recommend(track: &AudioTrack, total_audios: usize) -> (String, String) {
    let title_l = track.title.to_lowercase();
    let lang_l = track.language.to_lowercase();

    let looks_commentary = title_l.contains("commentary")
        || title_l.contains("comment")
        || title_l.contains("评论")
        || title_l.contains("解説")
        || lang_l.contains("comment");

    if looks_commentary {
        return ("commentary".into(), "疑似评论音轨".into());
    }

    if total_audios <= 1 {
        return ("primary".into(), "唯一音轨".into());
    }

    if track.is_default {
        return ("primary".into(), "容器默认音轨".into());
    }

    ("alternate".into(), "备选音轨".into())
}

fn channel_layout_name(v: &Value) -> String {
    let l = s(v, "channel_layout");
    if !l.is_empty() {
        return l;
    }
    match i(v, "channels") {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        _ => String::new(),
    }
}

fn is_hdr(v: &Value) -> bool {
    let side = v
        .get("side_data_list")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let has_mastering = side.iter().any(|d| {
        s(d, "side_data_type")
            .to_lowercase()
            .contains("mastering display")
    });
    let pix = s(v, "pix_fmt").to_lowercase();
    has_mastering || pix.contains("10") || pix.contains("12")
}

pub fn probe(video: &str, ffprobe: &std::path::Path) -> Result<ProbeResult, String> {
    let ffprobe_s = ffprobe.to_string_lossy().to_string();
    let out = tools::run_capture(
        &ffprobe_s,
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            video,
        ],
    )?;

    let root: Value = serde_json::from_str(&out).map_err(|e| format!("ffprobe 输出解析失败: {e}"))?;

    let mut result = ProbeResult {
        file_path: video.to_string(),
        file_name: std::path::Path::new(video)
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .unwrap_or_default(),
        ..Default::default()
    };

    if let Some(fmt) = root.get("format") {
        result.format_name = s(fmt, "format_name");
        result.format_long_name = s(fmt, "format_long_name");
        result.duration = f(fmt, "duration");
        result.size_bytes = fmt.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
    }

    let streams = root
        .get("streams")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut audio_ordinal = 0i64;
    let mut raw_lines: Vec<String> = Vec::new();

    for st in &streams {
        match s(st, "codec_type").as_str() {
            "video" => {
                if result.video.height != 0 {
                    continue; // 跳过附加图片/封面流
                }
                result.video = VideoInfo {
                    stream_index: i(st, "index"),
                    codec: s(st, "codec_name"),
                    width: i(st, "width"),
                    height: i(st, "height"),
                    r_frame_rate: s(st, "r_frame_rate"),
                    start_time: f(st, "start_time"),
                    pix_fmt: s(st, "pix_fmt"),
                    is_hdr: is_hdr(st),
                };
            }
            "audio" => {
                let bit_rate = {
                    let b = i(st, "bit_rate");
                    if b > 0 {
                        format!("{} kbps", b / 1000)
                    } else {
                        String::new()
                    }
                };
                let mut track = AudioTrack {
                    stream_index: i(st, "index"),
                    ordinal: audio_ordinal,
                    codec: s(st, "codec_name"),
                    language: tag(st, "language"),
                    title: tag(st, "title"),
                    channels: i(st, "channels"),
                    channel_layout: channel_layout_name(st),
                    sample_rate: s(st, "sample_rate"),
                    bit_rate,
                    start_time: f(st, "start_time"),
                    is_default: st
                        .get("disposition")
                        .and_then(|d| d.get("default"))
                        .and_then(|d| d.as_i64())
                        .unwrap_or(0)
                        == 1,
                    recommendation: String::new(),
                    note: String::new(),
                };
                raw_lines.push(format!(
                    "#{} a:{} {} {}ch {} {}",
                    track.stream_index, track.ordinal, track.codec, track.channels,
                    track.language, track.title
                ));
                audio_ordinal += 1;
                result.audios.push(track);
            }
            _ => {}
        }
    }

    let total = result.audios.len();
    for t in result.audios.iter_mut() {
        let (rec, note) = recommend(t, total);
        t.recommendation = rec;
        t.note = note;
    }

    // 时间轴对齐：音轨起点相对视频起点的偏移
    if let Some(first) = result.audios.first() {
        let raw = ((first.start_time - result.video.start_time) * 1000.0).round() as i64;
        result.delay_ms = raw;
        result.align_note = if raw.abs() <= 5 {
            "音轨与视频起点一致，无需补偿".into()
        } else if raw > 0 {
            format!("音轨比视频晚 {raw} ms，导出音频时将在头部补静音对齐")
        } else {
            format!("音轨比视频早 {} ms，导出音频时将裁掉开头对齐", -raw)
        };
    } else {
        result.align_note = "未找到音频流".into();
    }

    result.raw_summary = raw_lines.join("\n");
    Ok(result)
}
