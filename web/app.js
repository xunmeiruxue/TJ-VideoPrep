/* TJ-VideoPrep 前端逻辑
 * 不使用任何 npm 包：依赖 Tauri 的 withGlobalTauri 暴露的 window.__TAURI__
 * 注意：嵌套结构体字段按 Rust 侧 snake_case 传递（serde 不做 camelCase 转换）
 */

const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const $ = (id) => document.getElementById(id);

const state = {
  files: [],        // { path, name, status, error, probe, selected, selectedAuto }
  active: -1,
  outMode: "app",
  customDir: "",
  outputRoot: "",
  appDir: "",
  settings: null,
  running: false,
};

const REC_LABEL = {
  primary: "建议使用",
  alternate: "备选",
  commentary: "评论轨",
  unknown: "未知",
};

const STATUS_LABEL = {
  idle: "待分析",
  probing: "分析中",
  ready: "就绪",
  error: "失败",
};

const STATUS_CLASS = {
  idle: "",
  probing: "info",
  ready: "ok",
  error: "err",
};

/* ---------------------------------------------------------------- 基础工具 */

function log(msg) {
  const el = $("log");
  const ts = new Date().toLocaleTimeString("zh-CN", { hour12: false });
  el.textContent += `[${ts}] ${msg}\n`;
  el.scrollTop = el.scrollHeight;
}

function setProgress(pct, text) {
  $("progress-bar").style.width = `${Math.max(0, Math.min(100, pct))}%`;
  if (text !== undefined) $("progress-text").textContent = text;
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

function baseName(p) {
  const parts = String(p).split(/[\\/]/);
  return parts[parts.length - 1] || String(p);
}

/* ---------------------------------------------------------------- 运行环境 */

async function refreshStatus() {
  const st = await invoke("app_status");
  state.appDir = st.app_dir;
  state.outputRoot = st.output_root;

  $("app-version").textContent = `v${st.version}`;
  $("app-version").title = st.app_dir;
  $("out-app-path").textContent = st.output_root;

  const mark = (el, value, fallback) => {
    if (value) {
      el.textContent = value;
      el.classList.add("env-ok");
      el.classList.remove("env-miss");
    } else {
      el.textContent = fallback;
      el.classList.add("env-miss");
      el.classList.remove("env-ok");
    }
  };

  mark($("env-ffmpeg"), st.ffmpeg, "未找到（请指定或安装）");
  mark($("env-ffprobe"), st.ffprobe, "未找到（请指定或安装）");
  mark($("env-mpv"), st.mpv ? `${st.mpv}${st.mpv_version ? "  ·  " + st.mpv_version : ""}` : null, "未找到（可下载）");

  for (const [id, val] of [["env-ffmpeg", st.ffmpeg], ["env-ffprobe", st.ffprobe], ["env-mpv", st.mpv]]) {
    $(id).title = val || "";
  }
  return st;
}

async function recheck() {
  $("btn-recheck").disabled = true;
  try {
    await refreshStatus();
    log("运行环境已重新检测");
  } catch (e) {
    log(`检测失败：${e}`);
  } finally {
    $("btn-recheck").disabled = false;
  }
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
    await recheck();
  } catch (e) {
    log(`下载失败：${e}（可点"下载页"手动下载后指定路径）`);
  } finally {
    $("btn-dl-mpv").disabled = false;
  }
}

/* ---------------------------------------------------------------- 文件队列 */

