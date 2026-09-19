//! TJ-VideoPrep —— Tauri 命令层。
//!
//! 所有耗时操作都在 Rust 侧用阻塞线程执行，通过 `job-event` 事件回传进度。
//! 前端不使用任何 npm 包（依赖 `withGlobalTauri` 暴露的 `window.__TAURI__`）。

mod encode;
mod mpvdl;
mod probe;
mod tools;

use encode::{EncodeRequest, JobEvent};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
struct ToolsStatus {
    app_dir: String,
    version: String,
    ffmpeg: Option<String>,
    ffprobe: Option<String>,
    mpv: Option<String>,
    mpv_version: Option<String>,
    mpv_candidates: Vec<String>,
}

#[tauri::command]
fn app_status() -> ToolsStatus {
    let s = tools::load_settings();
    let ffmpeg = tools::resolve_ffmpeg("ffmpeg", &s.ffmpeg_path);
    let ffprobe = tools::resolve_ffmpeg("ffprobe", &s.ffprobe_path);
    let mpv = tools::resolve_mpv(&s.mpv_path);
    let mpv_ver = mpv.as_ref().and_then(|p| tools::mpv_version(p));

    ToolsStatus {
        app_dir: tools::app_dir().to_string_lossy().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ffmpeg: ffmpeg.map(|p| p.to_string_lossy().to_string()),
        ffprobe: ffprobe.map(|p| p.to_string_lossy().to_string()),
        mpv: mpv.map(|p| p.to_string_lossy().to_string()),
        mpv_version: mpv_ver,
        mpv_candidates: tools::mpv_candidates(),
    }
}

#[tauri::command]
fn get_settings() -> tools::Settings {
    tools::load_settings()
}

#[tauri::command]
fn set_settings(settings: tools::Settings) -> Result<(), String> {
    tools::save_settings(&settings)
}

/// 文件选择：kind = video | mpv | ffmpeg | ffprobe
#[tauri::command]
async fn pick_file(kind: String) -> Option<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new();
        let dialog = match kind.as_str() {
            "video" => dialog.add_filter(
                "媒体文件",
                &[
                    "mkv", "mp4", "m4v", "avi", "mov", "ts", "m2ts", "webm", "wmv", "flv", "vob",
                    "mpg", "mpeg", "rmvb", "3gp", "ogv",
                ],
            ),
            _ => dialog.add_filter("可执行文件", &["exe"]),
        };
        dialog.pick_file().map(|p| p.to_string_lossy().to_string())
    });

    handle.await.ok().flatten()
}

#[tauri::command]
async fn pick_dir() -> Option<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    });
    handle.await.ok().flatten()
}

#[tauri::command]
async fn probe_video(path: String) -> Result<probe::ProbeResult, String> {
    let settings = tools::load_settings();
    let ffprobe = tools::resolve_ffmpeg("ffprobe", &settings.ffprobe_path)
        .ok_or_else(|| "未找到 ffprobe，请在设置中指定".to_string())?;

    let handle =
        tauri::async_runtime::spawn_blocking(move || probe::probe(&path, &ffprobe));
    handle
        .await
        .map_err(|e| format!("分析任务异常: {e}"))?
}

/// 下载 mpv 到程序目录的 vendor/mpv
#[tauri::command]
async fn download_mpv(app: AppHandle) -> Result<String, String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools::app_dir();
        let emitter = app.clone();
        let mut emit = |pct: f32, msg: String| {
            let _ = emitter.emit(
                "job-event",
                JobEvent {
                    job_id: "mpv".into(),
                    stage: "mpv".into(),
                    percent: pct,
                    message: msg,
                    done: false,
                    error: false,
                    outputs: Vec::new(),
                },
            );
        };
        mpvdl::download_and_extract(&dir, &mut emit)
    });

    let result = handle.await.map_err(|e| format!("下载任务异常: {e}"))?;
    match result {
        Ok(path) => {
            let s = path.to_string_lossy().to_string();
            let mut settings = tools::load_settings();
            settings.mpv_path = s.clone();
            let _ = tools::save_settings(&settings);
            Ok(s)
        }
        Err(e) => Err(e),
    }
}

/// 预览将要执行的 ffmpeg 命令
#[tauri::command]
fn describe_plan(req: EncodeRequest) -> Result<Vec<String>, String> {
    let settings = tools::load_settings();
    let ffmpeg = tools::resolve_ffmpeg("ffmpeg", &settings.ffmpeg_path)
        .unwrap_or_else(|| std::path::PathBuf::from("ffmpeg"));
    Ok(encode::describe_plan(&req, &ffmpeg))
}

