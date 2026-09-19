# TJ-VideoPrep 计划书（待审查）

目标软件：`E:\TransJimakuWeb-portable`（渡幕-Web v0.11.0，Nuitka 后端 + Vite/React 前端）
场景：给电影做字幕打轴时，需要看着画面、选定正确音轨，并保证音频与视频时间轴严格一致。
状态：**仅计划，未动手**。

---

## 1. 你提出的硬性要求

| # | 要求 | 说明 |
|---|---|---|
| R1 | 软件本体零改动 | 不碰 `backend.exe`、`frontend-dist`、`index.html` 任何文件；软件更新后我的东西继续可用 |
| R2 | 零垃圾文件 | 软件目录不受污染；**选轨试听阶段不产生任何中间文件** |
| R3 | 选定后才分离 | 只有你确认音轨后，才执行分离与编码，产出正式文件 |
| R4 | 时间轴一致性 | 音频 0 秒必须等于视频 0 秒；处理 mkv 音轨延迟、负 start_time、edit list 等 |
| R5 | 波形精度优先 | 你在意波形，正式音频必须让渡幕里的波形尽量准 |
| R6 | 能看画面 | 打轴时画面跟随渡幕的播放头 |
| R7 | 支持原生多声道输出 | 作为可选项保留，不强制 downmix |
| R8 | 闭环校验 | 校准片验证，按容器格式各做一次 |

---

## 2. 已验证的事实（审查基础，均可复核）

### 2.1 渡幕的格式与媒体处理

| 事实 | 证据位置 |
|---|---|
| 接受的音频扩展名共 8 种：`.aac .aiff .flac .m4a .mp3 .ogg .opus .wav` | 前端 bundle 硬编码 |
| 界面明确提示"仅支持音频" | 界面文案 |
| 批量导入按"文件名主干分组 + 音频/字幕/台本"三类建轨，其余判 unsupported | `ir()` 分类函数（只看扩展名正则或 MIME `audio/*`） |
| 媒体可上传（`POST /tracks/{id}/media/upload`），也可按路径关联（`POST /tracks/{id}/media`，body `{file_path}`）——后者面向平台侧来源 | 两个调用函数 |
| 有 fallback 转码机制：前端读 `needs_fallback` / `fallback_ready`，每 2 秒轮询、最多 60 次（120 秒），超时报"媒体转换超时" | `w2()` 函数 |
| 转写为任务式：`POST /tracks/{id}/transcribe` 只提交参数，不上传文件本体 | 接口调用 |
| 转写参数：`enable_diarization`、`tag_audio_events`、`max_duration_s`、`min_duration_s`、`max_chars_per_line`、`gap_ms`、`merge_gap_threshold_s`；**无 `use_multi_channel`** | 参数表 |
| 会把音频复制进自身存储目录（"应用内的音频副本与波形缓存"） | 界面文案 |

### 2.2 渡幕的播放与波形机制

| 事实 | 证据位置 |
|---|---|
| 播放用 **HTMLAudioElement 全局单例**：`new Audio()`，`preload="auto"` | `Pr()` 函数 |
| 音频源为本地后端：`http://127.0.0.1:<port>/api/tracks/{id}/media/audio` | `ti()` 函数 |
| 播放头状态集中在单一 store：`mediaUrl` / `currentTimeMs` / `durationMs` / `isPlaying` / `playbackRate` / `volume` / `loop` / `channelMode` | store 定义 |
| 波形由**后端生成**：`POST /tracks/{id}/waveform` + `/waveform/status` 轮询，前端 canvas 渲染 | 接口 + 文案"正在解码音频…""正在生成波形… N%" |
| 波形数据分左右两路：`waveformDataLeft` / `waveformDataRight` | store 字段 |
| 波形缩放 `zoomPxPerSec` 默认 100 px/s，范围 0.05–2000 | store 定义 |
| 提供"声道切换：左声道 / 混合 / 右声道"；**单声道时该功能不可用**（"单声道音频，声道切换不可用"） | UI 文案 + `is_mono` 标记 |
| 波形的**峰值窗口精度**未知 | 逻辑位于编译后的 `backend.exe`，需实测 |

