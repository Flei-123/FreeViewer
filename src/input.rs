//! Host side input injection.
//!
//! On Windows everything goes through `SendInput`, because that is the only
//! way to get *relative* mouse motion (games read raw input / DirectInput and
//! ignore a warped cursor) and to reproduce real key combinations including
//! the Windows key. `enigo` stays as the portable fallback for other systems
//! and for the legacy unicode key path.

use crate::proto;

/// Rectangle of the shared screen inside the virtual desktop (physical px).
#[derive(Clone, Copy, Debug)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Default for ScreenRect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }
    }
}

pub struct Injector {
    #[allow(dead_code)]
    enigo: Option<enigo::Enigo>,
    #[cfg(windows)]
    held: std::collections::HashSet<u16>,
}

impl Injector {
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        {
            let _ = enigo::set_dpi_awareness();
        }
        // enigo is only needed for the legacy unicode/named key path
        let enigo = enigo::Enigo::new(&enigo::Settings::default()).ok();
        Self {
            enigo,
            #[cfg(windows)]
            held: std::collections::HashSet::new(),
        }
    }
}

// ------------------------------------------------------------------ windows --

#[cfg(windows)]
mod imp {
    use super::{Injector, ScreenRect};
    use crate::proto;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    fn send(inputs: &[INPUT]) {
        unsafe {
            SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) {
        let mut inp = INPUT {
            r#type: INPUT_MOUSE,
            ..Default::default()
        };
        inp.Anonymous.mi = MOUSEINPUT {
            dx,
            dy,
            mouseData: data as u32,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        send(&[inp]);
    }

    fn key(vk: u16, ext: bool, down: bool) {
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let mut inp = INPUT {
            r#type: INPUT_KEYBOARD,
            ..Default::default()
        };
        inp.Anonymous.ki = KEYBDINPUT {
            wVk: VIRTUAL_KEY(vk),
            wScan: scan,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        send(&[inp]);
    }

    fn tap(vk: u16, ext: bool) {
        key(vk, ext, true);
        key(vk, ext, false);
    }

    /// Ctrl+Alt+Del can only be triggered through sas.dll and only when the
    /// caller is allowed to (a service, or the "SoftwareSASGeneratedByServices"
    /// policy). Returns false when the call was not possible.
    fn send_sas() -> bool {
        unsafe {
            let name = windows::core::w!("sas.dll");
            let lib: HMODULE = match LoadLibraryW(name) {
                Ok(h) if !h.is_invalid() => h,
                _ => return false,
            };
            let proc = GetProcAddress(lib, windows::core::s!("SendSAS"));
            match proc {
                Some(p) => {
                    let f: extern "system" fn(i32) = std::mem::transmute(p);
                    f(0); // 0 = called by a service / privileged process
                    true
                }
                None => false,
            }
        }
    }

    impl Injector {
        pub fn mouse_abs(&mut self, nx: i32, ny: i32, screen: ScreenRect) {
            let px = screen.x as i64 + (nx as i64 * screen.w as i64 / 10000);
            let py = screen.y as i64 + (ny as i64 * screen.h as i64 / 10000);
            unsafe {
                let vx = GetSystemMetrics(SM_XVIRTUALSCREEN) as i64;
                let vy = GetSystemMetrics(SM_YVIRTUALSCREEN) as i64;
                let vw = (GetSystemMetrics(SM_CXVIRTUALSCREEN) as i64).max(1);
                let vh = (GetSystemMetrics(SM_CYVIRTUALSCREEN) as i64).max(1);
                let ax = ((px - vx) * 65535 / vw.max(1)).clamp(0, 65535) as i32;
                let ay = ((py - vy) * 65535 / vh.max(1)).clamp(0, 65535) as i32;
                mouse(
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    ax,
                    ay,
                    0,
                );
            }
        }

        /// Raw relative motion - this is what a 3D game camera reacts to.
        pub fn mouse_delta(&mut self, dx: i32, dy: i32) {
            if dx == 0 && dy == 0 {
                return;
            }
            mouse(MOUSEEVENTF_MOVE, dx, dy, 0);
        }

        pub fn button(&mut self, b: u8, down: bool) {
            let (flags, data) = match (b, down) {
                (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (3, true) => (MOUSEEVENTF_XDOWN, 1i32),
                (3, false) => (MOUSEEVENTF_XUP, 1i32),
                (4, true) => (MOUSEEVENTF_XDOWN, 2i32),
                (4, false) => (MOUSEEVENTF_XUP, 2i32),
                (_, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (_, false) => (MOUSEEVENTF_LEFTUP, 0),
            };
            mouse(flags, 0, 0, data);
        }

        pub fn wheel(&mut self, lines: i32) {
            mouse(MOUSEEVENTF_WHEEL, 0, 0, lines * 120);
        }

        pub fn key_vk(&mut self, vk: u16, ext: bool, down: bool) {
            if down {
                self.held.insert(vk);
            } else {
                self.held.remove(&vk);
            }
            key(vk, ext, down);
        }

        /// Releases everything the remote side still holds down, so a dropped
        /// session cannot leave Ctrl or W stuck on the host.
        pub fn release_all(&mut self) {
            let held: Vec<u16> = self.held.drain().collect();
            for vk in held {
                key(vk, false, false);
            }
            for vk in [
                VK_SHIFT.0,
                VK_CONTROL.0,
                VK_MENU.0,
                VK_LWIN.0,
                VK_RWIN.0,
                VK_LSHIFT.0,
                VK_RSHIFT.0,
                VK_LCONTROL.0,
                VK_RCONTROL.0,
                VK_LMENU.0,
                VK_RMENU.0,
            ] {
                key(vk, false, false);
            }
        }

        pub fn special(&mut self, code: u8) -> &'static str {
            match code {
                proto::SPECIAL_CAD => {
                    if send_sas() {
                        "Ctrl+Alt+Entf gesendet"
                    } else {
                        "Ctrl+Alt+Entf nicht erlaubt (Host muss als Dienst laufen)"
                    }
                }
                proto::SPECIAL_TASKMGR => {
                    key(VK_CONTROL.0, false, true);
                    key(VK_SHIFT.0, false, true);
                    tap(VK_ESCAPE.0, false);
                    key(VK_SHIFT.0, false, false);
                    key(VK_CONTROL.0, false, false);
                    "Task-Manager"
                }
                proto::SPECIAL_WIN => {
                    tap(VK_LWIN.0, true);
                    "Windows-Taste"
                }
                proto::SPECIAL_ALTTAB => {
                    key(VK_MENU.0, false, true);
                    tap(VK_TAB.0, false);
                    key(VK_MENU.0, false, false);
                    "Alt+Tab"
                }
                proto::SPECIAL_LOCK => {
                    key(VK_LWIN.0, true, true);
                    tap(0x4C, false); // L
                    key(VK_LWIN.0, true, false);
                    "Gesperrt"
                }
                proto::SPECIAL_RELEASE => {
                    self.release_all();
                    for b in 0..5u8 {
                        self.button(b, false);
                    }
                    "Eingaben freigegeben"
                }
                _ => "unbekannt",
            }
        }
    }

    /// Maps the portable named-key protocol onto virtual key codes so the old
    /// path also profits from SendInput.
    pub fn named_to_vk(code: u32) -> Option<(u16, bool)> {
        use crate::proto as p;
        let v = match code {
            p::KEY_BACKSPACE => (VK_BACK.0, false),
            p::KEY_ENTER => (VK_RETURN.0, false),
            p::KEY_TAB => (VK_TAB.0, false),
            p::KEY_ESCAPE => (VK_ESCAPE.0, false),
            p::KEY_LEFT => (VK_LEFT.0, true),
            p::KEY_RIGHT => (VK_RIGHT.0, true),
            p::KEY_UP => (VK_UP.0, true),
            p::KEY_DOWN => (VK_DOWN.0, true),
            p::KEY_DELETE => (VK_DELETE.0, true),
            p::KEY_HOME => (VK_HOME.0, true),
            p::KEY_END => (VK_END.0, true),
            p::KEY_PAGEUP => (VK_PRIOR.0, true),
            p::KEY_PAGEDOWN => (VK_NEXT.0, true),
            p::KEY_INSERT => (VK_INSERT.0, true),
            p::KEY_SPACE => (VK_SPACE.0, false),
            p::KEY_SHIFT => (VK_SHIFT.0, false),
            p::KEY_CTRL => (VK_CONTROL.0, false),
            p::KEY_ALT => (VK_MENU.0, false),
            p::KEY_META => (VK_LWIN.0, true),
            30..=41 => ((VK_F1.0 as u32 + (code - 30)) as u16, false),
            _ => return None,
        };
        Some(v)
    }


}

#[cfg(windows)]
pub use imp::named_to_vk;

// -------------------------------------------------------------- other OSes --

#[cfg(not(windows))]
impl Injector {
    pub fn mouse_abs(&mut self, nx: i32, ny: i32, screen: ScreenRect) {
        use enigo::{Coordinate, Mouse};
        if let Some(e) = self.enigo.as_mut() {
            let px = screen.x + (nx as i64 * screen.w as i64 / 10000) as i32;
            let py = screen.y + (ny as i64 * screen.h as i64 / 10000) as i32;
            let _ = e.move_mouse(px, py, Coordinate::Abs);
        }
    }
    pub fn mouse_delta(&mut self, dx: i32, dy: i32) {
        use enigo::{Coordinate, Mouse};
        if let Some(e) = self.enigo.as_mut() {
            let _ = e.move_mouse(dx, dy, Coordinate::Rel);
        }
    }
    pub fn button(&mut self, b: u8, down: bool) {
        use enigo::{Button, Direction, Mouse};
        if let Some(e) = self.enigo.as_mut() {
            let btn = match b {
                1 => Button::Right,
                2 => Button::Middle,
                3 => Button::Back,
                4 => Button::Forward,
                _ => Button::Left,
            };
            let _ = e.button(
                btn,
                if down {
                    Direction::Press
                } else {
                    Direction::Release
                },
            );
        }
    }
    pub fn wheel(&mut self, lines: i32) {
        use enigo::{Axis, Mouse};
        if let Some(e) = self.enigo.as_mut() {
            let _ = e.scroll(-lines, Axis::Vertical);
        }
    }
    pub fn key_vk(&mut self, _vk: u16, _ext: bool, _down: bool) {}
    pub fn release_all(&mut self) {}
    pub fn special(&mut self, _code: u8) -> &'static str {
        "auf diesem System nicht unterstuetzt"
    }
}

#[cfg(not(windows))]
pub fn named_to_vk(_code: u32) -> Option<(u16, bool)> {
    None
}

/// Portable path: named keys and unicode characters through enigo.
impl Injector {
    pub fn key_portable(&mut self, code: u32, named: bool, down: bool) {
        use enigo::{Direction, Keyboard};
        let dir = if down {
            Direction::Press
        } else {
            Direction::Release
        };
        let key = if named {
            named_key(code)
        } else {
            char::from_u32(code).map(enigo::Key::Unicode)
        };
        if let (Some(k), Some(e)) = (key, self.enigo.as_mut()) {
            let _ = e.key(k, dir);
        }
    }
}

fn named_key(code: u32) -> Option<enigo::Key> {
    use enigo::Key;
    let k = match code {
        proto::KEY_BACKSPACE => Key::Backspace,
        proto::KEY_ENTER => Key::Return,
        proto::KEY_TAB => Key::Tab,
        proto::KEY_ESCAPE => Key::Escape,
        proto::KEY_LEFT => Key::LeftArrow,
        proto::KEY_RIGHT => Key::RightArrow,
        proto::KEY_UP => Key::UpArrow,
        proto::KEY_DOWN => Key::DownArrow,
        proto::KEY_DELETE => Key::Delete,
        proto::KEY_HOME => Key::Home,
        proto::KEY_END => Key::End,
        proto::KEY_PAGEUP => Key::PageUp,
        proto::KEY_PAGEDOWN => Key::PageDown,
        // Der Mac hat keine Einfg-Taste, deshalb kennt enigo dort auch kein
        // Key::Insert. Auf macOS schicken wir stattdessen den rohen Tastencode
        // 0x72 (Help/Insert) - die Taste existiert im Tastaturlayout, nur nicht
        // als benannte Variante.
        #[cfg(not(target_os = "macos"))]
        proto::KEY_INSERT => Key::Insert,
        #[cfg(target_os = "macos")]
        proto::KEY_INSERT => Key::Other(0x72),
        proto::KEY_SPACE => Key::Space,
        proto::KEY_SHIFT => Key::Shift,
        proto::KEY_CTRL => Key::Control,
        proto::KEY_ALT => Key::Alt,
        proto::KEY_META => Key::Meta,
        30 => Key::F1,
        31 => Key::F2,
        32 => Key::F3,
        33 => Key::F4,
        34 => Key::F5,
        35 => Key::F6,
        36 => Key::F7,
        37 => Key::F8,
        38 => Key::F9,
        39 => Key::F10,
        40 => Key::F11,
        41 => Key::F12,
        _ => return None,
    };
    Some(k)
}
