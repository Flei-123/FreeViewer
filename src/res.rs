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

// --------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW,
        DISPLAY_DEVICEW, CDS_UPDATEREGISTRY, DISP_CHANGE_SUCCESSFUL, ENUM_CURRENT_SETTINGS,
    };

    const DM_PELSWIDTH: u32 = 0x0008_0000;
    const DM_PELSHEIGHT: u32 = 0x0010_0000;

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

    pub fn set(_index: usize, _w: u32, _h: u32) -> Result<()> {
        Err(anyhow!("Aufloesung aendern geht nur unter Windows"))
    }

    pub fn set_keep(_index: usize, _w: u32, _h: u32) -> Result<()> {
        Err(anyhow!("Aufloesung aendern geht nur unter Windows"))
    }
}
