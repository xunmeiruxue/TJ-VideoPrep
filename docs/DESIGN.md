# 设计依据与决策记录

本文件记录 TJ-VideoPrep 每个设计选择的**证据与理由**，便于日后复核，也便于版本更新后快速判断哪些结论仍然成立。

## 1. 为什么音频默认是「FLAC + 2 声道」

### 1.1 目标软件接受的格式

打轴软件（渡幕-Web v0.11.0）前端硬编码接受的扩展名共 8 种：

```
.aac  .aiff  .flac  .m4a  .mp3  .ogg  .opus  .wav
```

界面提示为"仅支持音频"。

### 1.2 云端转写的格式能力

其云端转写引擎为 ElevenLabs Scribe（软件内的 engine 选项与参数命名与之对应）。官方支持格式：

```
AAC / AIFF / OGG / MP3 / OPUS / WAV / FLAC / M4A / WebM，单文件上限 3 GB
```

官方**未**对格式优劣做推荐。业界通行结论（AssemblyAI 格式指南）：WAV 与 FLAC 最适合语音转写，MP3/AAC/M4A 对多数工作也够用，128 kbps 以上实际差异极小；FLAC 被视为"大批量处理的合理默认"，因为它与 WAV 逐位一致而体积约为其一半。

### 1.3 为什么波形要求无损

打轴软件的波形生成链路（已核实）：

```
POST /tracks/{id}/waveform  →  后端解码音频算峰值  →  /waveform/status 轮询
→  返回 waveformData{startMs,endMs,frames} + waveformDataLeft / waveformDataRight
→  前端 canvas 按 zoomPxPerSec（默认 100 px/s，范围 0.05–2000）绘制
```

**波形的精度上限 = 所给音频文件的精度。** 给 FLAC，峰值与源 PCM 一致；给 MP3，峰值来自有损解码后的信号。因此正式输出用 FLAC。

### 1.4 为什么是 2 声道而不是原生多声道

浏览器侧有硬性行为：除 Firefox 外，Chrome / Edge / Opera / Safari 对 `audio` / `video` 元素的**多声道音频一律先做立体声 downmix**，之后才进入 Web Audio 图（参见 Web Audio 规范讨论 issue #286）。打轴软件正是用 `createMediaElementSource` 接入 Web Audio 做声道分离，因此它拿到时已经是 2 声道。

配套证据：

- 软件的声道处理只准备了左右两路数据：`ChannelSplitter(2)` / `ChannelMerger(2)` / `waveformDataLeft` / `waveformDataRight`
- 软件提供"声道切换：左声道 / 混合 / 右声道"，**单声道时该功能不可用**（界面提示"单声道音频，声道切换不可用"），其 `is_mono` 标记由后端分析得出
- 云端走的是 diarization（说话人分离）路线——转写参数为 `enable_diarization` / `tag_audio_events` / `max_duration_s` / `min_duration_s` / `max_chars_per_line` / `gap_ms` / `merge_gap_threshold_s`，**没有 `use_multi_channel`**；而 ElevenLabs 的多声道模式与 diarization 互斥
- ElevenLabs 多声道模式的语义是"每个声道一个独立说话人"（最多 5 声道），**电影 5.1 不符合该语义**（中置是对白、环绕是环境声）

结论：给多声道既拿不到原生播放，还会放大体积与触发转码的风险；2 声道 downmix 是唯一正确路径。原生多声道保留为**可选输出**，供 mpv 等播放器欣赏，不导入打轴软件。

### 1.5 中置单声道什么时候用

5.1/7.1 源中，对白主要在中置声道。若对白被音乐音效掩盖导致识别不佳，用 `pan=mono|c0=FC` 只取中置。若源本身没有中置声道（立体声素材），本工具自动退化为标准单声道下混，不会生成无效滤镜。

## 2. 时间轴对齐

### 2.1 偏移来源

音频与视频不在同一时间起点的原因都在容器层：

- Matroska 的音轨延迟（音轨相对视频轨的起始差）
- MP4 的 edit list / AAC priming
- 负的 `start_time`

### 2.2 处理方式

`ffprobe` 分别读出视频流与所选音频流的 `start_time`：

- 差值为**正**（音轨晚于视频）→ 音频头部补静音：`adelay=<ms>:all=1`
- 差值为**负**（音轨早于视频）→ 裁掉音频开头：`atrim=start=<s>,asetpts=PTS-STARTPTS`
- 差值在 ±5 ms 内 → 视为无需补偿

界面会显示这条判断及具体毫秒数，导出时可选择是否应用。

### 2.3 校验

- **机器校验**（每次导出前展示）：视频与音频的起始差、总时长对比
- **人工校验**（按容器类型各做一次）：用校准片验证。校准片为 30 秒素材，第 10 秒画面闪一帧白、同时第 10 秒插一声 1 kHz beep；导入打轴软件后检查 beep 峰值是否落在 10.000 秒。此项为一次性验证，仅在更换 ffmpeg 版本、修改参数或遇到新容器类型时重做

## 3. mpv 策略：为什么下载 mpv.exe 而不是内嵌 libmpv

