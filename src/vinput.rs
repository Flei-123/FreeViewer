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
/// Relative Maus (Spielmodus). Die Tastatur wird auch in der Fernwartung
/// komplett uebernommen, die Maus aber nur im Spielmodus umgestellt.
static REL: AtomicBool = AtomicBool::new(false);
static CX: AtomicI32 = AtomicI32::new(0);
static CY: AtomicI32 = AtomicI32::new(0);
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
/// Timestamps of the last right-Ctrl presses (for the triple tap).
static PRESSES: std::sync::Mutex<Vec<std::time::Instant>> = std::sync::Mutex::new(Vec::new());

/// Right Ctrl was pressed while a session is running.
///
/// One tap always hands the input back - that is the way out of the remote
/// machine and it works in both profiles, not just in game mode. Three taps
/// within 1.5 seconds are the emergency exit and end the session, for the
/// moment when the remote side hangs and the picture no longer reacts.
fn host_key_pressed() {
    let sh = match SHARED.get() {
        Some(s) => s,
        None => return,
    };
    if !sh.connected.load(Ordering::Relaxed) {
        return;
    }
    set_active(false);
    let now = std::time::Instant::now();
    let mut p = PRESSES.lock().unwrap();
    p.retain(|t| now.duration_since(*t) < std::time::Duration::from_millis(1500));
    p.push(now);
    let code = if p.len() >= 3 {
        p.clear();
        2
    } else {
        1
    };
    sh.escape.store(code, Ordering::Relaxed);
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Maus relativ messen (nur Spielmodus).
pub fn set_relative(on: bool) {
    REL.store(on, Ordering::Relaxed);
}

pub fn is_relative() -> bool {
    REL.load(Ordering::Relaxed)
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
        if code >= 0 {
            let info = &*(l.0 as *const KBDLLHOOKSTRUCT);
            let injected = info.flags.0 & LLKHF_INJECTED != 0;
            let vk_any = info.vkCode as u16;
            let down_any = w.0 as u32 == WM_KEYDOWN || w.0 as u32 == WM_SYSKEYDOWN;
            // The host key works even when the input is not grabbed, so a
            // session can always be left - it stays local either way.
            if !injected && vk_any == 0xA3 && down_any && !ACTIVE.load(Ordering::Relaxed) {
                super::host_key_pressed();
            }
        }
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
                        super::host_key_pressed();
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
                let active = ACTIVE.load(Ordering::Relaxed) && REL.load(Ordering::Relaxed);
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
