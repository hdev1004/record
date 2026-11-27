const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const $ = (id) => document.getElementById(id);
const STORE_KEY = "recorder.settings";

const state = {
  region: null,   // { x, y, w, h } 물리 픽셀
  label: "",
  kind: "",
  recording: false,
  saving: false,
  startedAt: 0,
  settings: { theme: "system", hotkey: "F9", fps: "30", crf: "23", cursor: true, hide: true, outDir: "" },
};

// ---------------------------------------------------------------- 설정
function loadSettings() {
  try {
    Object.assign(state.settings, JSON.parse(localStorage.getItem(STORE_KEY) || "{}"));
  } catch {}
}
function saveSettings() {
  try { localStorage.setItem(STORE_KEY, JSON.stringify(state.settings)); } catch {}
}

// 긴 경로는 가운데를 생략: "D:\…\상위폴더\마지막폴더" (그래도 길면 끝을 … 로)
function fitPath(el, full) {
  el.textContent = full;
  if (el.scrollWidth <= el.clientWidth) return;
  const parts = full.split(/[\\/]/).filter(Boolean);
  for (let n = parts.length - 2; n >= 1; n--) {
    el.textContent = `${parts[0]}\\…\\${parts.slice(-n).join("\\")}`;
    if (el.scrollWidth <= el.clientWidth) return;
  }
  el.textContent = `…\\${parts[parts.length - 1]}`;
}

function renderSettings() {
  document.querySelectorAll(".seg").forEach((seg) => {
    const v = state.settings[seg.dataset.key];
    seg.querySelectorAll("button").forEach((b) => b.classList.toggle("on", b.dataset.v === v));
  });
  $("opt-cursor").checked = state.settings.cursor;
  $("opt-hide").checked = state.settings.hide;
  $("out-path").title = state.settings.outDir;
  fitPath($("out-path"), state.settings.outDir);
}

document.querySelectorAll(".seg").forEach((seg) => {
  seg.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    state.settings[seg.dataset.key] = b.dataset.v;
    saveSettings();
    renderSettings();
    refreshCapacity();
  });
});
$("opt-cursor").addEventListener("change", (e) => {
  state.settings.cursor = e.target.checked;
  saveSettings();
});
$("opt-hide").addEventListener("change", (e) => {
  state.settings.hide = e.target.checked;
  saveSettings();
  invoke("set_hide_self", { hide: state.settings.hide });
});
$("btn-dir").addEventListener("click", async () => {
  const dir = await invoke("pick_folder", { current: state.settings.outDir });
  if (dir) {
    state.settings.outDir = dir;
    saveSettings();
    renderSettings();
    refreshCapacity();
  }
});
$("out-path").addEventListener("click", () => invoke("reveal", { path: state.settings.outDir }));
$("btn-open-dir").addEventListener("click", () => invoke("reveal", { path: state.settings.outDir }));

// ---------------------------------------------------------------- 범위
const KIND_LABEL = { window: "창", area: "영역", monitor: "전체 화면" };
let monitors = [];

// 모니터 배치도: 모든 모니터를 비율대로 그리고 선택 범위를 겹쳐 표시
function renderMap() {
  const map = $("map");
  map.querySelectorAll(".mon").forEach((el) => el.remove());
  if (!monitors.length) return;
  const x0 = Math.min(...monitors.map((m) => m.rect.x)), y0 = Math.min(...monitors.map((m) => m.rect.y));
  const x1 = Math.max(...monitors.map((m) => m.rect.x + m.rect.w)), y1 = Math.max(...monitors.map((m) => m.rect.y + m.rect.h));
  const s = Math.min(312 / (x1 - x0), 104 / (y1 - y0));
  const px = (v) => Math.round(v * s) + "px";
  map.style.width = px(x1 - x0);
  map.style.height = px(y1 - y0);

  const r = state.region || { x: -1e9, y: -1e9, w: 0, h: 0 }, cx = r.x + r.w / 2, cy = r.y + r.h / 2;
  const GAP = 2; // 모니터 사이 간격(px)
  monitors.forEach((m, i) => {
    const el = document.createElement("div");
    const hit = cx >= m.rect.x && cx < m.rect.x + m.rect.w && cy >= m.rect.y && cy < m.rect.y + m.rect.h;
    el.className = "mon" + (hit ? " active" : "");
    el.style.left = `calc(${px(m.rect.x - x0)} + ${GAP}px)`;
    el.style.top = `calc(${px(m.rect.y - y0)} + ${GAP}px)`;
    el.style.width = `calc(${px(m.rect.w)} - ${GAP * 2}px)`;
    el.style.height = `calc(${px(m.rect.h)} - ${GAP * 2}px)`;
    el.innerHTML = `<b>${i + 1}</b>`;
    el.title = `${m.name} 전체 녹화`;
    el.addEventListener("click", () => {
      if (state.recording || state.saving) return;
      const r = m.rect;
      setRegion({ rect: { x: r.x, y: r.y, w: r.w - (r.w % 2), h: r.h - (r.h % 2) }, label: m.name, kind: "monitor" });
    });
    map.prepend(el);
  });
  const f = $("frame");
  f.style.left = px(r.x - x0);
  f.style.top = px(r.y - y0);
  f.style.width = `max(10px, ${px(r.w)})`;
  f.style.height = `max(8px, ${px(r.h)})`;
}

