use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const BORDER: i32 = 4; // 테두리 두께(물리 픽셀)

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Clone, Serialize)]
pub struct WinInfo {
    title: String,
    rect: Rect,
}

#[derive(Clone, Serialize)]
struct Selection {
    rect: Rect,
    label: String,
    kind: String, // "window" | "area" | "monitor"
}

#[derive(Clone, Serialize)]
struct MonInfo {
    name: String,
    rect: Rect,
}

#[derive(Serialize)]
struct SelectorData {
    windows: Vec<WinInfo>,
    monitors: Vec<MonInfo>,
}

#[derive(Clone, Serialize)]
struct BorderState {
    recording: bool,
    flash: bool,
}

struct Recording {
    child: Child,
    mkv: PathBuf,
    log: PathBuf,
}

#[derive(Default)]
struct AppState {
    windows: Mutex<Vec<WinInfo>>,
    hotkey: Mutex<Option<String>>,
    mode_floor: Mutex<usize>, // 이 순서부터 녹화 방식을 시도 (MODES 인덱스)
    rec: Mutex<Option<Recording>>,
}

// ------------------------------------------------------------------ Win32
mod win {
    use super::{Rect, WinInfo};
    use std::ffi::c_void;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::*;

    unsafe extern "system" fn enum_cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut Vec<WinInfo>);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return TRUE;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TRANSPARENT.0 != 0 {
            return TRUE; // 클릭이 통과되는 오버레이류
        }
        let mut cloaked = 0i32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut c_void,
            4,
        );
        if cloaked != 0 {
            return TRUE; // 다른 가상 데스크톱 / 숨겨진 UWP 창
        }
        // 바탕화면(Progman/WorkerW)은 제외 → 선택 화면에서 모니터 전체로 처리
        let mut cls = [0u16; 32];
        let cn = GetClassNameW(hwnd, &mut cls);
        let cls = String::from_utf16_lossy(&cls[..cn.max(0) as usize]);
        if cls == "Progman" || cls == "WorkerW" {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len == 0 {
            return TRUE;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

        let mut r = RECT::default();
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
        {
            let _ = GetWindowRect(hwnd, &mut r);
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w >= 20 && h >= 20 {
            out.push(WinInfo { title, rect: Rect { x: r.left, y: r.top, w, h } });
        }
        TRUE
    }

    /// 보이는 최상위 창 목록 (Z 순서: 위 → 아래, 물리 픽셀)
    pub fn list_windows() -> Vec<WinInfo> {
        let mut out: Vec<WinInfo> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&mut out as *mut _ as isize));
        }
        out
    }

    /// 경로가 있는 드라이브의 사용 가능 공간(바이트). 폴더가 아직 없으면 상위 폴더 기준
    pub fn disk_free(path: &std::path::Path) -> Option<u64> {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut p = path;
        while !p.exists() {
            p = p.parent()?;
        }
        let mut free = 0u64;
        unsafe {
            GetDiskFreeSpaceExW(&HSTRING::from(p.as_os_str()), Some(&mut free), None, None).ok()?;
        }
        Some(free)
    }

    pub fn virtual_screen() -> Rect {
        unsafe {
            Rect {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    pub fn monitor_at_cursor() -> Rect {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(hmon, &mut mi);
            let r = mi.rcMonitor;
            Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
        }
    }

    /// 앱이 어떻게 종료되든(강제 종료 포함) 자식 프로세스도 함께 종료되도록 Job에 묶음
    pub fn kill_with_parent(child: &std::process::Child) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::*;
        unsafe {
            let Ok(job) = CreateJobObjectW(None, None) else { return };
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()));
            // job 핸들은 일부러 닫지 않음: 앱 프로세스가 끝날 때 닫히면서 ffmpeg도 종료됨
        }
    }

    /// 기본 GPU(어댑터 0)에 연결된 모니터들의 좌표. 순서 = ddagrab의 output_idx
    pub fn dxgi_outputs() -> Vec<Rect> {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let mut out = Vec::new();
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
            let Ok(adapter) = factory.EnumAdapters1(0) else { return out };
            let mut i = 0;
            while let Ok(o) = adapter.EnumOutputs(i) {
                if let Ok(d) = o.GetDesc() {
                    let r = d.DesktopCoordinates;
                    out.push(Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top });
                }
                i += 1;
            }
        }
        out
    }

    /// 화면 캡처(녹화)에서 이 창을 제외 (Windows 10 2004+)
    pub fn exclude_from_capture(hwnd: *mut c_void, exclude: bool) {
        let aff = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        unsafe {
            let _ = SetWindowDisplayAffinity(HWND(hwnd), aff);
        }
    }
}

