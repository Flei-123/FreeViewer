//! Aufloesung des fernen Monitors aendern (auf Wunsch des Zugreifenden).
//!
//! Der Viewer schickt `SetResolution`, der Host stellt den gerade
//! gezeigten Bildschirm um. Die vorherige Aufloesung merkt sich dieses
//! Modul pro Monitor - `restore` setzt sie am Ende der Sitzung wieder.
//! Windows only; anderswo ist es ein kontrolliertes "geht nicht".

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Mutex;

/// Vorherige Aufloesung pro Monitor-Index (nur was wir selbst umgestellt haben).
static ORIGINAL: Mutex<Option<HashMap<usize, (u32, u32)>>> = Mutex::new(None);

fn merken(index: usize, w: u32, h: u32) {
    let mut g = ORIGINAL.lock().unwrap();
    let map = g.get_or_insert_with(HashMap::new);
    map.entry(index).or_insert((w, h));
}

fn nehmen(index: usize) -> Option<(u32, u32)> {
    ORIGINAL.lock().unwrap().as_mut()?.remove(&index)
}

/// Stellt den Monitor `index` auf w x h. Fehler sind nicht fatal - der
/// Anrufer ignoriert sie bewusst (die Sitzung laeuft einfach weiter).
pub fn set_resolution(index: usize, w: u32, h: u32) -> Result<()> {
    imp::set(index, w, h)
}

/// Stellt die vorherige Aufloesung wieder her, falls wir sie geaendert hatten.
pub fn restore(index: usize) {
    if let Some((w, h)) = nehmen(index) {
        let _ = imp::set_keep(index, w, h);
    }
}

/// Was der Monitor wirklich kann (breiteste zuerst, Duplikate raus).
/// Nur so landen im Auswahlmenue Aufloesungen, die Windows auch annimmt.
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

    /// Index fuer EnumDisplaySettingsW: alle Modi durchgehen.
    const fn ENUM_INDEX(i: u32) -> ENUM_DISPLAY_SETTINGS_MODE {
        ENUM_DISPLAY_SETTINGS_MODE(i)
    }

    fn devicename(index: usize) -> Result<Vec<u16>> {
        let mut dd = DISPLAY_DEVICEW::default();
        dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        let ok = unsafe { EnumDisplayDevicesW(None, index as u32, &mut dd, 0) };
        if !ok.as_bool() {
            return Err(anyhow!("Monitor {} nicht gefunden", index + 1));
        }
        let len = dd
            .DeviceName
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(dd.DeviceName.len());
        Ok(dd.DeviceName[..len].to_vec())
    }

    fn current(index: usize) -> Result<(u32, u32)> {
        let name = devicename(index)?;
        let mut dm = DEVMODEW::default();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
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