async function addPaths(paths) {
  if (!paths || !paths.length) return;
  const expanded = await invoke("expand_paths", { paths });
  if (!expanded.length) {
    log("未找到可处理的媒体文件");
    return;
  }

  const known = new Set(state.files.map((f) => f.path.toLowerCase()));
  const fresh = expanded.filter((p) => !known.has(p.toLowerCase()));
  if (!fresh.length) {
    log("这些文件已经在队列中");
    return;
  }

  const startIndex = state.files.length;
  for (const p of fresh) {
    state.files.push({
      path: p,
      name: baseName(p),
      status: "idle",
      error: "",
      probe: null,
      selected: null,
      selectedAuto: false,
    });
  }

  renderQueue();
  if (state.active < 0) setActive(startIndex);
  log(`已添加 ${fresh.length} 个文件，开始分析音轨…`);

  for (let i = startIndex; i < state.files.length; i++) {
    await probeOne(i);
  }
  log("分析完成");
}

async function probeOne(idx) {
  const f = state.files[idx];
  if (!f) return;
  f.status = "probing";
  renderQueue();

  try {
    const probe = await invoke("probe_video", { path: f.path });
    f.probe = probe;
    f.status = "ready";

    // 只有一条音轨时自动选中；多条时留空，由用户决定（导出前会提示）
    if (probe.audios.length === 1) {
      f.selected = probe.audios[0].ordinal;
      f.selectedAuto = true;
    }
  } catch (e) {
    f.status = "error";
    f.error = String(e);
  }

  renderQueue();
  if (idx === state.active) renderTracks();
  updateStartButton();
}

function renderQueue() {
  const box = $("queue");
  box.textContent = "";
  $("queue-count").textContent = `${state.files.length} 个`;

  if (!state.files.length) {
    const empty = document.createElement("div");
    empty.className = "queue-empty";
    empty.textContent = "还没有文件";
    box.appendChild(empty);
    return;
  }

  state.files.forEach((f, idx) => {
    const item = document.createElement("div");
    item.className = "queue-item" + (idx === state.active ? " active" : "");
    item.addEventListener("click", () => setActive(idx));

    const name = document.createElement("div");
    name.className = "queue-name";
    name.textContent = f.name;
    name.title = f.path;
    item.appendChild(name);

    const meta = document.createElement("div");
    meta.className = "queue-meta";

    const pill = document.createElement("span");
    pill.className = `pill ${STATUS_CLASS[f.status] || ""}`;
    pill.textContent = STATUS_LABEL[f.status] || f.status;
    meta.appendChild(pill);

    if (f.status === "ready" && f.probe) {
      const n = f.probe.audios.length;
      const tracks = document.createElement("span");
      tracks.textContent = `${n} 音轨 · ${fmtTime(f.probe.duration)}`;
      meta.appendChild(tracks);

      const sel = document.createElement("span");
      if (f.selected !== null) {
        const t = f.probe.audios.find((a) => a.ordinal === f.selected);
        sel.className = "pill ok";
        const lang = t && t.language ? ` ${t.language}` : "";
        sel.textContent = `已选 a:${f.selected}${lang}`;
      } else if (n > 1) {
        sel.className = "pill warn";
        sel.textContent = "未选音轨";
      } else {
        sel.className = "pill";
        sel.textContent = "无音频";
      }
      meta.appendChild(sel);
    }

    if (f.status === "error") {
      const err = document.createElement("span");
      err.className = "pill err";
      err.textContent = "分析失败";
      err.title = f.error;
      meta.appendChild(err);
    }

    item.appendChild(meta);

    const del = document.createElement("button");
    del.className = "queue-del";
    del.textContent = "×";
    del.title = "从队列移除";
    del.addEventListener("click", (e) => {
      e.stopPropagation();
      removeFile(idx);
    });
    item.appendChild(del);

    box.appendChild(item);
  });
}

function removeFile(idx) {
  if (state.running) return;
  state.files.splice(idx, 1);
  if (!state.files.length) {
    state.active = -1;
  } else if (state.active >= state.files.length) {
    state.active = state.files.length - 1;
  } else if (idx < state.active) {
    state.active -= 1;
  }
  renderQueue();
  renderTracks();
  updateStartButton();
}

function setActive(idx) {
  state.active = idx;
  renderQueue();
  renderTracks();
  refreshPlan();
}