fn exclude(w: &WebviewWindow, on: bool) {
    // SR_NO_EXCLUDE=1 : 스크린샷 확인용으로 캡처 제외 끄기
    if std::env::var_os("SR_NO_EXCLUDE").is_some() {
        return;
    }
    if let Ok(h) = w.hwnd() {
        win::exclude_from_capture(h.0 as *mut _, on);
    }
}

/// 가상 화면 안으로 자르고 H.264용 짝수 크기로 보정
fn normalize(r: Rect) -> Option<Rect> {
    let v = win::virtual_screen();
    let x1 = r.x.max(v.x);
    let y1 = r.y.max(v.y);
    let x2 = (r.x + r.w).min(v.x + v.w);
    let y2 = (r.y + r.h).min(v.y + v.h);
    let (mut w, mut h) = (x2 - x1, y2 - y1);
    w -= w % 2;
    h -= h % 2;
    (w >= 16 && h >= 16).then_some(Rect { x: x1, y: y1, w, h })
}

/// 모니터 목록 (왼쪽 → 오른쪽 순으로 "모니터 1, 2, …")
fn monitor_list(app: &AppHandle) -> Vec<MonInfo> {
    let Some(main) = app.get_webview_window("main") else { return vec![] };
    let mut ms: Vec<Rect> = main
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Rect {
            x: m.position().x,
            y: m.position().y,
            w: m.size().width as i32,
            h: m.size().height as i32,
        })
        .collect();
    ms.sort_by_key(|r| (r.x, r.y));
    ms.into_iter()
        .enumerate()
        .map(|(i, rect)| MonInfo { name: format!("모니터 {}", i + 1), rect })
        .collect()
}

fn ffmpeg_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let p = exe.parent().unwrap().join("ffmpeg.exe");
    if p.exists() {
        Ok(p)
    } else {
        Err(format!("ffmpeg를 찾을 수 없습니다: {}", p.display()))
    }
}

fn log_tail(path: &PathBuf) -> String {
    let mut s = String::new();
    if let Ok(mut f) = File::open(path) {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = f.seek(SeekFrom::Start(len.saturating_sub(800)));
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf);
        s = String::from_utf8_lossy(&buf).into_owned();
    }
    s
}

// ------------------------------------------------------------------ 영역 선택
// Windows에서는 동기 커맨드 안에서 창을 만들면 교착 상태가 되므로 async 필수
#[tauri::command]
async fn open_selector(app: AppHandle) -> Result<(), String> {
    *app.state::<AppState>().windows.lock().unwrap() = win::list_windows();

    let main = app.get_webview_window("main").unwrap();
    if let Some(b) = app.get_webview_window("border") {
        let _ = b.hide();
    }
    let monitors = main.available_monitors().map_err(|e| e.to_string())?;
    let _ = main.hide();

    let cursor = win::monitor_at_cursor();
    for (i, m) in monitors.iter().enumerate() {
        let pos = *m.position();
        let size = *m.size();
        let label = format!("sel-{i}");
        if let Some(old) = app.get_webview_window(&label) {
            let _ = old.destroy();
        }
        let url = format!("selector.html?ox={}&oy={}", pos.x, pos.y);
        let w = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
            .title("영역 선택")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?;
        let _ = w.set_position(PhysicalPosition::new(pos.x, pos.y));
        let _ = w.set_size(PhysicalSize::new(size.width, size.height));
        exclude(&w, true);
        let _ = w.show();
        if pos.x == cursor.x && pos.y == cursor.y {
            let _ = w.set_focus();
        }
    }
    Ok(())
}

#[tauri::command]
fn selector_data(app: AppHandle, state: State<AppState>) -> SelectorData {
    SelectorData {
        windows: state.windows.lock().unwrap().clone(),
        monitors: monitor_list(&app),
    }
}

#[tauri::command]
fn monitors(app: AppHandle) -> Vec<MonInfo> {
    monitor_list(&app)
}

#[tauri::command]
async fn finish_selection(app: AppHandle, rect: Option<Rect>, label: String, kind: String) {
    for (l, w) in app.webview_windows() {
        if l.starts_with("sel-") {
            let _ = w.destroy();
        }
    }
    let main = app.get_webview_window("main").unwrap();
    let _ = main.show();
    let _ = main.set_focus();
    let sel = rect.and_then(normalize).map(|rect| Selection { rect, label, kind });
    let _ = app.emit_to("main", "region-selected", sel);
}

