//! Windows integration for Options > General: the tray icon (minimize / close to tray, toast
//! notifications through it), launch at sign-in, the taskbar badge (overlay icon), the
//! notification sound, and the memory reading used by Session management.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// Set by the tray's Quit, so the close request that follows really closes.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// What the tray reports back to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// The tray icon or the window was brought back.
    Shown,
    /// A notification was clicked (opens the session it was about).
    NotificationClicked,
}

static EVENTS: Mutex<Vec<TrayEvent>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn take_events() -> Vec<TrayEvent> {
    std::mem::take(&mut *EVENTS.lock().unwrap())
}

fn push(e: TrayEvent) {
    EVENTS.lock().unwrap().push(e);
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
    const ID_SHOW: usize = 1;
    const ID_QUIT: usize = 2;

    static MAIN: AtomicIsize = AtomicIsize::new(0);
    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

    fn hwnd(v: isize) -> HWND {
        HWND(v as *mut _)
    }

    // ---- main window -----------------------------------------------------------------------

    unsafe extern "system" fn enum_cb(h: HWND, lparam: LPARAM) -> BOOL {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() && h.0 as isize != TRAY_HWND.load(Ordering::Relaxed) {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, &mut buf);
            if String::from_utf16_lossy(&buf[..n.max(0) as usize]).contains("AI Terminal") {
                *(lparam.0 as *mut isize) = h.0 as isize;
                return FALSE;
            }
        }
        TRUE
    }

    /// The app's main window (found once by process id and title, then cached).
    pub fn main_hwnd() -> Option<HWND> {
        let v = MAIN.load(Ordering::Relaxed);
        if v != 0 {
            return Some(hwnd(v));
        }
        let mut found: isize = 0;
        unsafe {
            let _ = EnumWindows(Some(enum_cb), LPARAM(&mut found as *mut isize as isize));
        }
        (found != 0).then(|| {
            MAIN.store(found, Ordering::Relaxed);
            hwnd(found)
        })
    }

    pub fn show_main() {
        if let Some(h) = main_hwnd() {
            unsafe {
                let _ = ShowWindow(h, SW_SHOW);
                if IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_RESTORE);
                }
                let _ = SetForegroundWindow(h);
            }
        }
    }

    pub fn hide_main() {
        if let Some(h) = main_hwnd() {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
    }

    /// Windows sometimes makes the window the foreground one at start-up without giving it the
    /// keyboard: keys then go nowhere, and clicking doesn't help (the window is already active, so
    /// a click doesn't activate it again). When the main window is in front but doesn't hold the
    /// keyboard, take it. Returns true when it had to. Call on the UI thread.
    pub fn claim_keyboard_focus() -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
        let Some(h) = main_hwnd() else { return false };
        unsafe {
            if GetForegroundWindow() != h || GetFocus() == h {
                return false;
            }
            crate::dlog!("main window is in front without the keyboard (focus {:?}): taking it", GetFocus());
            let _ = SetFocus(Some(h));
        }
        true
    }

    pub fn is_main_visible() -> bool {
        main_hwnd().is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
    }

    // ---- tray --------------------------------------------------------------------------------

    unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_TRAY {
            match lp.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                    show_main();
                    push(TrayEvent::Shown);
                }
                NIN_BALLOONUSERCLICK => {
                    show_main();
                    push(TrayEvent::NotificationClicked);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    if let Ok(menu) = CreatePopupMenu() {
                        let _ = AppendMenuW(menu, MF_STRING, ID_SHOW, w!("Show AI Terminal"));
                        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        // Needed so the menu closes when clicking elsewhere.
                        let _ = SetForegroundWindow(h);
                        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, Some(0), h, None);
                        let _ = DestroyMenu(menu);
                        match cmd.0 as usize {
                            ID_SHOW => {
                                show_main();
                                push(TrayEvent::Shown);
                            }
                            ID_QUIT => {
                                QUITTING.store(true, Ordering::Relaxed);
                                if let Some(m) = main_hwnd() {
                                    let _ = PostMessageW(Some(m), WM_CLOSE, WPARAM(0), LPARAM(0));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return LRESULT(0);
        }
        DefWindowProcW(h, msg, wp, lp)
    }

    fn icon_data(h: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: h, uID: 1, ..Default::default() }
    }

    fn copy_wide(dst: &mut [u16], s: &str) {
        let v: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
        dst[..v.len()].copy_from_slice(&v);
        dst[v.len()] = 0;
    }

    unsafe fn app_icon(size: i32) -> HICON {
        let inst = GetModuleHandleW(None).ok();
        // The exe's icon resource (id 1, from assets/ai_terminal.rc).
        LoadImageW(inst.map(|m| HINSTANCE(m.0)), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR).map(|h| HICON(h.0)).unwrap_or_default()
    }

    /// Adds the tray icon (idempotent). The hidden window that receives its clicks lives on
    /// the UI thread, whose message loop (winit's) dispatches to it.
    pub fn tray_show(ctx: &egui::Context) {
        let _ = CTX.set(ctx.clone());
        if TRAY_HWND.load(Ordering::Relaxed) != 0 {
            return;
        }
        unsafe {
            let Ok(inst) = GetModuleHandleW(None) else { return };
            let class = w!("AITerminalTray");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: HINSTANCE(inst.0), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("AI Terminal tray"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(HINSTANCE(inst.0)), None) else { return };
            TRAY_HWND.store(h.0 as isize, Ordering::Relaxed);
            let mut d = icon_data(h);
            d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            d.uCallbackMessage = WM_TRAY;
            d.hIcon = app_icon(16);
            copy_wide(&mut d.szTip, "AI Terminal");
            let _ = Shell_NotifyIconW(NIM_ADD, &d);
        }
    }

    pub fn tray_hide() {
        let v = TRAY_HWND.swap(0, Ordering::Relaxed);
        if v != 0 {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd(v)));
                let _ = DestroyWindow(hwnd(v));
            }
        }
    }

    // ---- single instance -----------------------------------------------------------------------
    //
    // While "Allow only one instance" is on, the running app keeps a hidden window of a known
    // class. A second launch finds it, asks it to show the main window, and exits.

    const WM_ACTIVATE_INSTANCE: u32 = WM_APP + 2;
    static INSTANCE_HWND: AtomicIsize = AtomicIsize::new(0);

    fn instance_class() -> PCWSTR {
        // The test instance (own settings) is kept apart from the real one.
        if crate::update::test_instance() {
            w!("AITerminalInstanceTest")
        } else {
            w!("AITerminalInstance")
        }
    }

    unsafe extern "system" fn instance_wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_ACTIVATE_INSTANCE {
            show_main();
            push(TrayEvent::Shown);
            return LRESULT(0);
        }
        DefWindowProcW(h, msg, wp, lp)
    }

    /// Creates (on) or removes (off) the window a second launch looks for. Call on the UI thread.
    pub fn set_single_instance(ctx: &egui::Context, on: bool) {
        let _ = CTX.set(ctx.clone());
        let cur = INSTANCE_HWND.load(Ordering::Relaxed);
        if on == (cur != 0) {
            return;
        }
        unsafe {
            if !on {
                INSTANCE_HWND.store(0, Ordering::Relaxed);
                let _ = DestroyWindow(hwnd(cur));
                return;
            }
            let Ok(inst) = GetModuleHandleW(None) else { return };
            let class = instance_class();
            let wc = WNDCLASSW { lpfnWndProc: Some(instance_wndproc), hInstance: HINSTANCE(inst.0), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            // No "AI Terminal" in the title: main_hwnd() looks for that.
            if let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("AITerminalInstance"), WS_OVERLAPPED, 0, 0, 0, 0, None, None, Some(HINSTANCE(inst.0)), None) {
                INSTANCE_HWND.store(h.0 as isize, Ordering::Relaxed);
            }
        }
    }

    /// On exit: drop the window, so a relaunch doesn't hand off to this closing process.
    pub fn release_single_instance() {
        let v = INSTANCE_HWND.swap(0, Ordering::Relaxed);
        if v != 0 {
            unsafe {
                let _ = DestroyWindow(hwnd(v));
            }
        }
    }

    /// At start-up: if a running instance wants to be the only one, bring it to the front
    /// (unless `activate` is false) and return true, so this launch exits.
    pub fn hand_off_to_running_instance(activate: bool) -> bool {
        unsafe {
            let Ok(h) = FindWindowW(instance_class(), PCWSTR::null()) else { return false };
            if h.is_invalid() {
                return false;
            }
            if activate {
                // We were just launched by the user, so we may pass the right to take the foreground on.
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(h), WM_ACTIVATE_INSTANCE, WPARAM(0), LPARAM(0));
            }
            true
        }
    }

    /// A toast notification (shown through the tray icon, so it carries the app's icon).
    pub fn notify(ctx: &egui::Context, title: &str, body: &str) {
        tray_show(ctx);
        let v = TRAY_HWND.load(Ordering::Relaxed);
        if v == 0 {
            return;
        }
        let mut d = icon_data(hwnd(v));
        d.uFlags = NIF_INFO;
        copy_wide(&mut d.szInfoTitle, title);
        copy_wide(&mut d.szInfo, body);
        // Our own sound plays (or not) separately, per the option.
        d.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON | NIIF_NOSOUND;
        unsafe {
            d.hBalloonIcon = app_icon(32);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn play_notification_sound() {
        unsafe {
            let _ = PlaySoundW(w!("Notification.Default"), None, SND_ALIAS | SND_ASYNC | SND_NODEFAULT);
        }
    }

    // ---- launch at sign-in ---------------------------------------------------------------------

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const RUN_VALUE: PCWSTR = w!("AI Terminal");

    fn startup_command() -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        Some(format!("\"{}\" --minimized", exe.display()))
    }

    pub fn autostart_enabled() -> bool {
        let mut size = 0u32;
        unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, Some(&mut size)).is_ok() }
    }

    pub fn set_autostart(on: bool) -> Result<(), String> {
        unsafe {
            if on {
                let cmd = startup_command().ok_or("Can't find the exe path")?;
                let wide: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
                let r = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(wide.as_ptr() as *const _), (wide.len() * 2) as u32);
                if r.is_err() {
                    return Err(format!("Couldn't write the Run key ({})", r.0));
                }
            } else {
                let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            }
        }
        Ok(())
    }

    // ---- installed Store packages ------------------------------------------------------------------

    /// The install folder of the current user's Store package whose full name starts with
    /// `prefix` (e.g. "OpenAI.Codex_"); the folder under WindowsApps can't be listed.
    pub fn package_root(prefix: &str) -> Option<std::path::PathBuf> {
        const PACKAGES: PCWSTR = w!("Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages");
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, PACKAGES, None, KEY_READ, &mut key).is_err() {
                return None;
            }
            let mut found: Option<(String, std::path::PathBuf)> = None;
            let mut i = 0u32;
            loop {
                let mut name = [0u16; 512];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                    break;
                }
                i += 1;
                let full = String::from_utf16_lossy(&name[..len as usize]);
                if !full.starts_with(prefix) {
                    continue;
                }
                let sub: Vec<u16> = full.encode_utf16().chain([0]).collect();
                let mut buf = [0u16; 1024];
                let mut size = (buf.len() * 2) as u32;
                if RegGetValueW(key, PCWSTR(sub.as_ptr()), w!("PackageRootFolder"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)).is_ok() {
                    let root = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]));
                    // Several versions can be registered for a moment during an update: take the newest.
                    if root.is_dir() && found.as_ref().is_none_or(|(n, _)| full > *n) {
                        found = Some((full, root));
                    }
                }
            }
            let _ = RegCloseKey(key);
            found.map(|f| f.1)
        }
    }

    // ---- PATH ------------------------------------------------------------------------------------

    /// The PATH as the registry has it now (machine, then user; %VARS% expanded). Installers
    /// (npm, uv, …) add folders there, which a running process never sees in its own PATH.
    pub fn registry_path() -> Vec<std::path::PathBuf> {
        const MACHINE: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment");
        let read = |root: HKEY, sub: PCWSTR| -> Option<String> {
            unsafe {
                let mut size = 0u32;
                // RRF_RT_REG_EXPAND_SZ values are expanded unless RRF_NOEXPAND is given.
                let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
                RegGetValueW(root, sub, w!("Path"), flags, None, None, Some(&mut size)).ok().ok()?;
                let mut buf = vec![0u16; size as usize / 2 + 1];
                let mut size = (buf.len() * 2) as u32;
                RegGetValueW(root, sub, w!("Path"), flags, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)).ok().ok()?;
                // With expansion, the returned size can exceed the expanded string (it may be the
                // raw value's size), leaving stale data after the terminator: stop at the first NUL.
                let n = (size as usize / 2).min(buf.len());
                let n = buf[..n].iter().position(|&c| c == 0).unwrap_or(n);
                Some(String::from_utf16_lossy(&buf[..n]))
            }
        };
        [read(HKEY_LOCAL_MACHINE, MACHINE), read(HKEY_CURRENT_USER, w!("Environment"))]
            .into_iter()
            .flatten()
            .flat_map(|s| s.split(';').map(str::trim).filter(|d| !d.is_empty()).map(std::path::PathBuf::from).collect::<Vec<_>>())
            .collect()
    }

    // ---- memory ----------------------------------------------------------------------------------

    /// Memory in use on the machine, in GB.
    pub fn memory_used_gb() -> f64 {
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        unsafe {
            if GlobalMemoryStatusEx(&mut m).is_err() {
                return 0.0;
            }
        }
        (m.ullTotalPhys - m.ullAvailPhys) as f64 / (1u64 << 30) as f64
    }

    // ---- taskbar badge -----------------------------------------------------------------------------

    thread_local! {
        static TASKBAR: Option<ITaskbarList3> = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok().and_then(|t: ITaskbarList3| t.HrInit().ok().map(|_| t))
        };
    }

    unsafe fn icon_from_rgba(rgba: &[u8], size: i32) -> Option<HICON> {
        // Windows wants BGRA rows; alpha is taken from the 32-bit colour bitmap.
        let bgra: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let color = CreateBitmap(size, size, 1, 32, Some(bgra.as_ptr() as *const _));
        let mask_bits = vec![0u8; ((size + 15) / 16 * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }

    /// Shows `rgba` (size × size) as the taskbar button's overlay, or clears it.
    pub fn set_badge(rgba: Option<(&[u8], i32)>, description: &str) {
        let Some(h) = main_hwnd() else { return };
        TASKBAR.with(|t| {
            let Some(t) = t else { return };
            unsafe {
                let desc: Vec<u16> = description.encode_utf16().chain([0]).collect();
                match rgba.and_then(|(px, s)| icon_from_rgba(px, s)) {
                    Some(icon) => {
                        let _ = t.SetOverlayIcon(h, icon, PCWSTR(desc.as_ptr()));
                        let _ = DestroyIcon(icon);
                    }
                    None => {
                        let _ = t.SetOverlayIcon(h, HICON::default(), PCWSTR::null());
                    }
                }
            }
        });
    }
}

#[cfg(windows)]
pub use imp::*;

/// The badge image: a filled circle in `bg`, with `count` in `fg` (no number when None).
pub fn badge_rgba(count: Option<usize>, bg: [u8; 3], fg: [u8; 3]) -> (Vec<u8>, i32) {
    let s = 32i32;
    let text = count.map(|n| if n > 9 { "9+".to_string() } else { n.to_string() });
    let digits = text.as_deref().and_then(|t| crate::emoji::render_text_rgba(t, [s as u32, s as u32], fg, true));
    let mut px = vec![0u8; (s * s * 4) as usize];
    let c = (s as f32 - 1.0) / 2.0;
    let r = if count.is_some() { s as f32 / 2.0 - 0.5 } else { s as f32 * 0.32 };
    for y in 0..s {
        for x in 0..s {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let a = (r - d + 0.5).clamp(0.0, 1.0);
            let i = ((y * s + x) * 4) as usize;
            let (mut cr, mut cg, mut cb) = (bg[0] as f32, bg[1] as f32, bg[2] as f32);
            if let Some(dg) = &digits {
                // Text over the circle (straight alpha).
                let ta = dg[i + 3] as f32 / 255.0;
                cr = cr * (1.0 - ta) + dg[i] as f32 * ta;
                cg = cg * (1.0 - ta) + dg[i + 1] as f32 * ta;
                cb = cb * (1.0 - ta) + dg[i + 2] as f32 * ta;
            }
            px[i..i + 4].copy_from_slice(&[cr as u8, cg as u8, cb as u8, (a * 255.0) as u8]);
        }
    }
    (px, s)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn package_root_is_a_folder_when_found() {
        if let Some(root) = super::package_root("OpenAI.Codex_") {
            eprintln!("Codex package: {}", root.display());
            assert!(root.is_dir());
        }
        assert!(super::package_root("No.Such.Package_").is_none());
    }

    #[test]
    fn registry_path_is_expanded_and_has_system32() {
        let dirs = super::registry_path();
        assert!(dirs.iter().all(|d| !d.to_string_lossy().contains('%')), "{dirs:?}");
        assert!(dirs.iter().any(|d| d.to_string_lossy().to_lowercase().ends_with("system32")), "{dirs:?}");
    }
}