function activeFile() {
  return state.files[state.active] || null;
}

/* ---------------------------------------------------------------- 音轨表 */

function renderTracks() {
  const tbody = $("track-rows");
  tbody.textContent = "";

  const f = activeFile();

  if (!f) {
    $("current-name").textContent = "未选择文件";
    $("current-meta").textContent = "";
    appendEmptyRow(tbody, "添加文件后在此列出音轨");
    setAlign("等待分析…", "muted");
    return;
  }

  $("current-name").textContent = f.name;
  const p = f.probe;
  $("current-meta").textContent = p
    ? `${p.format_long_name || p.format_name} · ${fmtTime(p.duration)} · ${fmtSize(p.size_bytes)}`
    : f.status === "error"
      ? f.error
      : "分析中…";

  if (!p) {
    appendEmptyRow(tbody, f.status === "error" ? "分析失败" : "分析中…");
    setAlign("等待分析…", "muted");
    return;
  }

  if (!p.audios.length) {
    appendEmptyRow(tbody, "该文件没有音频流");
    setAlign(p.align_note || "无音频", "warn");
    return;
  }

  setAlign(
    p.align_note || "—",
    Math.abs(p.delay_ms) <= 5 ? "ok" : "warn"
  );

  for (const t of p.audios) {
    const tr = document.createElement("tr");
    if (f.selected === t.ordinal) tr.classList.add("selected");
    tr.addEventListener("click", () => selectTrack(t.ordinal));

    const tdRadio = document.createElement("td");
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = "track-select";
    radio.checked = f.selected === t.ordinal;
    radio.title = "选择用于导出的音轨";
    radio.addEventListener("click", (e) => e.stopPropagation());
    radio.addEventListener("change", () => selectTrack(t.ordinal));
    tdRadio.appendChild(radio);
    tr.appendChild(tdRadio);

    const cells = [
      [String(t.ordinal), true],
      [t.codec || "—", true],
      [`${t.channels}ch${t.channel_layout ? " · " + t.channel_layout : ""}${t.sample_rate ? " · " + t.sample_rate + "Hz" : ""}`, true],
      [t.language || "—", false],
      [t.title || "", false],
    ];
    for (const [text, mono] of cells) {
      const td = document.createElement("td");
      td.textContent = text;
      if (mono) td.classList.add("mono");
      tr.appendChild(td);
    }

    const tdRec = document.createElement("td");
    const tag = document.createElement("span");
    tag.className = `tag ${t.recommendation || "unknown"}`;
    tag.textContent = REC_LABEL[t.recommendation] || t.recommendation || "";
    tag.title = t.note || "";
    tdRec.appendChild(tag);
    tr.appendChild(tdRec);

    const tdPlay = document.createElement("td");
    const play = document.createElement("button");
    play.className = "btn tiny";
    play.textContent = "试听";
    play.title = "用 mpv 打开源文件并切到这条音轨，不产生中间文件";
    play.addEventListener("click", (e) => {
      e.stopPropagation();
      previewTrack(t);
    });
    tdPlay.appendChild(play);
    tr.appendChild(tdPlay);

    tbody.appendChild(tr);
  }
}

function appendEmptyRow(tbody, text) {
  const tr = document.createElement("tr");
  tr.className = "empty";
  const td = document.createElement("td");
  td.colSpan = 8;
  td.textContent = text;
  tr.appendChild(td);
  tbody.appendChild(tr);
}

function setAlign(text, cls) {
  const el = $("align-info");
  el.textContent = text;
  el.className = `align-info ${cls}`;
}

function selectTrack(ordinal) {
  const f = activeFile();
  if (!f) return;
  f.selected = ordinal;
  f.selectedAuto = false;
  renderTracks();
  renderQueue();
  updateStartButton();
  refreshPlan();
}