function renderRegion() {
  document.body.classList.toggle("has-region", !!state.region);
  if (!state.region) return renderMap();
  $("dims").textContent = `${state.region.w} × ${state.region.h}`;
  $("src-kind").textContent = KIND_LABEL[state.kind] || "영역";
  $("src-title").textContent = state.label;
  renderMap();
}

async function setRegion(sel) {
  state.region = sel.rect;
  state.kind = sel.kind;
  state.label = sel.label;
  monitors = await invoke("monitors"); // 모니터 구성이 바뀌었을 수 있으니 매번 갱신
  renderRegion();
  refreshCapacity();
  invoke("set_border", { rect: sel.rect, recording: false, flash: true });
}

const openSelector = () => {
  if (state.recording || state.saving) return;
  invoke("open_selector");
};
$("btn-pick").addEventListener("click", openSelector);
// 전체 화면 → 모니터 목록에서 선택
const menu = $("mon-menu");
const closeMenu = () => document.body.classList.remove("menu-open");
let hereRect = null; // 이 창이 있는 모니터
function renderMenu() {
  const same = (a, b) => a && b && a.x === b.x && a.y === b.y;
  menu.innerHTML = "";
  monitors.forEach((m, i) => {
    const r = m.rect;
    const k = Math.min(44 / r.w, 28 / r.h); // 모니터 비율대로 작은 화면 그림
    const selected = state.kind === "monitor" && same(state.region, r);
    const b = document.createElement("button");
    b.className = "menu-item" + (selected ? " current" : "");
    b.innerHTML = `
      <span class="mi-screen"><span style="width:${Math.round(r.w * k)}px;height:${Math.round(r.h * k)}px">${i + 1}</span></span>
      <span class="mi-text"><span class="mi-name">${m.name}</span><span class="mi-sub">${r.w} × ${r.h}</span></span>
      ${same(hereRect, r) ? '<span class="mi-badge">이 창이 있는 화면</span>' : ""}
      ${selected ? '<svg class="mi-check" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>' : ""}`;
    b.addEventListener("click", () => {
      closeMenu();
      setRegion({ rect: { x: r.x, y: r.y, w: r.w - (r.w % 2), h: r.h - (r.h % 2) }, label: m.name, kind: "monitor" });
    });
    menu.appendChild(b);
  });
}
$("btn-full").addEventListener("click", (e) => {
  e.stopPropagation();
  if (state.recording || state.saving) return;
  if (document.body.classList.contains("menu-open")) return closeMenu();
  renderMenu(); // 캐시된 목록으로 즉시 열고
  document.body.classList.add("menu-open");
  Promise.all([invoke("monitors"), invoke("fullscreen_region")]).then(([ms, here]) => {
    monitors = ms; // 최신 정보로 갱신
    hereRect = here && here.rect;
    if (document.body.classList.contains("menu-open")) renderMenu();
  });
});
menu.addEventListener("click", (e) => e.stopPropagation());
document.addEventListener("click", closeMenu);
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });

listen("region-selected", ({ payload }) => {
  if (payload) setRegion(payload);
  else if (state.region) invoke("set_border", { rect: state.region, recording: false }); // 취소 → 기존 범위 복원
});

// ---------------------------------------------------------------- 남은 녹화 가능 시간
// 녹화 전: 해상도·프레임·화질로 추정 / 녹화 중: 파일이 실제로 늘어나는 속도로 계산
const BPP = { 18: 0.09, 23: 0.05, 28: 0.028 }; // 화면 녹화 평균 비트/픽셀/프레임 (보수적으로 잡은 값)
const MIN_FREE = 300 * 1024 * 1024; // 이 아래로 떨어지면 자동 정지
let measured = null; // 녹화 중 실측 { size, t, rate(바이트/초) }

