/* TJ-VideoPrep 前端逻辑
 * 不使用任何 npm 包：依赖 Tauri 的 withGlobalTauri 暴露的 window.__TAURI__
 * 注意：嵌套结构体字段按 Rust 侧 snake_case 传递（serde 不做 camelCase 转换）
 */

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);

const state = {
  video: null,
  probe: null,
  selected: null,
  outDir: "",
  settings: null,
  running: false,
};

function log(msg) {
  const el = $("log");
  const ts = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  el.textContent += `[${ts}] ${msg}\n`;
  el.scrollTop = el.scrollHeight;
}

function setProgress(pct, text) {
  $("progress-bar").style.width = `${Math.max(0, Math.min(100, pct))}%`;
  if (text !== undefined) {
    $("progress-text").textContent = text;
  }
}

function fmtTime(sec) {
  if (!sec || sec <= 0) return "—";
  const s = Math.floor(sec % 60);
  const m = Math.floor((sec / 60) % 60);
  const h = Math.floor(sec / 3600);
  const pad = (n) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

function fmtSize(bytes) {
  if (!bytes) return "—";
  const gb = bytes / 1073741824;
  if (gb >= 1) return `${gb.toFixed(2)} GB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

const REC_LABEL = {
  primary: "建议使用",
  alternate: "备选",
  commentary: "评论轨",
  unknown: "未知",
};

/* ---------------------------------------------------------------- 环境状态 */

async function refreshStatus() {
  const st = await invoke("app_status");
  $("app-version").textContent = `v${st.version}`;
  $("app-version").title = st.app_dir;

  const mark = (el, value, label) => {
    if (value) {
      el.textContent = value;
      el.classList.add("env-ok");
      el.classList.remove("env-miss");
    } else {
      el.textContent = label || "未找到";
      el.classList.add("env-miss");
      el.classList.remove("env-ok");
    }
  };

  mark($("env-ffmpeg"), st.ffmpeg, "未找到（请指定或安装）");
  mark($("env-ffprobe"), st.ffprobe, "未找到（请指定或安装）");
  mark($("env-mpv"), st.mpv ? `${st.mpv}${st.mpv_version ? "  ·  " + st.mpv_version : ""}` : null, "未找到（可下载）");

  $("env-mpv").title = st.mpv || "";
  $("env-ffmpeg").title = st.ffmpeg || "";
  $("env-ffprobe").title = st.ffprobe || "";
  return st;
}

/* ---------------------------------------------------------------- 源文件 */

async function pickVideo() {
  const path = await invoke("pick_file", { kind: "video" });
  if (!path) return;

  state.video = path;
  state.probe = null;
  state.selected = null;
  $("video-path").textContent = path;
  $("video-path").classList.remove("muted");
  $("media-meta").textContent = "";
  $("align-info").textContent = "分析中…";
  $("align-info").className = "align-info muted";
  setProgress(0, "正在分析…");
  $("btn-start").disabled = true;

  try {
    const probe = await invoke("probe_video", { path });
    state.probe = probe;
    renderMeta(probe);
    renderTracks(probe);

    const first = probe.audios.find((a) => a.recommendation === "primary") || probe.audios[0];
    if (first) selectTrack(first);

    const d = probe.delay_ms;
    const info = $("align-info");
    info.textContent = probe.align_note;
    info.className =
      "align-info " + (Math.abs(d) <= 5 ? "ok" : "warn");

    // 记忆最近使用的文件与输出目录
    const s = { ...state.settings, last_video: path };
    if (state.outDir) s.out_dir = state.outDir;
    state.settings = s;
    await invoke("set_settings", { settings: s });

    setProgress(0, "就绪");
    log(`已加载 ${probe.file_name}（${probe.audios.length} 条音轨，时长 ${fmtTime(probe.duration)}）`);
  } catch (err) {
    setProgress(0, "分析失败");
    $("align-info").textContent = String(err);
    $("align-info").className = "align-info warn";
    log(`分析失败：${err}`);
  }
}

function renderMeta(p) {
  const v = p.video || {};
  const parts = [
    ["容器", p.format_long_name || p.format_name || "—"],
    ["时长", fmtTime(p.duration)],
    ["体积", fmtSize(p.size_bytes)],
    ["画面", v.width ? `${v.width}×${v.height}` : "—"],
    ["视频编码", v.codec || "—"],
    ["帧率", v.r_frame_rate && v.r_frame_rate !== "0/0" ? v.r_frame_rate : "—"],
    ["像素格式", v.pix_fmt || "—"],
  ];
  if (v.is_hdr) parts.push(["动态范围", "HDR / 10bit+"]);

  const box = $("media-meta");
  box.textContent = "";
  for (const [k, val] of parts) {
    const span = document.createElement("span");
    const b = document.createElement("b");
    b.textContent = val;
    span.append(`${k}：`, b);
    box.appendChild(span);
  }
}

/* ---------------------------------------------------------------- 音轨表 */

function renderTracks(probe) {
  const tbody = $("track-rows");
  tbody.textContent = "";

  if (!probe.audios.length) {
    const tr = document.createElement("tr");
    tr.className = "empty";
    const td = document.createElement("td");
    td.colSpan = 7;
    td.textContent = "该文件没有音频流";
    tr.appendChild(td);
    tbody.appendChild(tr);
    return;
  }

  for (const t of probe.audios) {
    const tr = document.createElement("tr");
    tr.dataset.ordinal = String(t.ordinal);

    const cells = [
      String(t.ordinal),
      t.codec || "—",
      `${t.channels}ch${t.channel_layout ? " · " + t.channel_layout : ""}`,
      t.language || "—",
      t.title || "",
    ];

    for (const c of cells) {
      const td = document.createElement("td");
      td.textContent = c;
      if (c === cells[0] || c === cells[1] || c === cells[2]) td.classList.add("mono");
      tr.appendChild(td);
    }

    const recTd = document.createElement("td");
    const tag = document.createElement("span");
    tag.className = `tag ${t.recommendation || "unknown"}`;
    tag.textContent = REC_LABEL[t.recommendation] || t.recommendation;
    tag.title = t.note || "";
    recTd.appendChild(tag);
    tr.appendChild(recTd);

    const actTd = document.createElement("td");
    const btn = document.createElement("button");
    btn.className = "btn tiny";
    btn.textContent = "试听";
    btn.title = "用 mpv 打开源文件并切到该音轨，不产生中间文件";
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      previewTrack(t);
    });
    actTd.appendChild(btn);
    tr.appendChild(actTd);

    tr.addEventListener("click", () => selectTrack(t));
    tbody.appendChild(tr);
  }
}

function selectTrack(t) {
  state.selected = t;
  for (const tr of $("track-rows").querySelectorAll("tr")) {
    tr.classList.toggle("selected", tr.dataset.ordinal === String(t.ordinal));
  }
  $("btn-start").disabled = !state.video || state.running;
  refreshPlan();
}

async function previewTrack(t) {
  if (!state.video) return;
  try {
    await invoke("open_in_mpv", {
      req: {
        video: state.video,
        stream_index: t.stream_index,
        start_seconds: 0,
      },
    });
    log(`已用 mpv 打开音轨 ${t.ordinal}（${t.codec} ${t.channels}ch）。窗口内按 # 键可循环切换音轨。`);
  } catch (err) {
    log(`无法启动 mpv：${err}`);
    $("envbar").classList.remove("hidden");
  }
}

