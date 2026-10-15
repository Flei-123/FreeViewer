//! Change the resolution of the remote monitor (at the request of the accessing side).
//!
//! The viewer sends `SetResolution`, the host switches the screen
//! currently shown. This module remembers the previous resolution
//! per monitor - `restore` sets it back at the end of the session.
//! Windows only; elsewhere it is a controlled "not possible".

use crate::lockutil::LockExt;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Mutex;

/// Previous resolution per monitor index (only what we switched ourselves).
static ORIGINAL: Mutex<Option<HashMap<usize, (u32, u32)>>> = Mutex::new(None);

fn merken(index: usize, w: u32, h: u32) {
    let mut g = ORIGINAL.lock_ok();
    let map = g.get_or_insert_with(HashMap::new);
    map.entry(index).or_insert((w, h));
}

fn nehmen(index: usize) -> Option<(u32, u32)> {
    ORIGINAL.lock_ok().as_mut()?.remove(&index)
}

/// Sets monitor `index` to w x h. Errors are not fatal - the
/// caller deliberately ignores them (the session simply keeps running).
pub fn set_resolution(index: usize, w: u32, h: u32) -> Result<()> {
    imp::set(index, w, h)
}

/// Restores the previous resolution if we had changed it.
pub fn restore(index: usize) {
    if let Some((w, h)) = nehmen(index) {
        let _ = imp::set_keep(index, w, h);
    }
}

/// What the monitor can really do (widest first, duplicates removed).
/// Only this way do resolutions end up in the selection menu that Windows also accepts.
pub fn supported(index: usize) -> Vec<(u32, u32)> {
    imp::supported(index)
}

// --------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW,
        DISPLAY_DEVICEW, CDS_UPDATEREGISTRY, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT,
        DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
    };
    use windows::Win32::Graphics::Gdi::ENUM_DISPLAY_SETTINGS_MODE;

    /// Index for EnumDisplaySettingsW: go through all modes.
    const fn ENUM_INDEX(i: u32) -> ENUM_DISPLAY_SETTINGS_MODE {
        ENUM_DISPLAY_SETTINGS_MODE(i)
    }

    /// Bit 1 in StateFlags: this device is really attached to the desktop.
    const ATTACHED_TO_DESKTOP: u32 = 0x0000_0001;

    /// The device name of the `index`-th CONNECTED screen.
    ///
    /// WHY it is filtered: EnumDisplayDevicesW counts ALL display devices,
    /// including unplugged ones and mirror drivers. The index the viewer
    /// sends, however, counts only the screens the capture sees
    /// (DXGI counts only connected outputs). Unfiltered,
    /// the index therefore pointed to a dead device - and Windows reported
    /// "current resolution unreadable", the switch was rejected every
    /// time. Exactly this error was in Justin's log.
    fn devicename(index: usize) -> Result<Vec<u16>> {
        let mut gefunden = 0usize;
        for i in 0..64u32 {
            let mut dd = DISPLAY_DEVICEW::default();
            dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
            // SAFETY: `dd` is a local DISPLAY_DEVICEW with `cb` set to its size.
            let ok = unsafe { EnumDisplayDevicesW(None, i, &mut dd, 0) };
            if !ok.as_bool() {
                break;
            }
            if dd.StateFlags & ATTACHED_TO_DESKTOP == 0 {
                continue;
            }
            if gefunden == index {
                let len = dd
                    .DeviceName
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(dd.DeviceName.len());
                return Ok(dd.DeviceName[..len].to_vec());
            }
            gefunden += 1;
        }
        Err(anyhow!(
            "Monitor {} nicht gefunden ({} angeschlossen)",
            index + 1,
            gefunden
        ))
    }

    fn current(index: usize) -> Result<(u32, u32)> {
        let name = devicename(index)?;
        let mut dm = DEVMODEW::default();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        // SAFETY: `name` is a NUL-terminated UTF-16 buffer; `dm` is a local DEVMODEW with `dmSize` set.
        let ok = unsafe {
            EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm)
        };
        if !ok.as_bool() {
            return Err(anyhow!("aktuelle Aufloesung unlesbar"));
        }
        Ok((dm.dmPelsWidth, dm.dmPelsHeight))
    }

    pub fn supported(index: usize) -> Vec<(u32, u32)> {
        let Ok(name) = devicename(index) else {
            return Vec::new();
        };
        let mut out: Vec<(u32, u32)> = Vec::new();
        let mut i = 0u32;
        loop {
            let mut dm = DEVMODEW::default();
            dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
            // SAFETY: `name` is a NUL-terminated UTF-16 buffer; `dm` is a local DEVMODEW with `dmSize` set.
            let ok = unsafe {
                EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_INDEX(i), &mut dm)
            };
            if !ok.as_bool() {
                break;
            }
            i += 1;
            let paar = (dm.dmPelsWidth, dm.dmPelsHeight);
            if paar.0 >= 800 && !out.contains(&paar) {
                out.push(paar);
            }
            if i > 512 {
                break;
            }
        }
        out.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1)));
        while out.len() > 24 {
            out.pop();
        }
        out
    }

    pub fn set(index: usize, w: u32, h: u32) -> Result<()> {
        let (ow, oh) = current(index)?;
        if (ow, oh) == (w, h) {
            return Ok(());
        }
        super::merken(index, ow, oh);
        set_keep(index, w, h)
    }

    pub fn set_keep(index: usize, w: u32, h: u32) -> Result<()> {
        let name = devicename(index)?;
        let mut dm = DEVMODEW::default();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT;
        dm.dmPelsWidth = w;
        dm.dmPelsHeight = h;
        // SAFETY: `name` is a NUL-terminated UTF-16 buffer and `dm` a local DEVMODEW, both alive during the call.
        let r = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(name.as_ptr()),
                Some(&dm),
                None,
                CDS_UPDATEREGISTRY,
                None,
            )
        };
        if r == DISP_CHANGE_SUCCESSFUL {
            // After a mode change the running screen capture is
            // invalid (the duplication then reports 0x887A0001). The
            // capturer rebuilds itself on errors - here we only note
            // that there will be a hiccup shortly.
            crate::capture::log_line(&format!("Aufloesung auf {}x{} gestellt", w, h));
            Ok(())
        } else {
            Err(anyhow!("Windows lehnt {}x{} ab (Code {})", w, h, r.0))
        }
    }
}

// ------------------------------------------------------------- andere OS

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn supported(_index: usize) -> Vec<(u32, u32)> {
        Vec::new()
    }

    pub fn set(_index: usize, _w: u32, _h: u32) -> Result<()> {
        Err(anyhow!("Aufloesung aendern geht nur unter Windows"))
    }

    pub fn set_keep(_index: usize, _w: u32, _h: u32) -> Result<()> {
        Err(anyhow!("Aufloesung aendern geht nur unter Windows"))
    }
}