function estimatedRate() {
  const r = state.region || { w: 1920, h: 1080 };
  return (r.w * r.h * Number(state.settings.fps) * (BPP[state.settings.crf] || 0.05)) / 8;
}

function fmtDuration(sec) {
  if (!isFinite(sec) || sec > 99 * 3600) return "100시간 이상";
  if (sec < 60) return "1분 미만";
  const h = Math.floor(sec / 3600), m = Math.floor(sec / 60) % 60;
  if (h >= 10) return `${h}시간`;
  return h ? `${h}시간${m ? ` ${m}분` : ""}` : `${m}분`;
}
function fmtBytes(b) {
  const gb = b / 1024 ** 3;
  return gb >= 100 ? `${Math.round(gb)}GB` : gb >= 1 ? `${gb.toFixed(1)}GB` : `${Math.round(b / 1024 ** 2)}MB`;
}

async function refreshCapacity() {
  const free = await invoke("disk_free", { path: state.settings.outDir });
  const chip = $("remain-idle");
  if (free == null) { chip.textContent = ""; return; }
  const sec = (free - MIN_FREE) / estimatedRate();
  chip.textContent = `${fmtBytes(free)} · 약 ${fmtDuration(sec)}`;
  chip.title = `디스크 여유 공간 ${fmtBytes(free)}
현재 범위·프레임·화질 기준으로 약 ${fmtDuration(sec)} 녹화할 수 있어요.
화면 변화가 많으면 더 짧아질 수 있어요.`;
  chip.classList.toggle("low", sec < 600);
}

async function updateLiveCapacity() {
  const [size, free] = await Promise.all([
    invoke("recording_size"),
    invoke("disk_free", { path: state.settings.outDir }),
  ]);
  if (!state.recording || free == null) return;
  const now = Date.now();
  if (size != null) {
    if (measured) {
      const inst = Math.max(0, (size - measured.size) / ((now - measured.t) / 1000));
      // 지수 이동 평균으로 흔들림 완화
      measured = { size, t: now, rate: measured.rate == null ? inst : measured.rate * 0.8 + inst * 0.2 };
    } else measured = { size, t: now, rate: null };
  }
  // 처음 몇 초는 인코더 버퍼 때문에 실측이 부정확 → 추정치 사용
  const elapsed = (now - state.startedAt) / 1000;
  // 정지된 화면은 용량이 거의 안 늘어 지나치게 낙관적으로 나오므로 추정치의 30%를 하한으로 둠
  const rate = measured && measured.rate && elapsed > 8 ? Math.max(measured.rate, estimatedRate() * 0.3) : estimatedRate();
  const sec = (free - MIN_FREE) / rate;
  const el = $("remain-live");
  el.textContent = `남은 공간으로 약 ${fmtDuration(sec)} 더 녹화 가능`;
  el.classList.toggle("low", sec < 600);
  if (free < MIN_FREE) {
    await stop();
    showToast("err", "저장 공간이 부족해 녹화를 멈췄어요", `남은 공간 ${fmtBytes(free)}`);
  }
}

// ---------------------------------------------------------------- 녹화
let tick = null;
let aliveCheck = null;
let capTimer = null;

function setMode() {
  document.body.classList.toggle("recording", state.recording);
  document.body.classList.toggle("saving", state.saving);
  const busy = state.recording || state.saving;
  $("btn-pick").disabled = busy;
  $("btn-full").disabled = busy;
}

function fmt(sec) {
  const p = (n) => String(n).padStart(2, "0");
  return `${p(Math.floor(sec / 3600))}:${p(Math.floor(sec / 60) % 60)}:${p(sec % 60)}`;
}