#[tauri::command]
fn fullscreen_region(app: AppHandle) -> Option<Selection> {
    let cur = win::monitor_at_cursor();
    let label = monitor_list(&app)
        .into_iter()
        .find(|m| m.rect.x == cur.x && m.rect.y == cur.y)
        .map(|m| m.name)
        .unwrap_or_else(|| "모니터".into());
    normalize(cur).map(|rect| Selection { rect, label, kind: "monitor".into() })
}

// ------------------------------------------------------------------ 테두리 / 캡처 제외
#[tauri::command]
fn set_border(app: AppHandle, rect: Option<Rect>, recording: bool, flash: Option<bool>) {
    let Some(b) = app.get_webview_window("border") else { return };
    match rect {
        None => {
            let _ = b.hide();
        }
        Some(r) => {
            // 기본은 영역 바깥에 테두리. 모니터 가장자리에 닿는 변은 잘려서
            // 영역 안쪽에 그려짐 (테두리 창은 녹화에서 제외되므로 영상엔 안 찍힘)
            let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
            let mon = monitor_list(&app)
                .into_iter()
                .map(|m| m.rect)
                .find(|m| cx >= m.x && cx < m.x + m.w && cy >= m.y && cy < m.y + m.h)
                .unwrap_or_else(win::virtual_screen);
            let x1 = (r.x - BORDER).max(mon.x);
            let y1 = (r.y - BORDER).max(mon.y);
            let x2 = (r.x + r.w + BORDER).min(mon.x + mon.w);
            let y2 = (r.y + r.h + BORDER).min(mon.y + mon.h);
            let _ = b.set_position(PhysicalPosition::new(x1, y1));
            let _ = b.set_size(PhysicalSize::new((x2 - x1).max(1) as u32, (y2 - y1).max(1) as u32));
            let _ = b.emit_to(
                "border",
                "border-state",
                BorderState { recording, flash: flash.unwrap_or(false) },
            );
            let _ = b.show();
        }
    }
}

#[tauri::command]
fn set_hide_self(app: AppHandle, hide: bool) {
    if let Some(m) = app.get_webview_window("main") {
        exclude(&m, hide);
    }
}

// ------------------------------------------------------------------ 녹화
#[derive(Deserialize)]
struct StartOpts {
    rect: Rect,
    fps: u32,
    crf: u32,
    cursor: bool,
    out_dir: String,
}

/// 녹화 방식 (앞에서부터 시도하고, 바로 실패하면 다음 방식으로)
/// - DdaQsv : GPU 캡처(ddagrab) + Intel Quick Sync 인코딩. 프레임이 GPU를 벗어나지 않아 CPU 부하가 가장 적음
/// - DdaX264: GPU 캡처 + CPU 인코딩(libx264)
/// - Gdi    : gdigrab + libx264. 범위가 여러 모니터에 걸치거나 GPU 캡처를 쓸 수 없을 때
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    DdaQsv,
    DdaX264,
    Gdi,
}
const MODES: [Mode; 3] = [Mode::DdaQsv, Mode::DdaX264, Mode::Gdi];

fn ffmpeg_args(o: &StartOpts, mode: Mode) -> Option<Vec<String>> {
    let r = o.rect;
    let (fps, mouse) = (o.fps, if o.cursor { "1" } else { "0" });
    let mut a: Vec<String> = Vec::new();
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));

    if mode == Mode::Gdi {
        push(&["-f", "gdigrab", "-framerate", &fps.to_string(), "-draw_mouse", mouse]);
        push(&["-offset_x", &r.x.to_string(), "-offset_y", &r.y.to_string()]);
        push(&["-video_size", &format!("{}x{}", r.w, r.h), "-i", "desktop"]);
    } else {
        // 범위를 완전히 포함하는 모니터(기본 GPU에 연결된 것)만 ddagrab으로 녹화 가능
        let (idx, out) = win::dxgi_outputs().into_iter().enumerate().find(|(_, m)| {
            r.x >= m.x && r.y >= m.y && r.x + r.w <= m.x + m.w && r.y + r.h <= m.y + m.h
        })?;
        let tail = if mode == Mode::DdaQsv {
            // setparams: 프레임에 BT.709 · limited 정보를 붙이면 QSV가 그 기준으로 RGB→YUV 변환하고 파일에도 같게 표기됨
            "hwmap=derive_device=qsv,format=qsv,setparams=range=tv:colorspace=bt709:color_primaries=bt709:color_trc=bt709"
        } else {
            "hwdownload,format=bgra"
        };
        let graph = format!(
            "ddagrab=output_idx={idx}:framerate={fps}:draw_mouse={mouse}:offset_x={}:offset_y={}:video_size={}x{},{tail}",
            r.x - out.x, r.y - out.y, r.w, r.h
        );
        push(&["-filter_complex", &graph]);
    }

    if mode == Mode::DdaQsv {
        // 영상 스트림 안의 색 정보(VUI)도 BT.709 · limited 로 맞춤 (틀리면 플레이어에서 색이 바래 보임)
        push(&["-c:v", "h264_qsv", "-preset", "veryfast"]);
        push(&["-global_quality", &(o.crf + 1).to_string()]);
        push(&["-bsf:v", "h264_metadata=video_full_range_flag=0:colour_primaries=1:transfer_characteristics=1:matrix_coefficients=1"]);
    } else {
        push(&["-c:v", "libx264", "-preset", "superfast", "-crf", &o.crf.to_string(), "-pix_fmt", "yuv420p"]);
    }
    Some(a)
}