/* ---------------------------------------------------------------- 请求组装 */

function collectRequest() {
  const t = state.selected;
  let base = $("opt-basename").value.trim();
  if (!base && state.video) {
    const name = state.video.split(/[\\/]/).pop() || "output";
    base = name.replace(/\.[^.]+$/, "");
  }

  return {
    video: state.video || "",
    audio_ordinal: t ? t.ordinal : 0,
    channel_layout: t ? t.channel_layout || "" : "",
    out_dir: state.outDir,
    base_name: base,
    duration: state.probe ? state.probe.duration : 0,
    want_audio: $("opt-audio").checked,
    audio_codec: $("opt-audio-codec").value,
    audio_mode: $("opt-audio-mode").value,
    audio_bitrate: $("opt-audio-bitrate").value,
    delay_ms: state.probe ? state.probe.delay_ms : 0,
    apply_align: $("opt-align").checked,
    want_preview: $("opt-preview").checked,
    preview_height: parseInt($("opt-preview-height").value, 10) || 720,
    preview_crf: parseInt($("opt-preview-crf").value, 10) || 30,
    want_high: $("opt-high").checked,
    high_crf: parseInt($("opt-high-crf").value, 10) || 18,
    want_remux: $("opt-remux").checked,
  };
}

async function refreshPlan() {
  if (!state.video) return;
  try {
    const lines = await invoke("describe_plan", { req: collectRequest() });
    $("cmd-preview").textContent = lines.length ? lines.join("\n\n") : "没有选择任何输出项";
  } catch (err) {
    $("cmd-preview").textContent = String(err);
  }
}

