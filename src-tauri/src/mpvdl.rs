//! mpv 获取：自动下载或指定本地副本。
//!
//! 下载源：zhongfly/mpv-winbuild 的 GitHub Release（x86_64 通用构建，.7z）。
//! 解压到 `<程序目录>/vendor/mpv/`，因此不会在系统其它位置留下任何东西。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const API: &str = "https://api.github.com/repos/zhongfly/mpv-winbuild/releases/latest";
pub const DOWNLOAD_PAGE: &str = "https://github.com/zhongfly/mpv-winbuild/releases/latest";
const UA: &str = "TJ-VideoPrep";

/// 在最新发布中挑选 x86_64 通用构建（排除 debug / dev / v3 / aarch64）
pub fn latest_mpv_asset() -> Result<(String, String), String> {
    let resp = ureq::get(API)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| format!("请求 GitHub 失败: {e}"))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 GitHub 响应失败: {e}"))?;

    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    for a in assets {
        let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let url = a
            .get("browser_download_url")
            .and_then(|x| x.as_str())
            .unwrap_or("");

        if !name.starts_with("mpv-") || !name.ends_with(".7z") {
            continue;
        }
        if name.contains("debug") || name.contains("dev-") {
            continue;
        }
        if name.contains("aarch64") {
            continue;
        }
        if name.contains("-v3-") {
            continue; // v3 需要 Haswell 及更新的 CPU
        }
        if !name.contains("x86_64") {
            continue;
        }
        return Ok((name.to_string(), url.to_string()));
    }

    Err("未在最新发布中找到 x86_64 的 mpv 包".into())
}

fn find_file_recursive(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 || !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() {
            if p.file_name()
                .map(|n| n.to_string_lossy().eq_ignore_ascii_case(target))
                .unwrap_or(false)
            {
                return Some(p);
            }
        } else if p.is_dir() {
            subdirs.push(p);
        }
    }
    for d in subdirs {
        if let Some(found) = find_file_recursive(&d, target, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// 下载并解压 mpv，返回 mpv.exe 的完整路径
pub fn download_and_extract(
    app_dir: &Path,
    on_progress: &mut dyn FnMut(f32, String),
) -> Result<PathBuf, String> {
    let (name, url) = latest_mpv_asset()?;

    let tmp_dir = app_dir.join("runtime");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("无法创建 runtime 目录: {e}"))?;
    let archive = tmp_dir.join(&name);

    on_progress(0.0, format!("正在下载 {name}"));

    let resp = ureq::get(&url)
        .set("User-Agent", UA)
        .call()
        .map_err(|e| format!("下载失败: {e}"))?;

    let total: u64 = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&archive).map_err(|e| format!("无法写入临时文件: {e}"))?;
    let mut buf = vec![0u8; 128 * 1024];
    let mut got: u64 = 0;

    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("下载中断: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        let pct = if total > 0 {
            (got as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        on_progress(
            pct,
            format!(
                "下载中 {:.1} / {:.1} MB",
                got as f32 / 1_048_576.0,
                total as f32 / 1_048_576.0
            ),
        );
    }
    drop(file);

    let dest = app_dir.join("vendor").join("mpv");
    std::fs::create_dir_all(&dest).map_err(|e| format!("无法创建 vendor/mpv: {e}"))?;

    on_progress(100.0, "正在解压".into());
    sevenz_rust::decompress_file(&archive, &dest).map_err(|e| format!("解压失败: {e}"))?;
    let _ = std::fs::remove_file(&archive);

    let exe_name = if cfg!(windows) { "mpv.exe" } else { "mpv" };
    let found = find_file_recursive(&dest, exe_name, 0)
        .ok_or_else(|| "解压完成但未找到 mpv 可执行文件".to_string())?;

    on_progress(100.0, format!("mpv 已就绪: {}", found.display()));
    Ok(found)
}
