//! Windows host.
//!
//! One thread, one hidden-by-default layered window:
//!   * layered + transparent + topmost + no-activate -> click-through overlay
//!   * receives tray callbacks, the global hotkey and messages from the
//!     settings app / `--toggle`.
//!
//! While hidden there are no timers and no ETW session: the process just
//! sleeps in GetMessage.

mod tray;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use uo_common::config::MonitorMode;
use uo_common::hotkey::Hotkey;
use uo_common::{MSG_QUIT, MSG_RELOAD, MSG_TOGGLE, OVERLAY_WINDOW_CLASS, SETTINGS_EXE};
use uo_overlay::core::{Core, Rect};
use uo_overlay::render::{Canvas, PxRect};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const TIMER_SENSORS: usize = 1;
const TIMER_GRAPH: usize = 2;
const HOTKEY_ID: i32 = 1;
pub const WM_TRAY: u32 = WM_APP + 2;

const CMD_TOGGLE: usize = 1;
const CMD_SETTINGS: usize = 2;
const CMD_RELOAD: usize = 4;
const CMD_EXIT: usize = 5;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------- surface

/// Memory DC + 32-bit DIB that UpdateLayeredWindow reads from.
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
    /// Window position of the last present (a move needs a full update).
    pos: (i32, i32),
}

impl Surface {
    fn new() -> Self {
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        Self {
            dc,
            bmp: std::ptr::null_mut(),
            old: std::ptr::null_mut(),
            bits: std::ptr::null_mut(),
            w: 0,
            h: 0,
            pos: (i32::MIN, i32::MIN),
        }
    }

    fn ensure(&mut self, w: i32, h: i32) {
        if w == self.w && h == self.h && !self.bmp.is_null() {
            return;
        }
        unsafe {
            if !self.bmp.is_null() {
                SelectObject(self.dc, self.old);
                DeleteObject(self.bmp);
            }
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h; // top-down
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            self.bmp = CreateDIBSection(
                self.dc,
                &bi,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            self.bits = bits as *mut u32;
            self.old = SelectObject(self.dc, self.bmp);
        }
        self.w = w;
        self.h = h;
    }

    /// Copy the canvas into the DIB and update the layered window. When only
    /// `dirty` changed (the graph between sensor ticks) just that part is
    /// copied and handed to the compositor.
    fn present(&mut self, hwnd: HWND, x: i32, y: i32, cv: &Canvas, dirty: Option<PxRect>) {
        let resized = cv.w != self.w || cv.h != self.h || self.bmp.is_null();
        self.ensure(cv.w, cv.h);
        if self.bits.is_null() {
            return;
        }
        let dirty = if resized || self.pos != (x, y) {
            None
        } else {
            dirty
        };
        unsafe {
            match dirty {
                None => std::ptr::copy_nonoverlapping(cv.px.as_ptr(), self.bits, cv.px.len()),
                Some(r) => {
                    for row in r.y..r.y + r.h {
                        let i = (row * cv.w + r.x) as usize;
                        std::ptr::copy_nonoverlapping(
                            cv.px.as_ptr().add(i),
                            self.bits.add(i),
                            r.w as usize,
                        );
                    }
                }
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dst = POINT { x, y };
            let size = SIZE { cx: cv.w, cy: cv.h };
            let src = POINT { x: 0, y: 0 };
            let rect = dirty.map(|r| RECT {
                left: r.x,
                top: r.y,
                right: r.x + r.w,
                bottom: r.y + r.h,
            });
            let info = UPDATELAYEREDWINDOWINFO {
                cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
                hdcDst: std::ptr::null_mut(),
                pptDst: &dst,
                psize: &size,
                hdcSrc: self.dc,
                pptSrc: &src,
                crKey: 0,
                pblend: &blend,
                dwFlags: ULW_ALPHA,
                prcDirty: rect.as_ref().map_or(std::ptr::null(), |r| r as *const RECT),
            };
            UpdateLayeredWindowIndirect(hwnd, &info);
        }
        self.pos = (x, y);
    }
}

// ---------------------------------------------------------------- app

struct App {
    hwnd: HWND,
    core: Core,
    visible: bool,
    surface: Surface,
    tray: tray::Tray,
    hotkey_ok: bool,
    /// Target monitor and DPI scale, refreshed every sensor tick.
    mon: (Rect, f32),
}

impl App {
    fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            core: Core::new(),
            visible: false,
            surface: Surface::new(),
            tray: tray::Tray::new(hwnd),
            hotkey_ok: false,
            mon: (
                Rect {
                    x: 0,
                    y: 0,
                    w: 1920,
                    h: 1080,
                },
                1.0,
            ),
        }
    }