fn spawn_ffmpeg(args: &[String], mkv: &PathBuf, log: &PathBuf) -> Result<Child, String> {
    let log_file = File::create(log).map_err(|e| e.to_string())?;
    let child = Command::new(ffmpeg_path()?)
        .args(["-hide_banner", "-loglevel", "warning", "-y"])
        .args(args)
        .arg(mkv)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log_file)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("ffmpeg 실행 실패: {e}"))?;
    win::kill_with_parent(&child);
    Ok(child)
}

#[tauri::command]
async fn start_recording(app: AppHandle, opts: StartOpts) -> Result<String, String> {
    if app.state::<AppState>().rec.lock().unwrap().is_some() {
        return Err("이미 녹화 중입니다.".into());
    }
    let dir = PathBuf::from(&opts.out_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("저장 폴더를 만들 수 없습니다: {e}"))?;

    let name = format!("녹화_{}", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    // 비정상 종료돼도 살아남도록 MKV로 녹화 → 정지 시 MP4로 변환
    let mkv = dir.join(format!("{name}.mkv"));
    let log = dir.join(format!("{name}.log"));
    // 이전에 실패한 방식은 이번 실행 동안 건너뜀
    let first = *app.state::<AppState>().mode_floor.lock().unwrap();

    let (child, mode, mkv, log) = tauri::async_runtime::spawn_blocking(move || {
        let mut last_err = String::from("녹화를 시작할 수 없습니다.");
        for (i, &mode) in MODES.iter().enumerate().skip(first) {
            let Some(args) = ffmpeg_args(&opts, mode) else { continue };
            let mut child = match spawn_ffmpeg(&args, &mkv, &log) {
                Ok(c) => c,
                Err(e) => {
                    last_err = e;
                    continue;
                }
            };
            if mode == Mode::Gdi {
                return Ok((child, i, mkv, log)); // 마지막 수단
            }
            // 지원하지 않는 방식이면 ffmpeg가 곧바로 종료됨
            std::thread::sleep(Duration::from_millis(1200));
            match child.try_wait() {
                Ok(None) => return Ok((child, i, mkv, log)),
                _ => last_err = log_tail(&log),
            }
        }
        Err(last_err)
    })
    .await
    .map_err(|e| e.to_string())??;

    let state = app.state::<AppState>();
    // 범위 때문에 건너뛴 게 아니라 실제로 실패한 방식만 다음부터 제외
    let mut floor = state.mode_floor.lock().unwrap();
    if MODES[mode] != Mode::Gdi {
        *floor = mode;
    }
    let path = mkv.to_string_lossy().into_owned();
    *state.rec.lock().unwrap() = Some(Recording { child, mkv, log });
    Ok(path)
}

#[derive(Serialize)]
struct StopResult {
    path: Option<String>,
    error: Option<String>,
}

#[tauri::command]
async fn stop_recording(app: AppHandle) -> Result<StopResult, String> {
    let taken = app.state::<AppState>().rec.lock().unwrap().take();
    let Some(mut rec) = taken else {
        return Ok(StopResult { path: None, error: None });
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut error = None;
        match rec.child.try_wait() {
            Ok(Some(_)) => error = Some(log_tail(&rec.log)), // 이미 죽어 있었음
            _ => {
                if let Some(mut stdin) = rec.child.stdin.take() {
                    let _ = stdin.write_all(b"q");
                    let _ = stdin.flush();
                }
                let t = Instant::now();
                loop {
                    if let Ok(Some(_)) = rec.child.try_wait() {
                        break;
                    }
                    if t.elapsed() > Duration::from_secs(20) {
                        let _ = rec.child.kill();
                        let _ = rec.child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let has_video = std::fs::metadata(&rec.mkv).map(|m| m.len() > 0).unwrap_or(false);
        if !has_video {
            return StopResult { path: None, error: error.or(Some(log_tail(&rec.log))) };
        }
        let mp4 = rec.mkv.with_extension("mp4");
        let ok = ffmpeg_path()
            .ok()
            .and_then(|ff| {
                Command::new(ff)
                    .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                    .arg(&rec.mkv)
                    .args(["-c", "copy", "-movflags", "+faststart"])
                    .arg(&mp4)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .status()
                    .ok()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        let path = if ok {
            let _ = std::fs::remove_file(&rec.mkv);
            let _ = std::fs::remove_file(&rec.log);
            mp4
        } else {
            rec.mkv.clone()
        };
        StopResult { path: Some(path.to_string_lossy().into_owned()), error }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn disk_free(path: String) -> Option<u64> {
    win::disk_free(std::path::Path::new(&path))
}

/// 녹화 중인 파일의 현재 크기(바이트)
#[tauri::command]
fn recording_size(state: State<AppState>) -> Option<u64> {
    let rec = state.rec.lock().unwrap();
    // 쓰는 중인 파일은 디렉터리 정보가 늦게 갱신되므로 핸들을 열어 실제 크기를 읽음
    rec.as_ref()
        .and_then(|r| File::open(&r.mkv).and_then(|f| f.metadata()).ok())
        .map(|m| m.len())
}

/// 녹화 프로세스가 예기치 않게 종료됐는지 확인
#[tauri::command]
fn recording_alive(state: State<AppState>) -> bool {
    let mut rec = state.rec.lock().unwrap();
    match rec.as_mut() {
        Some(r) => matches!(r.child.try_wait(), Ok(None)),
        None => false,
    }
}

// ------------------------------------------------------------------ 단축키
/// 녹화 단축키 변경. None이면 일시 해제(새 키 입력받는 동안).
/// 등록에 실패하면 이전 단축키를 다시 등록하고 오류를 돌려줌.
#[tauri::command]
fn set_hotkey(app: AppHandle, state: State<AppState>, accel: Option<String>) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let Some(accel) = accel else { return Ok(()) };
    match gs.register(accel.as_str()) {
        Ok(()) => {
            *state.hotkey.lock().unwrap() = Some(accel);
            Ok(())
        }
        Err(e) => {
            if let Some(old) = state.hotkey.lock().unwrap().clone() {
                let _ = gs.register(old.as_str());
            }
            Err(e.to_string())
        }
    }
}

// ------------------------------------------------------------------ 기타
#[tauri::command]
fn default_out_dir(app: AppHandle) -> String {
    app.path()
        .video_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("ScreenRecorder")
        .to_string_lossy()
        .into_owned()
}

#[tauri::command]
async fn pick_folder(app: AppHandle, current: String) -> Option<String> {
    app.dialog()
        .file()
        .set_directory(current)
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn reveal(path: String) {
    let p = PathBuf::from(&path);
    let mut cmd = Command::new("explorer");
    if p.is_file() {
        cmd.arg(format!("/select,{}", p.display()));
    } else {
        let _ = std::fs::create_dir_all(&p);
        cmd.arg(&p);
    }
    let _ = cmd.spawn();
}

#[tauri::command]
fn open_file(path: String) {
    let _ = Command::new("cmd")
        .args(["/C", "start", "", &path])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let _ = app.emit_to("main", "toggle-record", ());
                    }
                })
                .build(),
        )
        .manage(AppState::default())
        .setup(|app| {
            // 녹화 단축키는 화면이 저장된 설정으로 set_hotkey 를 호출해 등록함
            // 선택 영역 테두리: 클릭 통과 + 녹화에 찍히지 않음
            let border = WebviewWindowBuilder::new(app, "border", WebviewUrl::App("border.html".into()))
                .title("")
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .build()?;
            let _ = border.set_ignore_cursor_events(true);
            exclude(&border, true);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Destroyed = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            open_selector,
            selector_data,
            monitors,
            finish_selection,
            fullscreen_region,
            set_border,
            set_hide_self,
            start_recording,
            stop_recording,
            recording_alive,
            disk_free,
            recording_size,
            default_out_dir,
            pick_folder,
            reveal,
            open_file,
            set_hotkey
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
