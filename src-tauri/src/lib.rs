//! TJ-VideoPrep —— Tauri 命令层。
//!
//! 所有耗时操作都在 Rust 侧用阻塞线程执行，通过 `job-event` 事件回传进度；
//! 拖放的路径通过 `files-dropped` 事件回传。
//! 前端不使用任何 npm 包（依赖 `withGlobalTauri` 暴露的 `window.__TAURI__`）。

mod encode;
mod mpvdl;
mod probe;
mod tools;

use encode::{EncodeRequest, JobEvent};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "ts", "m2ts", "mts", "webm", "wmv", "flv", "vob", "mpg",
    "mpeg", "rmvb", "3gp", "ogv", "f4v", "asf",
];

#[derive(Serialize)]
struct ToolsStatus {
    app_dir: String,
    version: String,
    ffmpeg: Option<String>,
    ffprobe: Option<String>,
    mpv: Option<String>,
    mpv_version: Option<String>,
    mpv_candidates: Vec<String>,
    output_root: String,
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
        output_root: tools::default_output_root().to_string_lossy().to_string(),
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

#[tauri::command]
fn default_output_root() -> String {
    tools::default_output_root().to_string_lossy().to_string()
}

/// 选单个文件：kind = video | mpv | ffmpeg
#[tauri::command]
async fn pick_file(kind: String) -> Option<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let dialog = rfd::FileDialog::new();
        let dialog = match kind.as_str() {
            "video" => dialog.add_filter("媒体文件", VIDEO_EXTS),
            _ => dialog.add_filter("可执行文件", &["exe"]),
        };
        dialog.pick_file().map(|p| p.to_string_lossy().to_string())
    });
    handle.await.ok().flatten()
}

/// 多选视频文件
#[tauri::command]
async fn pick_video_files() -> Vec<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter("媒体文件", VIDEO_EXTS)
            .pick_files()
            .map(|v| {
                v.into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default()
    });
    handle.await.unwrap_or_default()
}

/// 选文件夹并递归收集其中的媒体文件
#[tauri::command]
async fn pick_video_folder() -> Vec<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        match rfd::FileDialog::new().pick_folder() {
            Some(d) => tools::expand_paths(&[d.to_string_lossy().to_string()]),
            None => Vec::new(),
        }
    });
    handle.await.unwrap_or_default()
}

/// 展开一批路径（拖放或粘贴进来的可能是文件，也可能是目录）
#[tauri::command]
async fn expand_paths(paths: Vec<String>) -> Vec<String> {
    let handle = tauri::async_runtime::spawn_blocking(move || tools::expand_paths(&paths));
    handle.await.unwrap_or_default()
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
        .ok_or_else(|| "未找到 ffprobe，请在运行环境中指定".to_string())?;

    let handle = tauri::async_runtime::spawn_blocking(move || probe::probe(&path, &ffprobe));
    handle
        .await
        .map_err(|e| format!("分析任务异常: {e}"))?
}

/// 下载 mpv 到程序目录的 vendor/mpv
#[tauri::command]
async fn download_mpv(app: AppHandle) -> Result<String, String> {
    let handle = tauri::async_runtime::spawn_blocking(move || {
        let dir = tools::app_dir();
        let emitter = app;
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

/// 批量执行分离/编码
#[tauri::command]
async fn start_encode(app: AppHandle, reqs: Vec<EncodeRequest>) -> Result<Vec<String>, String> {
    let settings = tools::load_settings();
    let ffmpeg = tools::resolve_ffmpeg("ffmpeg", &settings.ffmpeg_path)
        .ok_or_else(|| "未找到 ffmpeg，请在运行环境中指定".to_string())?;

    // 闭包拿走一份句柄，外层保留 app 用于在任务结束后发送 done / error 事件
    let worker_app = app.clone();
    let count = reqs.len();

    let handle = tauri::async_runtime::spawn_blocking(move || {
        let emitter = worker_app;
        let mut emit = |pct: f32, msg: String| {
            let _ = emitter.emit(
                "job-event",
                JobEvent {
                    job_id: "encode".into(),
                    stage: "encode".into(),
                    percent: pct,
                    message: msg,
                    done: false,
                    error: false,
                    outputs: Vec::new(),
                },
            );
        };
        encode::execute_batch(&reqs, &ffmpeg, &mut emit)
    });

    let outcome = handle.await.map_err(|e| format!("编码任务异常: {e}"))?;
    match outcome {
        Ok(outputs) => {
            let _ = app.emit(
                "job-event",
                JobEvent {
                    job_id: "encode".into(),
                    stage: "done".into(),
                    percent: 100.0,
                    message: format!("全部完成：{} 个文件，{} 项输出", count, outputs.len()),
                    done: true,
                    error: false,
                    outputs: outputs.clone(),
                },
            );
            Ok(outputs)
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
    /// 音频轨序号（0 起）
    audio_ordinal: i64,
    start_seconds: f64,
}

/// 用 mpv 打开指定音轨做试听（不产生任何中间文件）
///
/// 注意：mpv 的 `--aid` 是「音频轨序号」（从 1 开始），与 ffmpeg 的 stream index 不同。
/// 实测：一个 audio=index0 / video=index1 的文件，mpv 显示 `--aid=1`。
/// 传错会静音播放（该轨不存在），所以这里用 ordinal + 1。
#[tauri::command]
async fn open_in_mpv(req: PlayRequest) -> Result<(), String> {
    let settings = tools::load_settings();
    let mpv = tools::resolve_mpv(&settings.mpv_path)
        .ok_or_else(|| "未找到 mpv，请先下载或指定路径".to_string())?;

    let handle = tauri::async_runtime::spawn_blocking(move || {
        let aid = req.audio_ordinal.max(0) + 1;
        let mut cmd = tools::hidden_command(&mpv.to_string_lossy());
        cmd.arg(&req.video)
            .arg(format!("--aid={aid}"))
            .arg(format!("--start={:.3}", req.start_seconds.max(0.0)))
            .arg("--force-window=yes")
            .arg("--keep-open=yes")
            .arg("--osc=yes")
            .arg(format!(
                "--title=TJ-VideoPrep 试听 - 音轨 {}",
                req.audio_ordinal + 1
            ));
        cmd.spawn().map_err(|e| format!("启动 mpv 失败: {e}"))?;
        Ok::<(), String>(())
    });

    handle.await.map_err(|e| format!("mpv 任务异常: {e}"))?
}

/// 在资源管理器中打开路径（文件则打开其所在目录）
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
        .on_window_event(|window, event| {
            // 拖放：Tauri 会拦截 HTML5 的文件拖放，路径只能在这里拿到
            if let tauri::WindowEvent::DragDrop(drag) = event {
                match drag {
                    tauri::DragDropEvent::Enter { .. } => {
                        let _ = window.emit("drag-enter", ());
                    }
                    tauri::DragDropEvent::Leave => {
                        let _ = window.emit("drag-leave", ());
                    }
                    tauri::DragDropEvent::Drop { paths, .. } => {
                        let list: Vec<String> = paths
                            .iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect();
                        let _ = window.emit("drag-leave", ());
                        let _ = window.emit("files-dropped", list);
                    }
                    _ => {}
                }
            }
        })
        .setup(|app| {
            // 主窗口就绪后把窗口句柄留着备用（当前仅用于事件发送，无需额外处理）
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_status,
            get_settings,
            set_settings,
            default_output_root,
            pick_file,
            pick_video_files,
            pick_video_folder,
            expand_paths,
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
