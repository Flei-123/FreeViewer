//! The tray icon - FreeViewer stays reachable without a window in the way.
//!
//! Why hand rolled instead of a crate: eframe already owns the winit event
//! loop of this process and every tray crate wants to own one too. A tray
//! icon is just a hidden window plus `Shell_NotifyIcon`, so it gets its own
//! thread with its own message pump here and never touches egui's loop.
//!
//! Showing and hiding the main window goes through plain Win32
//! (`ShowWindow`) on purpose: it has to work even while egui draws no frames
//! because nothing is visible.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::shared::Shared;

/// The main window is currently folded away into the tray.
static HIDDEN: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 0 = offline, 1 = ready, 2 = session running.
static STATE: AtomicU8 = AtomicU8::new(0);
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
/// Set by the tray thread, picked up and cleared by the GUI thread.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

pub fn is_hidden() -> bool {
    HIDDEN.load(Ordering::Relaxed)
}

/// State the icon is showing right now (0 offline, 1 ready, 2 session).
pub fn state() -> u8 {
    STATE.load(Ordering::Relaxed)
}

/// Something the user should know about (autostart could not be written...).
pub fn take_error() -> Option<String> {
    let mut g = LAST_ERROR.lock().unwrap();
    if g.is_empty() {
        None
    } else {
        Some(std::mem::take(&mut *g))
    }
}

fn set_error(s: impl Into<String>) {
    *LAST_ERROR.lock().unwrap() = s.into();
}