async function start() {
  if (!state.region) return openSelector();
  try {
    await invoke("start_recording", {
      opts: {
        rect: state.region,
        fps: Number(state.settings.fps),
        crf: Number(state.settings.crf),
        cursor: state.settings.cursor,
        out_dir: state.settings.outDir,
      },
    });
  } catch (err) {
    return showToast("err", "녹화를 시작할 수 없어요", String(err));
  }
  state.recording = true;
  state.startedAt = Date.now();
  hideToast();
  setMode();
  invoke("set_border", { rect: state.region, recording: true });
  $("timer").textContent = "00:00:00";
  tick = setInterval(() => {
    $("timer").textContent = fmt(Math.floor((Date.now() - state.startedAt) / 1000));
  }, 250);
  aliveCheck = setInterval(async () => {
    if (state.recording && !(await invoke("recording_alive"))) stop();
  }, 1000);
  measured = null;
  $("remain-live").textContent = "남은 시간 계산 중…";
  $("remain-live").classList.remove("low");
  updateLiveCapacity();
  capTimer = setInterval(updateLiveCapacity, 2000);
}

async function stop() {
  if (!state.recording) return;
  clearInterval(tick);
  clearInterval(aliveCheck);
  clearInterval(capTimer);
  state.recording = false;
  state.saving = true;
  setMode();
  invoke("set_border", { rect: state.region, recording: false });

  let res;
  try {
    res = await invoke("stop_recording");
  } catch (err) {
    res = { path: null, error: String(err) };
  }
  state.saving = false;
  setMode();
  refreshCapacity();

  if (res.path) {
    const name = res.path.split(/[\\/]/).pop();
    showToast(res.error ? "err" : "ok", res.error ? "녹화가 중간에 멈췄어요" : "저장했어요", name, res.path);
  } else {
    showToast("err", "녹화된 내용이 없어요", (res.error || "").trim().split("\n").pop());
  }
}

const toggle = () => (capturing ? null : state.recording ? stop() : state.saving ? null : start());
$("btn-rec").addEventListener("click", toggle);
listen("toggle-record", toggle);

// ---------------------------------------------------------------- 녹화 단축키
// 형식: Tauri 전역 단축키 문자열 ("Ctrl+Shift+R", "F9" …)
let capturing = false;
const HK_SUB = "다른 프로그램을 쓰는 중에도 동작해요";
const SOLO_OK = /^(F\d{1,2}|Pause|ScrollLock|PrintScreen|Insert)$/; // 조합 없이 단독으로 써도 되는 키

const hkParts = (accel) => accel.split("+").map((p) => (p === "Control" ? "Ctrl" : p));
function renderHotkey() {
  if (capturing) return;
  const parts = hkParts(state.settings.hotkey);
  $("hk-btn").innerHTML = parts.map((p) => `<kbd>${p}</kbd>`).join('<span class="plus">+</span>');
  $("rec-kbd").textContent = parts.join("+");
}
function setHkSub(text, warn) {
  $("hk-sub").textContent = text;
  $("hk-sub").classList.toggle("warn", !!warn);
}

function keyName(e) {
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^F\d{1,2}$/.test(c)) return c;
  const map = { Space: "Space", Enter: "Enter", Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/", Backslash: "\\", Insert: "Insert", Home: "Home",
    End: "End", PageUp: "PageUp", PageDown: "PageDown", Pause: "Pause", ScrollLock: "ScrollLock", PrintScreen: "PrintScreen",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right" };
  if (map[c]) return map[c];
  if (/^Numpad\d$/.test(c)) return "Num" + c.slice(6);
  return null;
}

async function beginCapture() {
  if (capturing || state.recording || state.saving) return;
  capturing = true;
  await invoke("set_hotkey", { accel: null }); // 지금 단축키가 눌려도 녹화가 시작되지 않게 잠시 해제
  $("hk-btn").classList.add("capturing");
  $("hk-btn").innerHTML = '<span class="wait">키 조합을 누르세요…</span>';
  setHkSub("ESC: 취소", false);
}
async function endCapture(accel) {
  if (!capturing) return;
  capturing = false;
  $("hk-btn").classList.remove("capturing");
  const prev = state.settings.hotkey;
  if (accel) {
    try {
      await invoke("set_hotkey", { accel });
      state.settings.hotkey = accel;
      saveSettings();
      setHkSub(HK_SUB);
    } catch (err) {
      await invoke("set_hotkey", { accel: prev }).catch(() => {});
      setHkSub(`${accel.replace(/\+/g, " + ")}: 다른 프로그램이 쓰고 있어요`, true);
    }
  } else {
    await invoke("set_hotkey", { accel: prev }).catch(() => {});
    setHkSub(HK_SUB);
  }
  renderHotkey();
}