async function previewTrack(t) {
  const f = activeFile();
  if (!f) return;
  try {
    await invoke("open_in_mpv", {
      req: {
        video: f.path,
        audio_ordinal: t.ordinal,
        start_seconds: 0,
      },
    });
    log(`已用 mpv 打开音轨 a:${t.ordinal}（${t.codec} ${t.channels}ch${t.language ? " " + t.language : ""}）。mpv 窗口里按 # 可循环切换音轨。`);
  } catch (e) {
    log(`无法启动 mpv：${e}`);
    $("envbar").classList.remove("hidden");
  }
}

/* ---------------------------------------------------------------- 输出设置 */

function readOptions() {
  return {
    want_audio: $("opt-audio").checked,
    audio_codec: $("opt-audio-codec").value,
    audio_mode: $("opt-audio-mode").value,
    audio_bitrate: $("opt-audio-bitrate").value,
    apply_align: $("opt-align").checked,
    want_preview: $("opt-preview").checked,
    preview_height: parseInt($("opt-preview-height").value, 10) || 720,
    preview_crf: parseInt($("opt-preview-crf").value, 10) || 30,
    want_high: $("opt-high").checked,
    high_crf: parseInt($("opt-high-crf").value, 10) || 18,
    want_remux: $("opt-remux").checked,
  };
}

function hasAnyOutput(opts) {
  return opts.want_audio || opts.want_preview || opts.want_high || opts.want_remux;
}

function buildRequest(f) {
  const opts = readOptions();
  const audios = (f.probe && f.probe.audios) || [];
  let ordinal = f.selected;
  if (ordinal === null) ordinal = audios.length ? audios[0].ordinal : 0;
  const track = audios.find((a) => a.ordinal === ordinal);

  return {
    video: f.path,
    audio_ordinal: ordinal,
    channel_layout: track ? track.channel_layout || "" : "",
    base_name: "",
    duration: f.probe ? f.probe.duration : 0,
    out_mode: state.outMode,
    custom_dir: state.customDir,
    delay_ms: f.probe ? f.probe.delay_ms : 0,
    ...opts,
  };
}

async function refreshPlan() {
  const f = activeFile();
  if (!f || f.status !== "ready") {
    $("cmd-preview").textContent = "选择文件与音轨后可预览实际调用参数";
    return;
  }
  try {
    const lines = await invoke("describe_plan", { req: buildRequest(f) });
    $("cmd-preview").textContent = lines.join("\n\n");
  } catch (e) {
    $("cmd-preview").textContent = String(e);
  }
}

function updateStartButton() {
  const ready = state.files.filter((f) => f.status === "ready").length;
  $("btn-start").disabled = state.running || ready === 0;
  $("btn-start").textContent = ready > 1 ? `开始输出（${ready} 个文件）` : "开始输出";
}

/* ---------------------------------------------------------------- 输出位置 */

function readOutMode() {
  const el = document.querySelector('input[name="outmode"]:checked');
  state.outMode = el ? el.value : "app";
  $("custom-dir-row").classList.toggle("hidden", state.outMode !== "custom");
  refreshPlan();
}

async function pickCustomDir() {
  const dir = await invoke("pick_dir");
  if (!dir) return;
  state.customDir = dir;
  $("custom-dir").textContent = dir;
  $("custom-dir").classList.remove("muted");
  refreshPlan();
}

function currentOutputDirHint() {
  if (state.outMode === "app") return state.outputRoot;
  if (state.outMode === "source") return "各源文件目录下的 TJ-Output";
  return state.customDir || "（未设置）";
}

/* ---------------------------------------------------------------- 模态框 */