### 2.3 浏览器侧约束

| 事实 | 证据 |
|---|---|
| 浏览器**不能切换嵌入音轨**：`HTMLMediaElement.audioTracks` 仅 Safari 支持，Chrome/Edge/Firefox/Opera 均不支持 | MDN 兼容性表 |
| MKV 容器浏览器无法直接播放 | 通用约束 |
| Chrome 等浏览器对 audio/video 元素的多声道音频**先做立体声 downmix**，之后才进 Web Audio 图 | Web Audio 规范讨论 + 独立实测 |

### 2.4 云端转写（ElevenLabs）

| 事实 | 证据 |
|---|---|
| 支持格式：AAC / AIFF / OGG / MP3 / OPUS / WAV / FLAC / M4A / WebM；单文件上限 3 GB | 官方文档 |
| 多声道模式：最多 5 声道，语义为"每声道一个独立说话人"，与 diarization **互斥** | 官方文档 |
| **官方未对格式优劣做任何推荐** | 官方文档 |

### 2.5 你的本机环境（已实测）

| 工具 | 版本 / 位置 |
|---|---|
| Node.js | v24.18.0（`E:\Scoop\apps\nodejs-lts`） |
| Python | 3.10.11，**含 tkinter**（`E:\Scoop\apps\python310`） |
| mpv | 0.36.0（`E:\ARCTIME_PRO_4.5_WIN64\tools\MPV\mpv.exe`） |
| ffmpeg | 已在 PATH（Scoop shims），另有软件自带 `bin\ffmpeg.exe` |
| 浏览器 | CatsXP（Chromium 137）、Chrome |

---

## 3. 交付物

| 代号 | 交付物 | 作用 |
|---|---|---|
| A | `TJ-Prep` 桌面小工具 | 列出音轨 + 给建议 + 试听（零文件）；选定后分离编码 + 出对齐报告 |
| B | 画面面板用户脚本（Tampermonkey） | 在渡幕界面里旁挂画面，跟随其播放头 |
| C | 校准片工具 | 闭环校验（每种容器一次） |

---

## 4. 端到端流程

```
① 双击启动 TJ-Prep（背后是 python.exe，不打包 exe）
② 选择原视频文件（不做任何预处理、不产生文件）
③ 工具用 ffprobe 列出全部音轨：索引 / 语言 / title / 声道 / 码率 / default / 推荐标记
④ 点任意音轨 → 通过 mpv IPC 让 mpv 切到该轨播放原文件；可任意跳转
   （此时磁盘上零新增文件；波形若需要，由工具现场计算，不落盘）
⑤ 你确认音轨 + 选定参数
⑥ 工具执行：分离音轨 → 时间轴对齐 → 编码正式音频 → （可选）生成画面版 → 输出对齐报告
⑦ 把正式音频导入渡幕，开始打轴
⑧ 脚本为渡幕旁挂画面面板，画面跟随其播放头
```

---

## 5. 各交付物技术方案

### A. TJ-Prep

**形态**：Python 3.10（标准库 Tkinter）+ mpv 0.36（IPC 控制）。零 pip 安装、零 npm 安装，`python.exe` 直接跑，不打包成 exe（启动快、便于修改）。

**选轨阶段（零文件）**：
- `ffprobe -show_streams` 取全部音轨元数据，按"是否 default、语言、title 是否含 Commentary、声道数、码率"给推荐标记
- 点列表项 → JSON IPC 发给 mpv（`--input-ipc-server`）切轨；跳转发 `seek` 命令
- 波形：读 ffmpeg 输出的 PCM 管道，按窗算 min/max，Tkinter Canvas 绘制；**纯内存、不落盘**

