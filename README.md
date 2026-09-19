# TJ-VideoPrep

给电影做字幕打轴时的**音轨选择 + 分离转码**工具。Windows 桌面应用，Tauri v2 + Rust，前端为纯静态页面（无 npm 依赖）。

## 这个工具解决什么

原打轴软件（渡幕-Web）只接受音频文件，给电影打轴时看不到画面、也无法确认该选哪条音轨。本工具负责：

1. 直接读取原视频，列出全部音轨并给出建议（默认轨、语言、评论轨识别等）
2. 用 mpv 试听任意音轨（**直接播放源文件，不产生任何中间文件**）
3. 列出音轨相对视频的起始偏移，导出时自动补偿（保证音频 0 秒 = 视频 0 秒）
4. 选定后一次性输出音频与多档视频

## 输出内容

| 项目 | 默认 | 说明 |
|---|---|---|
| 音频 | FLAC / 2 声道 downmix | 无损，波形与源 PCM 一致；打轴软件的波形由此保证精度 |
| 音频（可选） | 保持源声道 / 中置单声道 | 原生多声道输出、对白最干净的单声道 |
| 视频 · 预览版 | 720p H.264 CRF 30，短 GOP、无音轨、faststart | 供打轴时看画面，拖动响应快 |
| 视频 · 高质量版 | 原分辨率 H.264 CRF 18 | 更完整的输出 |
| 原样封装 | `-c copy` | 无损归档，不重新编码 |

## 干净不残留

- 所有配置写在**程序目录**下的 `TJ-VideoPrep.settings.json`
- 下载的 mpv 解压在**程序目录**的 `vendor/mpv/`
- 下载用的临时压缩包放在**程序目录**的 `runtime/`，解压后即删除
- 不写注册表业务项、不写 `%APPDATA%`、不在系统其它位置留文件
- 安装包为 NSIS（安装时可选目录）；另附免安装 zip

卸载时删除程序目录即完全清除（NSIS 仅保留常规卸载登记项）。

## 依赖

| 组件 | 是否必需 | 获取方式 |
|---|---|---|
| ffmpeg / ffprobe | **必需** | 自动从 PATH 探测；也可在界面中手动指定 |
| mpv | 试听功能需要 | 界面内一键下载（源：`zhongfly/mpv-winbuild`），或指定本地已有的 mpv.exe |

mpv 已内置"下载 / 指定本地路径"两种方式，两个按钮都在顶部"运行环境"栏里。

## 构建

**不在本地构建，全部走 GitHub Actions。**

推送到仓库后：

```
手动触发：Actions → build → Run workflow
打 tag 触发：git tag v0.1.0 && git push origin v0.1.0
```

工作流产出（发布为 draft release）：

- `TJ-VideoPrep_<版本>_x64-setup.exe` —— NSIS 安装包
- `TJ-VideoPrep_<版本>_x64_portable.zip` —— 免安装版

工作流位于 `.github/workflows/build.yml`，使用 `tauri-apps/tauri-action@v1`，运行在 `windows-latest`。

## 目录结构

```
TJ-VideoPrep/
├── .github/workflows/build.yml   构建与发布流水线
├── docs/DESIGN.md                设计依据与决策记录
├── web/                          前端（纯静态，frontendDist 指向此处）
│   ├── index.html
│   ├── styles.css
│   └── app.js
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── icons/
    └── src/
        ├── main.rs     入口
        ├── lib.rs      Tauri 命令层
        ├── tools.rs    工具发现 + portable 设置
        ├── probe.rs    ffprobe 音轨分析
        ├── encode.rs   ffmpeg 输出与进度
        └── mpvdl.rs    mpv 下载与解压
```

## 状态

**首次构建尚未验证。** 代码按 Tauri v2 官方规范编写（参考官方 config 参考与 tauri-action 文档），但没有本地 Rust 工具链可用，因此未经过编译。首次 CI 运行若报错，多为依赖版本或 API 细节，按报错修正即可。

## 与打轴软件的关系

本工具**不修改打轴软件的任何文件**，也不向其目录写入任何内容。它只负责产出音频与视频文件；打轴软件里的画面同步（浏览器侧旁挂面板）是另一项独立交付，见 `docs/DESIGN.md`。