function confirmDialog(title, note, items) {
  return new Promise((resolve) => {
    $("modal-title").textContent = title;
    const body = $("modal-body");
    body.textContent = "";
    if (note) {
      const p = document.createElement("p");
      p.style.margin = "0 0 4px";
      p.textContent = note;
      body.appendChild(p);
    }
    if (items && items.length) {
      const ul = document.createElement("ul");
      for (const it of items) {
        const li = document.createElement("li");
        li.textContent = it;
        ul.appendChild(li);
      }
      body.appendChild(ul);
    }
    $("modal").classList.remove("hidden");

    const finish = (value) => {
      $("modal").classList.add("hidden");
      $("modal-ok").onclick = null;
      $("modal-cancel").onclick = null;
      resolve(value);
    };
    $("modal-ok").onclick = () => finish(true);
    $("modal-cancel").onclick = () => finish(false);
  });
}

/* ---------------------------------------------------------------- 执行 */

async function startEncode() {
  if (state.running) return;

  const ready = state.files.filter((f) => f.status === "ready");
  if (!ready.length) {
    log("没有可处理的文件");
    return;
  }

  const opts = readOptions();
  if (!hasAnyOutput(opts)) {
    log("请至少选择一项输出内容");
    return;
  }

  if (state.outMode === "custom" && !state.customDir) {
    log("输出位置选了「指定目录」，但还没有设置目录");
    return;
  }

  // 多条音轨且未选择的文件
  const pending = ready.filter((f) => f.selected === null && (f.probe?.audios.length || 0) > 1);

  if (pending.length) {
    if (!$("opt-auto-first").checked) {
      await confirmDialog(
        "有文件尚未选择音轨",
        "以下文件包含多条音轨却未选择，且「未选时默认第一条」已关闭，无法继续：",
        pending.map((f) => `${f.name}（${f.probe.audios.length} 条音轨）`)
      );
      return;
    }
    const ok = await confirmDialog(
      "有文件尚未选择音轨",
      "以下文件包含多条音轨，你没有选择。继续将默认使用每个文件的第一条音轨：",
      pending.map((f) => `${f.name}（共 ${f.probe.audios.length} 条，将用 a:${f.probe.audios[0].ordinal} ${f.probe.audios[0].language || ""}）`)
    );
    if (!ok) return;
  }

  const reqs = ready.map(buildRequest);

  state.running = true;
  $("btn-start").disabled = true;
  $("btn-cancel").disabled = false;
  $("log").textContent = "";
  setProgress(0, "开始…");
  log(`开始处理 ${reqs.length} 个文件，输出位置：${currentOutputDirHint()}`);

  try {
    const outputs = await invoke("start_encode", { reqs });
    log(`全部完成，共 ${outputs.length} 项输出`);
    for (const p of outputs) log(`  ${p}`);
    setProgress(100, "完成");
  } catch (e) {
    log(`失败：${e}`);
    setProgress(0, "失败");
  } finally {
    state.running = false;
    $("btn-cancel").disabled = true;
    updateStartButton();
    renderQueue();
  }
}

async function cancelEncode() {
  await invoke("cancel_encode");
  log("已请求取消…");
}

async function onJobEvent(payload) {
  if (!payload) return;
  setProgress(payload.percent, payload.message);
  if (payload.stage === "mpv") {
    return;
  }
  if (payload.message) log(payload.message);
}

/* ---------------------------------------------------------------- 设置读写 */

async function initSettings() {
  const s = await invoke("get_settings");
  state.settings = s;

  if (s.preview_height) $("opt-preview-height").value = String(s.preview_height);
  if (s.preview_crf) $("opt-preview-crf").value = String(s.preview_crf);
  if (s.high_crf) $("opt-high-crf").value = String(s.high_crf);
  if (s.audio_codec) $("opt-audio-codec").value = s.audio_codec;
  if (s.audio_mode) $("opt-audio-mode").value = s.audio_mode;
  if (s.audio_bitrate) $("opt-audio-bitrate").value = s.audio_bitrate;

  toggleBitrateField();
}

function toggleBitrateField() {
  $("field-bitrate").style.visibility = $("opt-audio-codec").value === "mp3" ? "visible" : "hidden";
}