试听需要「切换视频内的嵌入音轨」这一能力。浏览器与 WebView2 都不具备——`HTMLMediaElement.audioTracks` 仅 Safari 实现（MDN 兼容性表），Chromium 系全部不支持。因此播放引擎必须是 mpv 一类的外部播放器。

采用**外部 mpv.exe + 命令行调用**而非内嵌 libmpv，理由：

- 不需要让 Tauri 窗口与外部渲染面做句柄嵌入，规避透明窗口 / DPI / 焦点的坑
- mpv 可被独立替换与升级，版本问题不影响主程序
- 下载源稳定：`zhongfly/mpv-winbuild` 的 GitHub Release 提供 x86_64 通用构建（`.7z`，约 33 MB），由 CI 持续产出

获取方式两条都实现：一键下载解压到 `vendor/mpv/`，或指定本地已有 mpv（例如 `E:\ARCTIME_PRO_4.5_WIN64\tools\MPV\mpv.exe`）。

试听命令形态：

```
mpv.exe "<源视频>" --aid=<stream_index+1> --start=<秒> --force-window=yes --keep-open=yes --osc=yes
```

（mpv 的 track id 约定为 ffmpeg stream index + 1；窗口内按 `#` 可循环切换，作为兜底。）

## 4. 视频输出分档的理由

| 档位 | 参数 | 用途 |
|---|---|---|
| 预览版 | 720p / CRF 30 / `-g 24` / `-an` / `+faststart` | 打轴时看画面。短 GOP 让拖动时关键帧密集、响应快；无音轨减小体积；faststart 把索引前置，seek 即时 |
| 高质量版 | 原分辨率 / CRF 18 / `-an` / `+faststart` | 更完整的输出 |
| 原样封装 | `-c copy` | 无损归档，速度最快 |

统一使用 H.264 + mp4：这是浏览器与各类播放器兼容性最好的组合，避开 MKV 与 HEVC 的兼容问题。

## 5. 干净不残留的实现

- 程序目录即数据目录：`TJ-VideoPrep.settings.json`、`vendor/`、`runtime/` 全在 exe 同级
- 不使用任何把数据写往 `%APPDATA%` 的插件（因此未引入 Tauri 的 store 插件，配置自行以 JSON 读写）
- 文件对话框使用 `rfd`（原生对话框），不经由插件在系统侧留下状态
- 安装形态二选一：NSIS 安装包（安装时可选目录，`installMode: perMachine`）或免安装 zip
- 不做 WebView2 数据目录改写的猜测性配置；若需彻底便携（连 WebView2 缓存也不落系统目录），可在后续用环境变量 `WEBVIEW2_USER_DATA_FOLDER` 指向程序目录，此项**待实测确认**

## 6. 与打轴软件的配合（另行交付）

打轴时"看着画面"由浏览器侧旁挂面板实现，属于独立交付物，不在本工具内：

- 形式：Tampermonkey 用户脚本（`@grant none`，确保运行在页面世界才能 hook 页面的 `window.Audio`）
- 原理：打轴软件的播放器是全局单例 `new Audio()`，音频由本地后端 `http://127.0.0.1:<port>/api/tracks/{id}/media/audio` 提供；脚本取其引用，驱动一个静音 `<video>` 跟随播放头
- 跟随策略：播放时让 video 自然播放，仅在漂移超过约 80 ms 时校正；暂停/拖动时精确定位（不做每帧 seek，否则解码卡顿）
- 该方案不改动打轴软件任何文件，软件更新不影响它

## 7. 路线图状态

**已完成**

1. 音轨分析、建议与 mpv 试听（含 `--aid` 为「音频轨序号」的映射修正）
2. 时间轴偏移检测与补偿（正偏移补静音、负偏移裁头）
3. 音频输出：FLAC / mp3 / wav × 三档声道
4. 视频输出：预览版 / 高质量版 / 原样封装
5. 批量队列、拖放添加、多选与文件夹扫描
6. 音轨单选与未选音轨确认、输出位置三档

**已拆分到独立仓库 `TJ-Sync`**（属于打轴环节的配套，与本仓库"视频 → 产物"的职责不同）

- 校准片生成（`make_calib.py`）与闭环校验判据
- 画面面板用户脚本（打轴时看画面）

**已放弃**

- **独立的波形显示**：打轴软件自带波形（后端 `POST /tracks/{id}/waveform` 生成、
  前端 canvas 渲染、分左右两路），打轴时看的本来就是它。再做一个是重复劳动。

**待做**

- 帧截图：在当前播放位置抓帧，辅助确认口型
- draft release 正式发布、版本号管理（现为 0.1.0）
- `actions/checkout@v4` 的 Node 20 弃用提示可顺手升级

## 8. 已知限制

- 渡幕的波形峰值窗口精度、fallback 转码触发条件位于其编译后的 `backend.exe` 内，静态不可读，需实测
- 若渡幕将来不再使用 `new Audio()` 播放，画面面板脚本的 hook 点会失效，需重新定位
- 本工具假设源文件的音轨可被 ffmpeg 正常解码；异常容器需先修复