/* ---------------------------------------------------------------- 执行 */

async function startEncode() {
  if (!state.video || state.running) return;
  if (!state.outDir) {
    log("请先选择输出目录");
    return;
  }

  const req = collectRequest();
  if (!req.want_audio && !req.want_preview && !req.want_high && !req.want_remux) {
    log("至少选择一项输出内容");
    return;
  }

  state.running = true;
  $("btn-start").disabled = true;
  $("btn-cancel").disabled = false;
  $("log").textContent = "";
  setProgress(0, "开始…");
  log("任务开始");

  const s = { ...state.settings, out_dir: state.outDir, last_video: state.video };
  state.settings = s;
  await invoke("set_settings", { settings: s }).catch(() => {});

  try {
    const outputs = await invoke("start_encode", { req });
    log(`完成，共 ${outputs.length} 个文件`);
    outputs.forEach((p) => log(`  ${p}`));
    setProgress(100, "完成");
  } catch (err) {
    log(`失败：${err}`);
    setProgress(0, "失败");
  } finally {
    state.running = false;
    $("btn-start").disabled = !state.video;
    $("btn-cancel").disabled = true;
  }
}

async function cancelEncode() {
  await invoke("cancel_encode");
  log("已请求取消…");
}

/* ---------------------------------------------------------------- 设置 */

async function initSettings() {
  const s = await invoke("get_settings");
  state.settings = s;

  if (s.out_dir) {
    state.outDir = s.out_dir;
    $("out-dir").textContent = s.out_dir;
    $("out-dir").classList.remove("muted");
  }
  if (s.preview_height) $("opt-preview-height").value = String(s.preview_height);
  if (s.preview_crf) $("opt-preview-crf").value = String(s.preview_crf);
  if (s.high_crf) $("opt-high-crf").value = String(s.high_crf);
  if (s.audio_codec) $("opt-audio-codec").value = s.audio_codec;
  if (s.audio_mode) $("opt-audio-mode").value = s.audio_mode;
  if (s.audio_bitrate) $("opt-audio-bitrate").value = s.audio_bitrate;

  if (s.last_video) {
    $("video-path").textContent = `上次使用：${s.last_video}`;
    $("video-path").title = s.last_video;
  }

  // 输出目录默认取源文件旁边的 out 子目录
  toggleBitrateField();
}

function toggleBitrateField() {
  const isMp3 = $("opt-audio-codec").value === "mp3";
  $("field-bitrate").style.visibility = isMp3 ? "visible" : "hidden";
}

async function pickOutDir() {
  const dir = await invoke("pick_dir");
  if (!dir) return;
  state.outDir = dir;
  $("out-dir").textContent = dir;
  $("out-dir").classList.remove("muted");
  refreshPlan();
}

