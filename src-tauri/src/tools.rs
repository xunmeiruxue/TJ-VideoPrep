//! 工具发现与 portable 设置读写。
//!
//! 设计要点（对应"干净不残留"要求）：
//! - 所有工具解析顺序：用户显式设置 → 程序目录 `vendor/` → PATH → 已知安装位置
//! - 所有配置写在程序目录下的 `TJ-VideoPrep.settings.json`，绝不写注册表或 AppData

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Settings {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub mpv_path: String,
    pub out_dir: String,
    pub last_video: String,
    pub preview_height: u32,
    pub preview_crf: u32,
    pub high_crf: u32,
    pub audio_codec: String,
    pub audio_mode: String,
    pub audio_bitrate: String,
}

impl Settings {
    pub fn with_defaults(mut self) -> Self {
        if self.preview_height == 0 {
            self.preview_height = 720;
        }
        if self.preview_crf == 0 {
            self.preview_crf = 30;
        }
        if self.high_crf == 0 {
            self.high_crf = 18;
        }
        if self.audio_codec.is_empty() {
            self.audio_codec = "flac".into();
        }
        if self.audio_mode.is_empty() {
            self.audio_mode = "stereo".into();
        }
        if self.audio_bitrate.is_empty() {
            self.audio_bitrate = "320k".into();
        }
        self
    }
}

/// 程序所在目录（portable 根）。开发模式下是 target/debug 之类，同样满足"只写程序目录"。
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn settings_path() -> PathBuf {
    app_dir().join("TJ-VideoPrep.settings.json")
}

pub fn load_settings() -> Settings {
    let p = settings_path();
    if let Ok(text) = std::fs::read_to_string(&p) {
        if let Ok(s) = serde_json::from_str::<Settings>(&text) {
            return s.with_defaults();
        }
    }
    Settings::default().with_defaults()
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(), text).map_err(|e| e.to_string())
}

/// 隐藏窗口地执行一个命令，返回标准输出。失败时返回 Err(说明)。
pub fn run_capture(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let out = cmd.output().map_err(|e| format!("无法启动 {program}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{program} 退出码 {:?}: {}", out.status.code(), err.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 构造一个隐藏窗口的 Command（供编码器与 mpv 复用）。
pub fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

fn exists_executable(p: &Path) -> bool {
    p.is_file()
}

/// 在 PATH 中查找可执行文件
fn which(stem: &str) -> Option<PathBuf> {
    let name = exe_name(stem);
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(&name);
        if exists_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| exists_executable(p)).cloned()
}

/// ffmpeg / ffprobe 解析
pub fn resolve_ffmpeg(stem: &str, explicit: &str) -> Option<PathBuf> {
    if !explicit.trim().is_empty() {
        let p = PathBuf::from(explicit.trim());
        if exists_executable(&p) {
            return Some(p);
        }
    }

    let dir = app_dir();
    let vendor = dir.join("vendor").join("ffmpeg").join(exe_name(stem));
    let portable = dir.join(exe_name(stem));
    if let Some(p) = first_existing(&[vendor, portable]) {
        return Some(p);
    }

    if let Some(p) = which(stem) {
        return Some(p);
    }

    // 常见安装位置（不遍历全盘，只查已知目录）
    let mut known: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        known.push(PathBuf::from(&local).join("Microsoft").join("WinGet").join("Links").join(exe_name(stem)));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        known.push(PathBuf::from(&pf).join("ffmpeg").join("bin").join(exe_name(stem)));
    }
    first_existing(&known)
}

/// mpv 解析（用户设置 → 程序目录 vendor → PATH → 已知位置）
pub fn resolve_mpv(explicit: &str) -> Option<PathBuf> {
    if !explicit.trim().is_empty() {
        let p = PathBuf::from(explicit.trim());
        if exists_executable(&p) {
            return Some(p);
        }
    }

    let dir = app_dir();
    let name = exe_name("mpv");
    if let Some(p) = first_existing(&[
        dir.join("vendor").join("mpv").join(&name),
        dir.join("vendor").join("mpv").join("mpv").join(&name),
        dir.join(&name),
    ]) {
        return Some(p);
    }

    if let Some(p) = which("mpv") {
        return Some(p);
    }

    let mut known: Vec<PathBuf> = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        known.push(PathBuf::from(&pf).join("mpv").join(&name));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        known.push(PathBuf::from(&local).join("Programs").join("mpv").join(&name));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        known.push(PathBuf::from(&pf).join("mpv.net").join(&name));
    }
    first_existing(&known)
}

/// 探测 mpv 版本（取首行）
pub fn mpv_version(mpv: &Path) -> Option<String> {
    let out = run_capture(&mpv.to_string_lossy(), &["--version"]).ok()?;
    out.lines().next().map(|s| s.trim().to_string())
}

/// 列出可以候选的 mpv 路径（供 UI 展示"探测到的候选"）
pub fn mpv_candidates() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let dir = app_dir();
    let name = exe_name("mpv");
    for p in [
        dir.join("vendor").join("mpv").join(&name),
        dir.join("vendor").join("mpv").join("mpv").join(&name),
    ] {
        if p.is_file() {
            v.push(p.to_string_lossy().to_string());
        }
    }
    if let Some(p) = which("mpv") {
        v.push(p.to_string_lossy().to_string());
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        for extra in ["mpv", "mpv.net"] {
            let p = PathBuf::from(&pf).join(extra).join(&name);
            if p.is_file() {
                v.push(p.to_string_lossy().to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}