/// 执行分离/编码
#[tauri::command]
async fn start_encode(app: AppHandle, req: EncodeRequest) -> Result<Vec<String>, String> {
    let settings = tools::load_settings();
    let ffmpeg = tools::resolve_ffmpeg("ffmpeg", &settings.ffmpeg_path)
        .ok_or_else(|| "未找到 ffmpeg，请在设置中指定".to_string())?;

    let job_id = format!("job-{}", std::process::id());
    // 闭包拿走一份句柄，外层保留 app 用于在任务结束后发送 done / error 事件。
    // 之前闭包直接 move 了 app，外层再 app.emit 就触发了 E0382。
    let worker_app = app.clone();
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let emitter = worker_app.clone();
        let jid = job_id.clone();
        let mut emit = |pct: f32, msg: String| {
            let _ = emitter.emit(
                "job-event",
                JobEvent {
                    job_id: jid.clone(),
                    stage: "encode".into(),
                    percent: pct,
                    message: msg,
                    done: false,
                    error: false,
                    outputs: Vec::new(),
                },
            );
        };
        encode::execute(&req, &ffmpeg, &mut emit)
    });

    let outcome = handle.await.map_err(|e| format!("编码任务异常: {e}"))?;
    match outcome {
        Ok(o) => {
            let _ = app.emit(
                "job-event",
                JobEvent {
                    job_id: "encode".into(),
                    stage: "done".into(),
                    percent: 100.0,
                    message: "全部输出完成".into(),
                    done: true,
                    error: false,
                    outputs: o.outputs.clone(),
                },
            );
            Ok(o.outputs)
        }
        Err(e) => {
            let _ = app.emit(
                "job-event",
                JobEvent {
                    job_id: "encode".into(),
                    stage: "error".into(),
                    percent: 0.0,
                    message: e.clone(),
                    done: true,
                    error: true,
                    outputs: Vec::new(),
                },
            );
            Err(e)
        }
    }
}

#[tauri::command]
fn cancel_encode() {
    encode::request_cancel();
}

#[derive(Deserialize)]
struct PlayRequest {
    video: String,
    stream_index: i64,
    start_seconds: f64,
}

/// 用 mpv 打开指定音轨做试听（不产生任何中间文件）
#[tauri::command]
async fn open_in_mpv(req: PlayRequest) -> Result<(), String> {
    let settings = tools::load_settings();
    let mpv = tools::resolve_mpv(&settings.mpv_path)
        .ok_or_else(|| "未找到 mpv，请先下载或指定路径".to_string())?;

    let handle = tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = tools::hidden_command(&mpv.to_string_lossy());
        cmd.arg(&req.video)
            .arg(format!("--aid={}", req.stream_index + 1))
            .arg(format!("--start={:.3}", req.start_seconds.max(0.0)))
            .arg("--force-window=yes")
            .arg("--keep-open=yes")
            .arg("--osc=yes")
            .arg(format!("--title=TJ-VideoPrep 试听 - 音轨 {}", req.stream_index));
        cmd.spawn().map_err(|e| format!("启动 mpv 失败: {e}"))?;
        Ok::<(), String>(())
    });

    handle.await.map_err(|e| format!("mpv 任务异常: {e}"))?
}

/// 在资源管理器中打开文件所在目录
#[tauri::command]
fn open_in_explorer(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    let target = if p.is_dir() {
        p
    } else {
        p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
    };

    #[cfg(windows)]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(not(windows))]
    let mut cmd = std::process::Command::new("xdg-open");

    cmd.arg(target).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// 打开外部链接（例如 mpv 下载页）
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(windows)]
    let mut cmd = std::process::Command::new("cmd");
    #[cfg(windows)]
    cmd.args(["/C", "start", "", &url]);

    #[cfg(not(windows))]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(not(windows))]
    cmd.arg(&url);

    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn mpv_download_page() -> String {
    mpvdl::DOWNLOAD_PAGE.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_status,
            get_settings,
            set_settings,
            pick_file,
            pick_dir,
            probe_video,
            download_mpv,
            describe_plan,
            start_encode,
            cancel_encode,
            open_in_mpv,
            open_in_explorer,
            open_url,
            mpv_download_page,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