/// Tooltip text plus the state it belongs to. Lives outside the Windows
/// module so it can be unit tested without a desktop.
fn tip_for(version: &str, id: &str, status: &str, peer: &str, in_session: bool) -> (u8, String) {
    let state = if in_session {
        2
    } else if id.is_empty() {
        0
    } else {
        1
    };
    let line = if id.is_empty() {
        status.to_string()
    } else {
        format!("ID {}", crate::partners::pretty_id(id))
    };
    let third = if in_session {
        peer.to_string()
    } else {
        "Bereit fuer Verbindungen".to_string()
    };
    (state, format!("FreeViewer {}\n{}\n{}", version, line, third))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::AtomicIsize;

    use std::sync::atomic::AtomicU32;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{
        BOOL, ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TRAY: u32 = WM_APP + 1;
    const ID_OPEN: usize = 1;
    const ID_COPY: usize = 2;
    const ID_AUTOSTART: usize = 3;
    const ID_QUIT: usize = 4;
    const ICON_ID: u32 = 1;

    /// Colours of the three states, in the order of STATE.
    const COLORS: [(u8, u8, u8); 3] = [
        (0x8b, 0x95, 0xab), // grey  - not registered at the relay
        (0x38, 0xbd, 0xf8), // blue  - ready (the product colour)
        (0x22, 0xc5, 0x5e), // green - a session is running
    ];

    /// Window handle and icons as plain integers: raw pointers are not `Send`
    /// and this state is reached from the timer inside the window procedure.
    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
    static ICONS: [AtomicIsize; 3] = [
        AtomicIsize::new(0),
        AtomicIsize::new(0),
        AtomicIsize::new(0),
    ];
    static SHOWN: AtomicU8 = AtomicU8::new(255);
    static LAST_TIP: Mutex<String> = Mutex::new(String::new());
    /// Broadcast message a second instance uses to say "show yourself".
    static SHOW_MSG: AtomicU32 = AtomicU32::new(0);
    /// Held for the lifetime of the process - dropping it would free the name.
    static INSTANCE_MUTEX: AtomicIsize = AtomicIsize::new(0);

    /// Are we the first instance of this user? A second one asks the running
    /// one to come to the front and then leaves - two hosts with the same
    /// identity would kick each other off the relay.
    pub fn claim_single_instance(agent: bool) -> bool {
        unsafe {
            let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string());
            // GUI und Dienst-Agent duerfen nebeneinander laufen - der Agent ist
            // kein "zweites Fenster". Frueher teilten sie sich eine Marke: der
            // Dienst startete den Agenten, der hielt die GUI fuer ein Duplikat,
            // riss ihr Fenster nach vorn und beendete sich - alle 4 Sekunden
            // von neuem. Genau das Dauer-Blinken in der Taskleiste.
            let rolle = if agent { "agent" } else { "gui" };
            let name = wide(&format!("Local\\FreeViewer-{}-{}", rolle, user));
            match CreateMutexW(None, false, PCWSTR(name.as_ptr())) {
                Ok(h) => {
                    if GetLastError() == ERROR_ALREADY_EXISTS {
                        if !agent {
                            let msg = RegisterWindowMessageW(w!("FreeViewerShowWindow"));
                            if msg != 0 {
                                let _ = PostMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
                            }
                        }
                        false
                    } else {
                        INSTANCE_MUTEX.store(h.0 as isize, Ordering::Relaxed);
                        true
                    }
                }
                // no mutex, no protection - better to run than to refuse
                Err(_) => true,
            }
        }
    }

    fn tray_hwnd() -> HWND {
        HWND(TRAY_HWND.load(Ordering::Relaxed) as *mut _)
    }

    /// Draws a 32x32 "eye": coloured ring, dark iris, white centre. Round on
    /// purpose - `CreateIcon` takes device dependent bits whose row order is
    /// bottom up, and a symmetric shape cannot end up upside down.
    unsafe fn make_icon(rgb: (u8, u8, u8)) -> Option<HICON> {
        const N: i32 = 32;
        let mut xor = vec![0u8; (N * N * 4) as usize];
        // 1 bit per pixel, 1 = leave the background alone (transparent)
        let mut and = vec![0xffu8; (N * N / 8) as usize];
        let c = (N as f32 - 1.0) / 2.0;
        for y in 0..N {
            for x in 0..N {
                let dx = x as f32 - c;
                let dy = y as f32 - c;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 15.0 {
                    continue;
                }
                let (r, g, b) = if d <= 5.0 {
                    (0xff, 0xff, 0xff)
                } else if d <= 8.5 {
                    (0x0b, 0x11, 0x1d)
                } else {
                    rgb
                };
                let i = ((y * N + x) * 4) as usize;
                xor[i] = b;
                xor[i + 1] = g;
                xor[i + 2] = r;
                xor[i + 3] = 0xff;
                let bit = (y * N + x) as usize;
                and[bit / 8] &= !(0x80u8 >> (bit % 8));
            }
        }
        let hinst = GetModuleHandleW(None).ok()?;
        CreateIcon(
            HINSTANCE(hinst.0),
            N,
            N,
            1,
            32,
            and.as_ptr(),
            xor.as_ptr(),
        )
        .ok()
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Fills one of the fixed size string fields of NOTIFYICONDATAW and keeps
    /// the terminating zero, no matter how long the text was.
    fn put(dst: &mut [u16], s: &str) {
        let len = dst.len();
        if len == 0 {
            return;
        }
        let src = wide(s);
        let n = src.len().min(len);
        dst[..n].copy_from_slice(&src[..n]);
        dst[len - 1] = 0;
        if n == len {
            dst[len - 1] = 0;
        }
    }

    fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            ..Default::default()
        }
    }

    /// Current tooltip and state, read from the shared host state.
    fn tip_now() -> (u8, String) {
        let sh = match SHARED.get() {
            Some(s) => s,
            None => return (0, "FreeViewer".to_string()),
        };
        let id = sh.my_id.lock().unwrap().clone();
        let peer = sh.host_peer.lock().unwrap().clone();
        let status = sh.host_status.lock().unwrap().clone();
        let in_session =
            sh.connected.load(Ordering::Relaxed) || peer.to_lowercase().contains("sitzung mit");
        tip_for(crate::update::VERSION, &id, &status, &peer, in_session)
    }

    unsafe fn refresh_icon() {
        let hwnd = tray_hwnd();
        if hwnd.0.is_null() {
            return;
        }
        let (state, tip) = tip_now();
        {
            let mut last = LAST_TIP.lock().unwrap();
            if state == SHOWN.load(Ordering::Relaxed) && *last == tip {
                return;
            }
            last.clone_from(&tip);
        }
        SHOWN.store(state, Ordering::Relaxed);
        STATE.store(state, Ordering::Relaxed);
        let mut data = nid(hwnd);
        data.uFlags = NIF_ICON | NIF_TIP;
        data.hIcon = HICON(ICONS[state as usize].load(Ordering::Relaxed) as *mut _);
        put(&mut data.szTip, &tip);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }

    /// A balloon, used exactly once: when the window disappears into the tray
    /// for the first time, so nobody thinks the program is gone.
    pub unsafe fn balloon(title: &str, text: &str) {
        let hwnd = tray_hwnd();
        if hwnd.0.is_null() {
            return;
        }
        let mut data = nid(hwnd);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        put(&mut data.szInfoTitle, title);
        put(&mut data.szInfo, text);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }

    /// Finds our own main window.
    ///
    /// Going by "first window of this process with a caption" is not enough:
    /// the graphics driver parks its own helper windows in here (NVIDIA has
    /// one called `__wglDummyWindowFodder`), and hiding one of those instead
    /// of the real window looks exactly like a broken tray icon. So the title
    /// has to match ours, and the tray window itself is skipped.
    unsafe extern "system" fn find_main(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != GetCurrentProcessId() {
            return true.into();
        }
        if hwnd.0 as isize == TRAY_HWND.load(Ordering::Relaxed) {
            return true.into();
        }
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return true.into();
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        if !title.starts_with("FreeViewer") || title.contains("Tray") {
            return true.into();
        }
        let out = lparam.0 as *mut isize;
        *out = hwnd.0 as isize;
        false.into()
    }

    pub unsafe fn main_hwnd() -> Option<HWND> {
        let mut found: isize = 0;
        let _ = EnumWindows(Some(find_main), LPARAM(&mut found as *mut isize as isize));
        if found == 0 {
            None
        } else {
            Some(HWND(found as *mut _))
        }
    }

    pub unsafe fn show_main() {
        if let Some(h) = main_hwnd() {
            let _ = ShowWindow(h, SW_SHOW);
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
        HIDDEN.store(false, Ordering::Relaxed);
    }

    pub unsafe fn hide_main() {
        if let Some(h) = main_hwnd() {
            let _ = ShowWindow(h, SW_HIDE);
        }
        HIDDEN.store(true, Ordering::Relaxed);
    }

    fn copy_id() {
        let sh = match SHARED.get() {
            Some(s) => s,
            None => return,
        };
        let id = sh.my_id.lock().unwrap().clone();
        if id.is_empty() {
            set_error("Noch keine ID - FreeViewer ist nicht beim Relay angemeldet");
            return;
        }
        let text = crate::partners::pretty_id(&id);
        if let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            set_error(format!("ID kopieren ging nicht: {}", e));
        }
    }

    unsafe fn menu(hwnd: HWND) {
        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };
        let open = wide("FreeViewer oeffnen");
        let copy = wide("Meine ID kopieren");
        let auto = wide("Mit Windows starten");
        let quit = wide("FreeViewer beenden");
        let _ = AppendMenuW(hmenu, MF_STRING, ID_OPEN, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_STRING, ID_COPY, PCWSTR(copy.as_ptr()));
        let flags = if crate::autostart::enabled() {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(hmenu, flags, ID_AUTOSTART, PCWSTR(auto.as_ptr()));
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(hmenu, MF_STRING, ID_QUIT, PCWSTR(quit.as_ptr()));

        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // without this the menu would stay open when the user clicks elsewhere
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            hmenu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            p.x,
            p.y,
            0,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(hmenu);
    }

    unsafe fn command(id: usize) {
        match id {
            ID_OPEN => show_main(),
            ID_COPY => copy_id(),
            ID_AUTOSTART => {
                let on = !crate::autostart::enabled();
                match crate::autostart::set(on) {
                    Ok(()) => balloon(
                        "FreeViewer",
                        if on {
                            "Startet ab jetzt automatisch mit Windows."
                        } else {
                            "Startet nicht mehr automatisch mit Windows."
                        },
                    ),
                    Err(e) => set_error(format!("Autostart ging nicht: {}", e)),
                }
            }
            ID_QUIT => {
                remove();
                std::process::exit(0);
            }
            _ => {}
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        let show = SHOW_MSG.load(Ordering::Relaxed);
        if show != 0 && msg == show {
            show_main();
            return LRESULT(0);
        }
        match msg {
            WM_TRAY => {
                let ev = (l.0 as u32) & 0xffff;
                if ev == WM_LBUTTONUP || ev == WM_LBUTTONDBLCLK {
                    show_main();
                } else if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                    menu(hwnd);
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                command((w.0 & 0xffff) as usize);
                LRESULT(0)
            }
            WM_TIMER => {
                refresh_icon();
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    /// Takes the icon out of the notification area again.
    pub fn remove() {
        let h = TRAY_HWND.swap(0, Ordering::Relaxed);
        if h == 0 {
            return;
        }
        unsafe {
            let data = nid(HWND(h as *mut _));
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
        RUNNING.store(false, Ordering::Relaxed);
    }

    pub fn start(shared: Arc<Shared>) {
        let _ = SHARED.set(shared);
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            let hinst = match GetModuleHandleW(None) {
                Ok(h) => HINSTANCE(h.0),
                Err(_) => {
                    RUNNING.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let class = w!("FreeViewerTray");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("FreeViewer Tray"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                HWND::default(),
                HMENU::default(),
                hinst,
                None,
            ) {
                Ok(h) => h,
                Err(_) => {
                    RUNNING.store(false, Ordering::Relaxed);
                    return;
                }
            };
            TRAY_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
            // The agent may run as SYSTEM while the taskbar runs as the user.
            // Without this, UIPI would silently drop every click on the icon.
            let show = RegisterWindowMessageW(w!("FreeViewerShowWindow"));
            SHOW_MSG.store(show, Ordering::Relaxed);
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_TRAY, MSGFLT_ALLOW, None);
            if show != 0 {
                let _ = ChangeWindowMessageFilterEx(hwnd, show, MSGFLT_ALLOW, None);
            }

            for (i, c) in COLORS.iter().enumerate() {
                if let Some(h) = make_icon(*c) {
                    ICONS[i].store(h.0 as isize, Ordering::Relaxed);
                }
            }
            let (state, tip) = tip_now();
            let mut data = nid(hwnd);
            data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            data.uCallbackMessage = WM_TRAY;
            data.hIcon = HICON(ICONS[state as usize].load(Ordering::Relaxed) as *mut _);
            put(&mut data.szTip, &tip);
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            *LAST_TIP.lock().unwrap() = tip;
            SHOWN.store(state, Ordering::Relaxed);
            STATE.store(state, Ordering::Relaxed);

            // one second is plenty for a tooltip and costs nothing
            SetTimer(hwnd, 1, 1000, None);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            remove();
        });
    }
}

#[cfg(windows)]
pub use imp::{claim_single_instance, remove, start};

/// Fenstergriff des Hauptfensters - fuer die Titelleistenfarbe.
#[cfg(windows)]
pub fn main_window() -> Option<windows::Win32::Foundation::HWND> {
    unsafe { imp::main_hwnd() }
}

#[cfg(not(windows))]
pub fn main_window() -> Option<()> {
    None
}

#[cfg(windows)]
pub fn show_window() {
    unsafe { imp::show_main() }
}

#[cfg(windows)]
pub fn hide_window() {
    unsafe { imp::hide_main() }
}

#[cfg(windows)]
pub fn balloon(title: &str, text: &str) {
    unsafe { imp::balloon(title, text) }
}

#[cfg(not(windows))]
pub fn start(shared: Arc<Shared>) {
    let _ = SHARED.set(shared);
}
#[cfg(not(windows))]
pub fn claim_single_instance(_agent: bool) -> bool {
    true
}
#[cfg(not(windows))]
pub fn remove() {}
#[cfg(not(windows))]
pub fn show_window() {}
#[cfg(not(windows))]
pub fn hide_window() {}
#[cfg(not(windows))]
pub fn balloon(_title: &str, _text: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_shows_the_id_once_the_relay_answered() {
        let (state, tip) = tip_for("0.10.0", "497628420", "Verbinde...", "", false);
        assert_eq!(state, 1);
        assert!(tip.contains("497 628 420"), "{}", tip);
        assert!(tip.contains("Bereit"), "{}", tip);
    }

    #[test]
    fn tooltip_falls_back_to_the_status_while_offline() {
        let (state, tip) = tip_for("0.10.0", "", "Verbinde...", "", false);
        assert_eq!(state, 0);
        assert!(tip.contains("Verbinde..."), "{}", tip);
    }

    #[test]
    fn a_running_session_turns_the_icon_green() {
        let (state, tip) = tip_for("0.10.0", "497628420", "", "Sitzung mit 123 456 789", true);
        assert_eq!(state, 2);
        assert!(tip.contains("Sitzung mit"), "{}", tip);
    }
}