    fn register_hotkey(&mut self) {
        unsafe { UnregisterHotKey(self.hwnd, HOTKEY_ID) };
        self.hotkey_ok = false;
        let hotkey = self.core.cfg.general.hotkey.clone();
        if let Some(hk) = Hotkey::parse(&hotkey) {
            if let Some(vk) = hk.vk() {
                let mut m = MOD_NOREPEAT;
                if hk.ctrl {
                    m |= MOD_CONTROL;
                }
                if hk.alt {
                    m |= MOD_ALT;
                }
                if hk.shift {
                    m |= MOD_SHIFT;
                }
                if hk.win {
                    m |= MOD_WIN;
                }
                self.hotkey_ok = unsafe { RegisterHotKey(self.hwnd, HOTKEY_ID, m, vk) } != 0;
            }
        }
        let tip = if self.hotkey_ok {
            format!("Universal Overlay — {hotkey}")
        } else {
            format!("Universal Overlay — сочетание {hotkey} занято")
        };
        self.tray.set_tip(&tip);
    }

    fn set_timers(&self) {
        let g = &self.core.cfg.general;
        unsafe {
            SetTimer(self.hwnd, TIMER_SENSORS, g.update_interval_ms, None);
            if self.core.cfg.graph.enabled {
                SetTimer(
                    self.hwnd,
                    TIMER_GRAPH,
                    (1000 / g.graph_refresh_hz).max(4),
                    None,
                );
            } else {
                KillTimer(self.hwnd, TIMER_GRAPH);
            }
        }
    }

    fn show(&mut self) {
        if self.visible {
            return;
        }
        self.visible = true;
        self.core.start_frames();
        self.set_timers();
        self.tick();
        unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
    }

    fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        unsafe {
            KillTimer(self.hwnd, TIMER_SENSORS);
            KillTimer(self.hwnd, TIMER_GRAPH);
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.core.on_hidden();
        // Return freed pages to the OS; the hidden overlay should be tiny.
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }

    fn toggle(&mut self) {
        if self.visible {
            self.hide()
        } else {
            self.show()
        }
    }

    fn reload(&mut self) {
        self.core.reload(self.visible);
        self.register_hotkey();
        if self.visible {
            self.set_timers();
            self.tick();
        }
    }

    fn foreground(&self) -> (HWND, u32) {
        unsafe {
            let fg = GetForegroundWindow();
            let mut pid = 0;
            if !fg.is_null() {
                GetWindowThreadProcessId(fg, &mut pid);
            }
            (fg, pid)
        }
    }

    fn tick(&mut self) {
        let (_, pid) = self.foreground();
        let own = pid == unsafe { GetCurrentProcessId() };
        self.core.tick((!own).then_some(pid));
        self.mon = self.monitor();
        self.render();
        // Games sometimes push themselves above other topmost windows.
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
        }
    }

    fn monitor(&self) -> (Rect, f32) {
        unsafe {
            let mon = match self.core.cfg.general.monitor {
                MonitorMode::Foreground => {
                    MonitorFromWindow(self.foreground().0, MONITOR_DEFAULTTOPRIMARY)
                }
                MonitorMode::Primary => {
                    MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
                }
            };
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let r = mi.rcMonitor;
            (
                Rect {
                    x: r.left,
                    y: r.top,
                    w: r.right - r.left,
                    h: r.bottom - r.top,
                },
                dx as f32 / 96.0,
            )
        }
    }

    fn render(&mut self) {
        if !self.visible {
            return;
        }
        let (mon, dpi) = self.mon;
        let (x, y) = self.core.render(mon, dpi, 1.0);
        self.surface
            .present(self.hwnd, x, y, &self.core.canvas, self.core.dirty);
    }

    fn open_settings(&self) {
        open_settings();
    }
}