async function pickTool(kind) {
  const path = await invoke("pick_file", { kind });
  if (!path) return;
  const s = { ...(state.settings || {}) };
  if (kind === "mpv") s.mpv_path = path;
  if (kind === "ffmpeg") {
    s.ffmpeg_path = path;
    s.ffprobe_path = path.replace(/ffmpeg(\.exe)?$/i, (m) =>
      m.toLowerCase().endsWith(".exe") ? "ffprobe.exe" : "ffprobe"
    );
  }
  state.settings = s;
  await invoke("set_settings", { settings: s });
  await refreshStatus();
  log(`已记录 ${kind} 路径：${path}`);
}

async function downloadMpv() {
  $("btn-dl-mpv").disabled = true;
  log("开始下载 mpv 到程序目录 vendor/mpv …");
  try {
    const path = await invoke("download_mpv");
    log(`mpv 就绪：${path}`);
    await refreshStatus();
  } catch (err) {
    log(`下载失败：${err}（可点"下载页"手动下载后指定路径）`);
  } finally {
    $("btn-dl-mpv").disabled = false;
  }
}

/* ---------------------------------------------------------------- 事件绑定 */

function bindEvents() {
  $("btn-pick-video").addEventListener("click", pickVideo);
  $("btn-pick-out").addEventListener("click", pickOutDir);
  $("btn-start").addEventListener("click", startEncode);
  $("btn-cancel").addEventListener("click", cancelEncode);
  $("btn-describe").addEventListener("click", refreshPlan);
  $("btn-settings").addEventListener("click", () => $("envbar").classList.toggle("hidden"));

  $("btn-pick-ffmpeg").addEventListener("click", () => pickTool("ffmpeg"));
  $("btn-pick-mpv").addEventListener("click", () => pickTool("mpv"));
  $("btn-dl-mpv").addEventListener("click", downloadMpv);
  $("btn-mpv-page").addEventListener("click", async () => {
    const url = await invoke("mpv_download_page");
    await invoke("open_url", { url });
  });

  $("btn-open-dir").addEventListener("click", async () => {
    const st = await invoke("app_status");
    await invoke("open_in_explorer", { path: st.app_dir });
  });

  $("btn-open-out").addEventListener("click", async () => {
    if (!state.outDir) return;
    await invoke("open_in_explorer", { path: state.outDir });
  });

  $("opt-audio-codec").addEventListener("change", () => {
    toggleBitrateField();
    refreshPlan();
  });

  const replan = [
    "opt-audio-mode",
    "opt-audio-bitrate",
    "opt-preview-height",
    "opt-preview-crf",
    "opt-high-crf",
    "opt-basename",
  ];
  for (const id of replan) {
    $(id).addEventListener("change", refreshPlan);
  }

  const recheck = ["opt-audio", "opt-preview", "opt-high", "opt-remux", "opt-align"];
  for (const id of recheck) {
    $(id).addEventListener("change", refreshPlan);
  }
}

async function onJobEvent(payload) {
  if (!payload) return;
  if (payload.stage === "mpv") {
    // mpv 下载进度
    setProgress(payload.percent, payload.message);
    return;
  }
  setProgress(payload.percent, payload.message);
  if (payload.message) log(payload.message);
  if (payload.error) log("（任务已中止）");
}

/* ---------------------------------------------------------------- 启动 */

window.addEventListener("DOMContentLoaded", async () => {
  bindEvents();
  try {
    await listen("job-event", (event) => onJobEvent(event.payload));
    await refreshStatus();
    await initSettings();
    setProgress(0, "就绪");
    log("TJ-VideoPrep 已启动。所有数据只写在程序目录内，不触碰系统其它位置。");
  } catch (err) {
    document.body.insertAdjacentHTML(
      "afterbegin",
      `<div style="padding:12px;color:#da3633">初始化失败：${String(err)}</div>`
    );
  }
});