**分离阶段（你确认后才执行）**：
- 抽取音轨：`-map 0:a:<N>`
- 时间轴对齐：`ffprobe` 读视频流与音频流首帧 PTS，差值即延迟；正延迟用 `adelay` 补静音头，负延迟裁头 → 保证音频 0 秒 = 视频 0 秒
- 声道模式（三档，见第 7 节开放项）：原生多声道 / 2 声道 downmix / 中置单声道
- 编码：FLAC（默认）或 mp3
- 画面版：`-vf scale=-2:720 -c:v libx264 -preset veryfast -crf 30 -g 24 -an -movflags +faststart`（H.264/mp4，短 GOP 便于拖动，无音轨，faststart 便于 seek）
- 对齐报告：源文件哈希、音轨索引、测得的 delay、ffprobe 原始输出、ffmpeg 版本与完整参数

### B. 画面面板脚本

- Tampermonkey，`@run-at document-start`，`@grant none`（**必须**，否则脚本进沙箱世界就 hook 不到页面对象）
- hook `window.Audio` 拿单例引用（只取引用，不改行为）
- 旁挂 `<video muted playsinline>`，rAF 循环读 `audio.currentTime` 驱动画面
- 跟随策略：播放时 `video.play()` 自然走，漂移超过阈值（约 80 ms）才回正；暂停/拖动时精确定位（**不做每帧 seek，否则解码卡顿**）
- 视频文件经 File System Access API 选一次，handle 存 IndexedDB 复用
- 按项目/trackId 记忆"哪个项目配哪个视频"
- 抗更新：完全在浏览器侧，软件更新无影响；脚本不往软件目录写任何东西

### C. 校准片工具

- 生成 30 秒校准片：第 10 秒画面闪一帧白 + 第 10 秒 1 kHz beep
- 按目标容器类型（mkv / mp4 / ts / vob / avi）各生成一份
- 人工验证：走完整流程后，看渡幕里 beep 峰值是否落在 10.000 秒
- 机器验证（每次跑流水线自动执行）：比对原视频音轨首帧 PTS 与分离后音频起始时间、比对总时长

### D. UI 架构选型（待决策）

前提：TJ-Prep 的播放引擎**必须是 mpv**——浏览器和 WebView2 都无法切换嵌入音轨（`HTMLMediaElement.audioTracks` 仅 Safari 支持）。所以"选架构"实际是在选 **UI 层**，不是在选播放能力。

| 路线 | UI 形态 | 播放 | 依赖 | 单窗口 | 波形观感 | 开发周期 |
|---|---|---|---|---|---|---|
| ① Python + Tkinter | 原生控件 | mpv 独立窗口 | Python（已有） | 否 | 一般 | 最短 |
| ② Python + 本地 HTML | 浏览器页面 | mpv 独立窗口 | Python（已有） | 否 | 好（Canvas） | 短 |
| ③ Python + pywebview | 原生窗口内嵌 WebView | mpv 独立或嵌入 | Python + pywebview（1 个 pip 包） | 是 | 好（Canvas） | 中 |
| ④ Tauri + tauri-plugin-libmpv | 原生窗口内嵌 WebView | libmpv 嵌入 | Rust 工具链（未安装）+ 第三方社区插件 | 是 | 好（Canvas） | 最长 |

补充说明：

- 路线 ④ 技术上成立，已有真实项目用 "mpv + Tauri" 做视频播放器（含一个 53 MB 的 Windows 安装包）；嵌入 mpv 依赖社区插件 `tauri-plugin-libmpv`，需要配置窗口透明。
- 本机（WSL）与你的 Windows 侧**均未安装 Rust 工具链**（cargo / rustc / rustup 全无）。
- 无论选哪条路线，核心逻辑（音轨分析、时间轴对齐、分离编码、校验）都与 UI 解耦，UI 层可在后期单独替换。

---

## 6. 已确定的技术决策

| 项 | 决策 | 理由 |
|---|---|---|
| 软件改动 | 一个字节都不改 | R1；所有注入在浏览器侧完成 |
| 工具形态 | 桌面工具，不是 WebUI | WebUI 因浏览器不能切嵌入音轨，必然要转码出中间文件，违反 R2 |
| 试听引擎 | mpv（直接读原视频） | 原生支持切轨，零转码、零中间文件 |
| 画面面板 | Tampermonkey 用户脚本 | 天然运行在页面世界，hook 最直接；你已倾向脚本 |
| 正式音频编码 | FLAC | R5：无损 → 渡幕里的波形与源 PCM 一致 |
| 声道 | 默认 2 声道 downmix | Chrome 对多声道先 downmix，多声道无实际收益（且有转码风险、体积翻倍） |
| 时间轴对齐 | ffprobe 测延迟 + adelay/裁头补偿 | R4 |