async function persistOptions() {
  const opts = readOptions();
  const s = {
    ...(state.settings || {}),
    preview_height: opts.preview_height,
    preview_crf: opts.preview_crf,
    high_crf: opts.high_crf,
    audio_codec: opts.audio_codec,
    audio_mode: opts.audio_mode,
    audio_bitrate: opts.audio_bitrate,
  };
  state.settings = s;
  await invoke("set_settings", { settings: s }).catch(() => {});
}

/* ---------------------------------------------------------------- 事件绑定 */

function bindEvents() {
  $("btn-env").addEventListener("click", () => $("envbar").classList.toggle("hidden"));
  $("btn-recheck").addEventListener("click", recheck);
  $("btn-pick-ffmpeg").addEventListener("click", () => pickTool("ffmpeg"));
  $("btn-pick-mpv").addEventListener("click", () => pickTool("mpv"));
  $("btn-dl-mpv").addEventListener("click", downloadMpv);
  $("btn-mpv-page").addEventListener("click", async () => {
    const url = await invoke("mpv_download_page");
    await invoke("open_url", { url });
  });

  $("btn-add-files").addEventListener("click", async () => {
    const list = await invoke("pick_video_files");
    await addPaths(list);
  });

  $("btn-add-folder").addEventListener("click", async () => {
    const list = await invoke("pick_video_folder");
    await addPaths(list);
  });

  $("btn-clear").addEventListener("click", () => {
    if (state.running) return;
    state.files = [];
    state.active = -1;
    renderQueue();
    renderTracks();
    updateStartButton();
    log("队列已清空");
  });

  $("btn-start").addEventListener("click", startEncode);
  $("btn-cancel").addEventListener("click", cancelEncode);
  $("btn-describe").addEventListener("click", refreshPlan);
  $("btn-pick-custom").addEventListener("click", pickCustomDir);

  $("btn-open-dir").addEventListener("click", async () => {
    await invoke("open_in_explorer", { path: state.appDir });
  });

  $("btn-open-out").addEventListener("click", async () => {
    const target = state.outMode === "app" ? state.outputRoot : state.customDir;
    if (target) {
      await invoke("open_in_explorer", { path: target });
    } else {
      log("当前输出位置不是单一目录（按源文件目录输出）");
    }
  });

  for (const el of document.querySelectorAll('input[name="outmode"]')) {
    el.addEventListener("change", readOutMode);
  }

  $("opt-audio-codec").addEventListener("change", () => {
    toggleBitrateField();
    persistOptions();
    refreshPlan();
  });

  const replan = ["opt-audio-mode", "opt-audio-bitrate", "opt-preview-height", "opt-preview-crf", "opt-high-crf"];
  for (const id of replan) {
    $(id).addEventListener("change", () => {
      persistOptions();
      refreshPlan();
    });
  }

  for (const id of ["opt-audio", "opt-preview", "opt-high", "opt-remux", "opt-align"]) {
    $(id).addEventListener("change", refreshPlan);
  }
}

/* ---------------------------------------------------------------- 启动 */

window.addEventListener("DOMContentLoaded", async () => {
  bindEvents();

  try {
    await listen("job-event", (e) => onJobEvent(e.payload));

    await listen("files-dropped", async (e) => {
      $("drop-overlay").classList.add("hidden");
      await addPaths(e.payload || []);
    });

    await listen("drag-enter", () => $("drop-overlay").classList.remove("hidden"));
    await listen("drag-leave", () => $("drop-overlay").classList.add("hidden"));

    await refreshStatus();
    await initSettings();
    readOutMode();

    renderQueue();
    renderTracks();
    setProgress(0, "就绪");
    log("TJ-VideoPrep 已启动。所有数据只写在程序目录内。");
  } catch (e) {
    document.body.insertAdjacentHTML(
      "afterbegin",
      `<div style="padding:12px;color:#da3633">初始化失败：${String(e)}</div>`
    );
  }
});