fn open_settings() {
    let path = super::exe_dir().join(format!("{SETTINGS_EXE}.exe"));
    let p = wide(&path.to_string_lossy());
    let op = wide("open");
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            p.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn find_overlay() -> Option<HWND> {
    let cls = wide(OVERLAY_WINDOW_CLASS);
    let w = unsafe { FindWindowW(cls.as_ptr(), std::ptr::null()) };
    (!w.is_null()).then_some(w)
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed (re-entrant message
/// during a modal loop) — such messages are simply skipped.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn tray_menu(hwnd: HWND) {
    let Some((visible, hotkey)) = with_app(|a| (a.visible, a.core.cfg.general.hotkey.clone()))
    else {
        return;
    };
    unsafe {
        let m = CreatePopupMenu();
        let add = |id: usize, s: &str| {
            let w = wide(s);
            AppendMenuW(m, MF_STRING, id, w.as_ptr());
        };
        add(
            CMD_TOGGLE,
            &format!(
                "{}\t{hotkey}",
                if visible {
                    "Скрыть оверлей"
                } else {
                    "Показать оверлей"
                }
            ),
        );
        add(CMD_SETTINGS, "Настройки…");
        add(CMD_RELOAD, "Перечитать конфиг");
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        add(CMD_EXIT, "Выход");
        SetMenuDefaultItem(m, CMD_SETTINGS as u32, 0);
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd); // required so the menu closes on outside click
        let cmd = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        match cmd as usize {
            CMD_TOGGLE => {
                with_app(|a| a.toggle());
            }
            CMD_SETTINGS => {
                with_app(|a| a.open_settings());
            }
            CMD_RELOAD => {
                with_app(|a| a.reload());
            }
            CMD_EXIT => {
                DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY | MSG_TOGGLE => {
            with_app(|a| a.toggle());
            0
        }
        WM_TIMER => {
            match wp {
                TIMER_SENSORS => with_app(|a| a.tick()),
                TIMER_GRAPH => with_app(|a| a.render()),
                _ => None,
            };
            0
        }
        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => {
                    with_app(|a| a.toggle());
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(hwnd),
                _ => {}
            }
            0
        }
        MSG_RELOAD => {
            with_app(|a| a.reload());
            0
        }
        MSG_QUIT => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            with_app(|a| {
                a.hide();
                a.tray.remove();
            });
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_app(|a| a.tray.add());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Returns false if another overlay instance is running.
fn single_instance() -> bool {
    let name = wide("Local\\UniversalOverlay.SingleInstance");
    // A quitting instance (update, restart) may need a moment to exit.
    for _ in 0..20 {
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS {
                return true; // keep the handle for the process lifetime
            }
            if !h.is_null() {
                CloseHandle(h);
            }
            Sleep(100);
        }
    }
    false
}

pub fn run() {
    let has = |a: &str| std::env::args().any(|x| x == a);
    let toggle = has("--toggle");
    if let Some(w) = find_overlay() {
        // Already running. --toggle toggles; a plain launch (Start menu,
        // desktop shortcut) behaves like reopening an app: settings.
        unsafe { PostMessageW(w, if toggle { MSG_TOGGLE } else { MSG_RELOAD }, 0, 0) };
        if !toggle && !has("--background") {
            open_settings();
        }
        return;
    }
    if !single_instance() {
        return;
    }
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(std::ptr::null());
        let cls = wide(OVERLAY_WINDOW_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst,
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let title = wide("Universal Overlay");
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            inst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        // Allow other integrity levels (e.g. an elevated installer) and Explorer to reach us.
        ChangeWindowMessageFilterEx(hwnd, MSG_RELOAD, MSGFLT_ALLOW, std::ptr::null_mut());
        ChangeWindowMessageFilterEx(hwnd, MSG_TOGGLE, MSGFLT_ALLOW, std::ptr::null_mut());
        ChangeWindowMessageFilterEx(hwnd, MSG_QUIT, MSGFLT_ALLOW, std::ptr::null_mut());
        let tb = wide("TaskbarCreated");
        let tbm = RegisterWindowMessageW(tb.as_ptr());
        TASKBAR_CREATED.store(tbm, Ordering::Relaxed);
        ChangeWindowMessageFilterEx(hwnd, tbm, MSGFLT_ALLOW, std::ptr::null_mut());

        let mut app = App::new(hwnd);
        let show = app.core.cfg.general.show_on_start || toggle;
        app.tray.add();
        app.register_hotkey();
        APP.with(|a| *a.borrow_mut() = Some(app));
        if show {
            with_app(|a| a.show());
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|a| a.borrow_mut().take());
    }
}