---

## 7. 待你决策的开放项

| # | 问题 | 选项 | 我的建议 |
|---|---|---|---|
| Q1 | 正式音频默认档 | ① FLAC 2ch ② FLAC 中置 mono ③ 原生多声道 | ① ；③ 作为额外输出另存一份供 mpv 欣赏 |
| Q2 | 是否额外输出原生多声道存档 | 是 / 否 | 是（不导入软件，纯存档/欣赏，不影响打轴） |
| Q3 | UI 架构选型（详见 5.D） | ① Python+Tkinter ② Python+本地HTML ③ Python+pywebview ④ Tauri+libmpv | ② 或 ③（拿到 Canvas 波形观感，依赖很轻） |
| Q4 | 工具内是否需要看波形 | 需要 / 不需要（只关心渡幕里的波形） | 需要（选轨时辅助判断，且纯内存无额外文件） |
| Q5 | 画面版视频参数 | 720p / 480p / 1080p；CRF | 720p、CRF 30（你已说明不需要原版画质） |
| Q6 | 画面面板面板位置 | 浮窗可拖动 / 固定侧栏 / 独立窗口 | 浮窗可拖动（不挤占渡幕原有布局） |
| Q7 | 校准片要覆盖哪些容器 | mkv / mp4 / ts / vob / avi | 你实际会遇到的类型（待你列） |
| Q8 | 是否现在做 5.1 FLAC 实测 | 是 / 否 | 是（会往渡幕导入一个测试文件，产生测试项目数据） |

---

## 8. 实施阶段

| 阶段 | 内容 | 产出 | 前置 |
|---|---|---|---|
| 0 | 验证性实测：5.1 FLAC 是否触发转码；渡幕波形峰值窗口精度 | 实测报告（追加进本节） | Q8 授权 |
| 1 | TJ-Prep 选轨部分：音轨表 + 推荐 + mpv 试听 + 跳转 + 波形 | 可用工具（`tj-prep.py` + 启动 bat） | Q3、Q4 |
| 2 | 分离/编码 + 时间轴对齐 + 对齐报告 + 校准片 | 工具扩展 + 报告 + 校准片 | Q1、Q2、Q5、Q7 |
| 3 | 画面面板脚本 | `.user.js` 脚本 | Q6 |
| 4 | 端到端验证 + 使用说明 | 验证记录 + README | 阶段 1–3 完成 |

---

## 9. 已知边界与风险

- 渡幕的波形峰值窗口精度、fallback 触发条件都在编译后的 `backend.exe` 里，静态不可读，需实测确认。
- 若渡幕未来改成不用 `new Audio()` 播放（例如换 Web Audio 直接解码），画面脚本的 hook 点会失效，需要重新定位；脚本会做特征探测与降级（退化为读界面时间码）。
- 多声道原始输出无法在渡幕内"原生播放"：Chrome 会先 downmix，渡幕的声道处理也只认左右两路。这是浏览器侧限制，不是配置问题。
- 渡幕会把音频复制进自身存储目录，正式音频体积会影响其存储占用（FLAC 2ch 1 小时约 350 MB）。
- 授权相关（`license.json`、公钥补丁）与本计划无关，不涉及、不修改。

---

## 10. 明确不做的事

- 不修改渡幕的任何文件（含 `backend.exe`、`frontend-dist`、`index.html`）
- 不往渡幕的存储目录放视频文件
- 不复制、不移动你的原视频
- 不触碰授权/许可证相关逻辑
- 不做云上传相关改动
- 不在你确认前执行任何分离、编码、转码操作

---

## 附：术语

- **干跑（dry-run）**：只分析不产出文件。
- **校准片**：人造的可验证样本，用来定量测出流水线的时间偏移。
- **对齐报告**：一次分离操作的时间轴证据留档，用于复核与更新后重建。
