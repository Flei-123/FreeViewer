//! Viewer side raw input capture ("Spielmodus").
//!
//! Two things a normal GUI cannot do are needed to remote control a game:
//!
//! 1. **Relative mouse motion.** A game reads raw mouse counts, not the
//!    cursor position, and hides the pointer. So we lock our own pointer to
//!    the middle of the picture, measure how far the user moved it and send
//!    that delta - exactly like Parsec/Moonlight do.
//! 2. **The whole keyboard.** Windows itself eats Alt+Tab, the Windows key,
//!    Alt+F4 ... A low level keyboard hook (WH_KEYBOARD_LL) sees those keys
//!    first, forwards them to the remote machine and swallows them locally.
//!
//! Right Ctrl is the escape hatch: it always stays local and switches the
//! grab off, so the user can never lock himself out. Letting go also tells
//! the host to release every key and button we might still hold, otherwise a
//! "W" pressed for walking would keep running forever on the remote machine.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

use crate::proto::{Msg, SPECIAL_RELEASE};
use crate::shared::Shared;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static CX: AtomicI32 = AtomicI32::new(0);
static CY: AtomicI32 = AtomicI32::new(0);
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Center of the remote picture in physical screen pixels - the pointer is
/// warped back here after every sample.
pub fn set_center(x: i32, y: i32) {
    CX.store(x, Ordering::Relaxed);
    CY.store(y, Ordering::Relaxed);
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const LLKHF_EXTENDED: u32 = 0x01;
    const LLKHF_INJECTED: u32 = 0x10;

    unsafe extern "system" fn hook_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code >= 0 && ACTIVE.load(Ordering::Relaxed) {
            let info = &*(l.0 as *const KBDLLHOOKSTRUCT);
            let injected = info.flags.0 & LLKHF_INJECTED != 0;
            if !injected {
                let vk = info.vkCode as u16;
                let ext = info.flags.0 & LLKHF_EXTENDED != 0;
                let down = w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN;

                // right ctrl = host key, never forwarded
                if vk == 0xA3 {
                    if down {
                        super::set_active(false);
                    }
                    return LRESULT(1);
                }
                if let Some(sh) = SHARED.get() {
                    sh.send_input(Msg::KeyVk { vk, ext, down });
                }
                return LRESULT(1); // swallow locally
            }
        }
        CallNextHookEx(None, code, w, l)
    }

    pub fn init(shared: Arc<Shared>) {
        let _ = SHARED.set(shared);
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || unsafe {
            // the hook must be owned by a thread that pumps messages
            let _hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
                Ok(h) => h,
                Err(_) => return,
            };
            let mut last_active = false;
            let mut msg = MSG::default();
            loop {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let active = ACTIVE.load(Ordering::Relaxed);
                if active {
                    let cx = CX.load(Ordering::Relaxed);
                    let cy = CY.load(Ordering::Relaxed);
                    if !last_active {
                        let _ = SetCursorPos(cx, cy);
                    } else {
                        let mut p = POINT::default();
                        if GetCursorPos(&mut p).is_ok() {
                            let (dx, dy) = (p.x - cx, p.y - cy);
                            if dx != 0 || dy != 0 {
                                if let Some(sh) = SHARED.get() {
                                    sh.send_input(Msg::MouseDelta { dx, dy });
                                }
                                let _ = SetCursorPos(cx, cy);
                            }
                        }
                    }
                }
                last_active = active;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        });
    }
}

#[cfg(windows)]
pub use imp::init;

#[cfg(not(windows))]
pub fn init(shared: Arc<Shared>) {
    let _ = SHARED.set(shared);
}

/// Turns the grab on or off. Letting go always asks the host to release every
/// key and mouse button it still holds for us.
pub fn set_active(on: bool) {
    let was = ACTIVE.swap(on, Ordering::Relaxed);
    if was && !on {
        if let Some(sh) = SHARED.get() {
            sh.send_input(Msg::Special {
                code: SPECIAL_RELEASE,
            });
        }
    }
}