$("hk-btn").addEventListener("click", (e) => { e.stopPropagation(); beginCapture(); });
document.addEventListener("click", () => capturing && endCapture(null));
window.addEventListener("blur", () => capturing && endCapture(null));
document.addEventListener("keydown", (e) => {
  if (!capturing) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") return endCapture(null);
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return; // 조합 키만 누른 상태 → 계속 대기
  const key = keyName(e);
  if (!key) return setHkSub("이 키는 쓸 수 없어요. 다른 키를 눌러 주세요", true);
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
  if (!mods.length && !SOLO_OK.test(key)) return setHkSub("Ctrl · Alt · Shift 중 하나와 함께 눌러 주세요", true);
  endCapture([...mods, key].join("+"));
}, true);

// ---------------------------------------------------------------- 토스트
let toastTimer = null;
let toastPath = null;
const ICONS = {
  ok: '<svg viewBox="0 0 12 12"><path d="M2.5 6.2l2.3 2.3 4.7-5" /></svg>',
  err: '<svg viewBox="0 0 12 12"><path d="M6 3v3.5M6 8.8v.1" /></svg>',
};
function showToast(kind, title, sub, path) {
  $("toast-icon").className = "toast-icon " + kind;
  $("toast-icon").innerHTML = ICONS[kind];
  $("toast-title").textContent = title;
  $("toast-sub").textContent = sub || "";
  toastPath = path || null;
  $("toast-act").style.display = path ? "" : "none";
  $("toast-folder").style.display = path ? "" : "none";
  $("toast").classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(hideToast, 8000);
}
function hideToast() {
  $("toast").classList.remove("show");
}
$("toast-act").addEventListener("click", () => {
  if (toastPath) invoke("open_file", { path: toastPath });
  hideToast();
});
$("toast-folder").addEventListener("click", () => {
  if (toastPath) invoke("reveal", { path: toastPath }); // 탐색기에서 파일 선택된 상태로 열림
  hideToast();
});

// ---------------------------------------------------------------- 테마 (시스템 → 라이트 → 다크)
const THEMES = ["system", "light", "dark"];
const THEME_UI = {
  system: ['시스템 설정 따름', '<svg viewBox="0 0 24 24"><rect x="3" y="4.5" width="18" height="12" rx="1.5"/><path d="M9 20h6M12 16.5V20"/></svg>'],
  light: ['라이트 모드', '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M4.6 4.6l1.4 1.4M18 18l1.4 1.4M2.5 12h2M19.5 12h2M4.6 19.4L6 18M18 6l1.4-1.4"/></svg>'],
  dark: ['다크 모드', '<svg viewBox="0 0 24 24"><path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/></svg>'],
};
const mqLight = window.matchMedia("(prefers-color-scheme: light)");
function applyTheme() {
  const t = state.settings.theme;
  document.documentElement.dataset.theme = t === "system" ? (mqLight.matches ? "light" : "dark") : t;
  $("btn-theme").innerHTML = THEME_UI[t][1];
  $("btn-theme").title = THEME_UI[t][0];
}
mqLight.addEventListener("change", applyTheme);
$("btn-theme").addEventListener("click", () => {
  const t = state.settings.theme;
  state.settings.theme = THEMES[(THEMES.indexOf(t) + 1) % THEMES.length];
  saveSettings();
  applyTheme();
});

// ---------------------------------------------------------------- 창 제어
$("btn-min").addEventListener("click", () => appWindow.minimize());
$("btn-close").addEventListener("click", async () => {
  if (state.recording) await stop();
  appWindow.close();
});
document.addEventListener("contextmenu", (e) => e.preventDefault());

// ---------------------------------------------------------------- 시작
(async function init() {
  loadSettings();
  invoke("monitors").then((ms) => { monitors = ms; renderRegion(); });
  setInterval(() => !state.recording && refreshCapacity(), 30000);
  if (!state.settings.outDir) state.settings.outDir = await invoke("default_out_dir");
  if (!THEMES.includes(state.settings.theme)) state.settings.theme = "system";
  saveSettings();
  applyTheme();
  renderSettings();
  renderHotkey();
  invoke("set_hotkey", { accel: state.settings.hotkey }).catch(() =>
    setHkSub(`${state.settings.hotkey}: 다른 프로그램이 쓰고 있어요. 다른 키로 바꿔 주세요`, true));
  renderRegion();
  setMode();
  invoke("set_hide_self", { hide: state.settings.hide });
  refreshCapacity();
})();
document.fonts.ready.then(() => fitPath($("out-path"), state.settings.outDir)); // 글꼴 로드 후 다시 맞춤
