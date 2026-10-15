//! Share a SINGLE program window - and outline in red what is shared.
//!
//! Two things that belong together:
//!
//! 1. **Window capture.** Until now the whole screen always went out. Whoever
//!    only wanted to show a table also showed their mails. So here
//!    the image comes from `PrintWindow` with `PW_RENDERFULLCONTENT` - this
//!    also draws windows correctly that compose their content on the
//!    graphics card (Chrome, Electron, modern UIs). The size
//!    is queried anew for EVERY image: so the capture follows when the
//!    window is moved or resized.
//!
//! 2. **Red frame.** A thin, click-through frame around whatever is
//!    currently going out. It is always on top, accepts no clicks
//!    (`WS_EX_TRANSPARENT`) and - this is the point - it is excluded from the own
//!    capture via `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
//!    Without that the viewer would see a second frame in the image, and
//!    with screen sharing it would even be doubled.
//!
//! Both are purely a Windows matter; on other systems the
//! functions stay empty shells, so the rest of the program does not have to fight with
//! `cfg`.

/// Ein teilbares Programmfenster.
#[derive(Clone, Debug, PartialEq)]
pub struct Fenster {
    /// Window identifier (HWND as a number - this keeps the type platform-free).
    pub kennung: isize,
    pub titel: String,
    /// Name of the program without path and extension ("firefox", "excel").
    pub programm: String,
    pub breite: u32,
    pub hoehe: u32,
}

/// All sensibly shareable windows, topmost first.
pub fn liste() -> Vec<Fenster> {
    #[cfg(windows)]
    {
        win::liste()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Aufnahme eines einzelnen Fensters oeffnen.
#[cfg(windows)]
pub fn aufnahme(kennung: isize) -> Option<Box<dyn crate::capture::Backend>> {
    win::FensterCap::neu(kennung).ok().map(|c| {
        let b: Box<dyn crate::capture::Backend> = Box::new(c);
        b
    })
}

#[cfg(not(windows))]
pub fn aufnahme(_kennung: isize) -> Option<Box<dyn crate::capture::Backend>> {
    None
}

/// A red frame around a screen area. Gone when dropped.
pub struct Rahmen {
    #[cfg(windows)]
    ziel: std::sync::Arc<std::sync::Mutex<Option<(i32, i32, i32, i32)>>>,
}

impl Rahmen {
    /// Rahmen anzeigen. `x`/`y`/`b`/`h` in Bildschirmkoordinaten.
    pub fn zeigen(x: i32, y: i32, b: i32, h: i32) -> Rahmen {
        #[cfg(windows)]
        {
            win::rahmen_starten(x, y, b, h)
        }
        #[cfg(not(windows))]
        {
            let _ = (x, y, b, h);
            Rahmen {}
        }
    }

    /// Move the frame to a new area (window moved).
    pub fn setzen(&self, x: i32, y: i32, b: i32, h: i32) {
        #[cfg(windows)]
        if let Ok(mut z) = self.ziel.lock() {
            *z = Some((x, y, b, h));
        }
        #[cfg(not(windows))]
        let _ = (x, y, b, h);
    }
}

impl Drop for Rahmen {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Ok(mut z) = self.ziel.lock() {
            // None tells the thread: stop and destroy the window.
            *z = None;
        }
    }
}

/// Wo liegt dieses Fenster gerade? (x, y, Breite, Hoehe) in Bildpunkten.
pub fn lage(kennung: isize) -> Option<(i32, i32, i32, i32)> {
    #[cfg(windows)]
    {
        win::lage(kennung)
    }
    #[cfg(not(windows))]
    {
        let _ = kennung;
        None
    }
}

#[cfg(windows)]
mod win {
    use super::{Fenster, Rahmen};
    use anyhow::{anyhow, Result};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Gdi::{
        CombineRgn, CreateCompatibleDC, CreateDIBSection, CreateRectRgn, CreateSolidBrush,
        DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, RGN_DIFF, SetWindowOrgEx, SetWindowRgn,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    // ------------------------------------------------------------ auflisten

    struct Sammler {
        raus: Vec<Fenster>,
        eigen: u32,
    }

    // SAFETY: window enumeration callback; the signature and `system` ABI match WNDENUMPROC. `lp`
    // is the address of the `Sammler` that `liste` created on its stack and passed through
    // EnumWindows, which runs the callback synchronously on the same thread; the pointer is
    // therefore valid, properly aligned and not aliased by anything else during the call.
    unsafe extern "system" fn sammeln(hwnd: HWND, lp: LPARAM) -> windows::Win32::Foundation::BOOL {
        let s = &mut *(lp.0 as *mut Sammler);
        if let Some(f) = pruefen(hwnd, s.eigen) {
            s.raus.push(f);
        }
        true.into()
    }

    // SAFETY: caller contract: none beyond a normal Win32 thread (stale or foreign window handles
    // make the Win32 calls fail, not misbehave). `versteckt` is a local u32 and the size passed to
    // DwmGetWindowAttribute is 4, matching DWMWA_CLOAKED; `pid` and `puffer` are locals and
    // GetWindowTextW is limited to the slice length. Only `n` UTF-16 units reported by the API are
    // read.
    /// Is this window fit for sharing? Everything else would be noise in the
    /// list: invisible helper windows, minimised programs, the empty
    /// shells of UWP applications and - important - our OWN window
    /// (sharing that would create a hall-of-mirrors effect).
    unsafe fn pruefen(hwnd: HWND, eigen: u32) -> Option<Fenster> {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        // Hidden by the window manager (UWP keeps zombie entries around).
        let mut versteckt = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut versteckt as *mut u32 as *mut _,
            4,
        )
        .is_ok()
            && versteckt != 0
        {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == eigen {
            return None;
        }
        let mut puffer = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut puffer);
        if n <= 0 {
            return None;
        }
        let titel = String::from_utf16_lossy(&puffer[..n as usize]);
        let (_, _, b, h) = rahmen_rect(hwnd)?;
        // Tiny windows are almost always helper constructs.
        if b < 120 || h < 80 {
            return None;
        }
        Some(Fenster {
            kennung: hwnd.0 as isize,
            titel,
            programm: programmname(pid),
            breite: b as u32,
            hoehe: h as u32,
        })
    }

    fn programmname(pid: u32) -> String {
        // SAFETY: `h` is a process handle opened by OpenProcess in this block and closed exactly
        // once on every path after the open. `puffer` is a local array, `len` its length in UTF-16
        // units as QueryFullProcessImageNameW expects, and the API updates `len` to the number of
        // characters written (without the NUL), so the slice `puffer[..len]` stays inside the
        // array.
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut puffer = [0u16; 260];
            let mut len = puffer.len() as u32;
            let name = if QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(puffer.as_mut_ptr()),
                &mut len,
            )
            .is_ok()
            {
                let voll = String::from_utf16_lossy(&puffer[..len as usize]);
                std::path::Path::new(&voll)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let _ = CloseHandle(h);
            name
        }
    }

    // SAFETY: caller contract: none beyond a normal Win32 thread. `r` is a local RECT and the size
    // passed to DwmGetWindowAttribute is exactly `size_of::<RECT>()`, matching
    // DWMWA_EXTENDED_FRAME_BOUNDS; GetWindowRect writes only into `r`.
    /// The VISIBLE bounds of a window.
    ///
    /// Since Windows Vista `GetWindowRect` returns too much: the invisible
    /// shadow border counts. Whoever captures by it shares a black strip
    /// on three sides. `DWMWA_EXTENDED_FRAME_BOUNDS` gives the
    /// edges you really see.
    unsafe fn rahmen_rect(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
        let mut r = RECT::default();
        let dwm = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if dwm.is_err() && GetWindowRect(hwnd, &mut r).is_err() {
            return None;
        }
        let b = r.right - r.left;
        let h = r.bottom - r.top;
        if b <= 0 || h <= 0 {
            return None;
        }
        Some((r.left, r.top, b, h))
    }

    pub fn lage(kennung: isize) -> Option<(i32, i32, i32, i32)> {
        // SAFETY: the argument is just an integer window handle taken from the window list; IsWindow
        // validates it first, and a handle reused by another window only changes which rectangle is
        // returned. No pointers other than the local ones in `rahmen_rect` are involved.
        unsafe {
            let hwnd = HWND(kennung as *mut core::ffi::c_void);
            if !IsWindow(hwnd).as_bool() {
                return None;
            }
            rahmen_rect(hwnd)
        }
    }

    pub fn liste() -> Vec<Fenster> {
        // SAFETY: `s` is a local `Sammler` that outlives the call; its address is passed as LPARAM
        // to `sammeln`, which casts it back to `&mut Sammler`. EnumWindows is synchronous, so the
        // pointer is valid for every callback and nothing else uses `s` meanwhile; the results are
        // read only after it returned.
        unsafe {
            let mut s = Sammler {
                raus: Vec::new(),
                eigen: windows::Win32::System::Threading::GetCurrentProcessId(),
            };
            let _ = EnumWindows(Some(sammeln), LPARAM(&mut s as *mut Sammler as isize));
            s.raus
        }
    }

    // -------------------------------------------------------------- aufnehmen

    /// Aufnahme eines Fensters ueber `PrintWindow`.
    pub struct FensterCap {
        hwnd: HWND,
        mem: HDC,
        bmp: HBITMAP,
        bits: *mut u8,
        buf: Vec<u8>,
        w: u32,
        h: u32,
        x: i32,
        y: i32,
        /// Inside the window frame the image starts at this position -
        /// PrintWindow draws from GetWindowRect, but the visible bounds
        /// lie further inside.
        rand_x: i32,
        rand_y: i32,
    }

    impl FensterCap {
        pub fn neu(kennung: isize) -> Result<Self> {
            let hwnd = HWND(kennung as *mut core::ffi::c_void);
            // SAFETY: the window handle is only validated with IsWindow before use. `c` is fully
            // initialised with null/default GDI handles before `flaeche_bauen` runs, so `Drop` is
            // safe on every error path. All pointers used inside the callees are locals.
            unsafe {
                if !IsWindow(hwnd).as_bool() {
                    return Err(anyhow!("das Fenster gibt es nicht mehr"));
                }
                let (_, _, b, h) = rahmen_rect(hwnd).ok_or_else(|| anyhow!("Fenster ohne Groesse"))?;
                let mut c = FensterCap {
                    hwnd,
                    mem: HDC::default(),
                    bmp: HBITMAP::default(),
                    bits: std::ptr::null_mut(),
                    buf: Vec::new(),
                    w: 0,
                    h: 0,
                    x: 0,
                    y: 0,
                    rand_x: 0,
                    rand_y: 0,
                };
                c.flaeche_bauen(b as u32, h as u32)?;
                Ok(c)
            }
        }

        // SAFETY: caller contract: `&mut self` gives exclusive access, and the capture thread that
        // owns this backend must be the one calling. The previous bitmap and DC are freed first via
        // `flaeche_freigeben`, which resets the handles, so nothing is freed twice. The screen DC
        // from GetDC is released on every path, `info` and `bits` are locals that outlive
        // CreateDIBSection, and `bits` points to a top-down 32 bpp section of `b * h * 4` bytes
        // which stays valid until `bmp` is deleted. `self.buf` is allocated with exactly the same
        // length, which is the invariant `next` relies on for its copy.
        /// Create a memory bitmap in the new size. Called anew on every
        /// size change of the window - otherwise the image would be cut off
        /// after the first drag at the corner.
        unsafe fn flaeche_bauen(&mut self, b: u32, h: u32) -> Result<()> {
            let b = b.max(2);
            let h = h.max(2);
            self.flaeche_freigeben();
            let bildschirm = GetDC(HWND::default());
            let mem = CreateCompatibleDC(bildschirm);
            if mem.is_invalid() {
                ReleaseDC(HWND::default(), bildschirm);
                return Err(anyhow!("kein Speicher-DC"));
            }
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: b as i32,
                // negative = top to bottom, like everywhere else here
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(bildschirm, &info, DIB_RGB_COLORS, &mut bits, None, 0);
            ReleaseDC(HWND::default(), bildschirm);
            let bmp = match bmp {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(mem);
                    return Err(anyhow!("CreateDIBSection: {}", e));
                }
            };
            SelectObject(mem, HGDIOBJ(bmp.0));
            self.mem = mem;
            self.bmp = bmp;
            self.bits = bits as *mut u8;
            self.buf = vec![0u8; b as usize * h as usize * 4];
            self.w = b;
            self.h = h;
            Ok(())
        }

        // SAFETY: caller contract: `&mut self` gives exclusive access, and the bitmap/DC are owned
        // by this struct. Each handle is checked with is_invalid() before use and reset right
        // after, so a second call is a no-op, and `bits` (which points into the freed section) is
        // set to null. Known nit: the bitmap is still selected into `mem` when it is deleted, so
        // DeleteObject can fail and leak the bitmap (see roadmap); no dangling use results.
        unsafe fn flaeche_freigeben(&mut self) {
            if !self.bmp.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
                self.bmp = HBITMAP::default();
            }
            if !self.mem.is_invalid() {
                let _ = DeleteDC(self.mem);
                self.mem = HDC::default();
            }
            self.bits = std::ptr::null_mut();
        }
    }

    impl crate::capture::Backend for FensterCap {
        fn next(&mut self, _timeout_ms: u32) -> crate::capture::Next {
            // SAFETY: all handles used here (`self.hwnd`, `self.mem`) are either validated
            // (IsWindow) or owned by this backend (created in `flaeche_bauen`). `aussen` is a local
            // RECT. The DIB section is rebuilt whenever the window size changes, so `self.bits`
            // always points to `self.w * self.h * 4` readable bytes (checked non-null) and
            // `self.buf` has exactly that length; the copy is therefore in bounds for both and the
            // regions cannot overlap (GDI section vs. Vec).
            unsafe {
                if !IsWindow(self.hwnd).as_bool() {
                    return crate::capture::Next::Lost;
                }
                // When minimised PrintWindow only returns black. Better keep the
                // last good image than send black.
                if IsIconic(self.hwnd).as_bool() {
                    return crate::capture::Next::Unchanged;
                }
                let Some((x, y, b, h)) = rahmen_rect(self.hwnd) else {
                    return crate::capture::Next::Lost;
                };
                self.x = x;
                self.y = y;
                // PrintWindow draws from the OUTER window edge. The
                // offset in between is the invisible shadow border.
                let mut aussen = RECT::default();
                if GetWindowRect(self.hwnd, &mut aussen).is_ok() {
                    self.rand_x = x - aussen.left;
                    self.rand_y = y - aussen.top;
                } else {
                    self.rand_x = 0;
                    self.rand_y = 0;
                }
                if b as u32 != self.w || h as u32 != self.h {
                    if self.flaeche_bauen(b as u32, h as u32).is_err() {
                        return crate::capture::Next::Lost;
                    }
                }
                // The offset comes in via the origin shift of the DC:
                // this way the visible corner really lands at (0,0).
                SetWindowOrgEx(self.mem, self.rand_x, self.rand_y, None);
                let ok = PrintWindow(self.hwnd, self.mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT))
                    .as_bool();
                if !ok {
                    // Some windows (full-screen games) refuse this.
                    // A second attempt without the extra flag often helps.
                    if !PrintWindow(self.hwnd, self.mem, PRINT_WINDOW_FLAGS(0)).as_bool() {
                        return crate::capture::Next::Unchanged;
                    }
                }
                if self.bits.is_null() {
                    return crate::capture::Next::Lost;
                }
                std::ptr::copy_nonoverlapping(self.bits, self.buf.as_mut_ptr(), self.buf.len());
            }
            crate::capture::Next::Frame
        }

        fn frame(&self) -> (&[u8], u32, u32, bool) {
            (&self.buf, self.w, self.h, true)
        }

        fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }

        fn origin(&self) -> (i32, i32) {
            (self.x, self.y)
        }

        fn cursor(&self) -> (i32, i32, bool) {
            // SAFETY: `ci` is a local CURSORINFO with `cbSize` set as the API requires;
            // GetCursorInfo only writes into it and keeps no pointer.
            unsafe {
                let mut ci = CURSORINFO {
                    cbSize: std::mem::size_of::<CURSORINFO>() as u32,
                    ..Default::default()
                };
                if GetCursorInfo(&mut ci).is_ok() {
                    (ci.ptScreenPos.x, ci.ptScreenPos.y, ci.flags == CURSOR_SHOWING)
                } else {
                    (0, 0, false)
                }
            }
        }

        fn name(&self) -> &'static str {
            "fenster"
        }
    }

    impl Drop for FensterCap {
        fn drop(&mut self) {
            // SAFETY: Drop runs once and nothing uses the backend afterwards, so freeing its bitmap
            // and DC here is the matching cleanup for `flaeche_bauen` (the function resets the
            // handles, so it is also safe if they were never created).
            unsafe { self.flaeche_freigeben() }
        }
    }

    // ----------------------------------------------------------- roter Rahmen

    const RAHMEN_DICKE: i32 = 4;

    // SAFETY: window procedure registered in `rahmen_faden`; the signature and `system` ABI match
    // WNDPROC. It forwards all arguments unchanged to DefWindowProcW and dereferences nothing
    // itself.
    unsafe extern "system" fn rahmen_proc(
        h: HWND,
        m: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(h, m, w, l)
    }

    pub fn rahmen_starten(x: i32, y: i32, b: i32, h: i32) -> Rahmen {
        let ziel = Arc::new(Mutex::new(Some((x, y, b, h))));
        let z2 = ziel.clone();
        let _ = std::thread::Builder::new()
            .name("rahmen".into())
            // SAFETY: the closure runs `rahmen_faden` on its own thread; the thread owns the window
            // it creates and `ziel` is moved into it (an Arc), so no raw pointer or borrowed data
            // crosses the thread boundary.
            .spawn(move || unsafe { rahmen_faden(z2) });
        Rahmen { ziel }
    }

    // SAFETY: runs on a dedicated thread that creates, pumps and destroys the one window it uses
    // (all Win32 window calls therefore happen on the owning thread). The class name buffer
    // `klasse` is NUL-terminated and outlives the calls (RegisterClassW copies the name), `wc` is a
    // local, `msg` is a local MSG, and `hwnd` is checked from CreateWindowExW before use. The
    // region handles are created per update: `innen` is deleted by us, `aussen` is handed to the
    // window with SetWindowRgn, which takes ownership, so it is not freed here.
    unsafe fn rahmen_faden(ziel: Arc<Mutex<Option<(i32, i32, i32, i32)>>>) {
        let klasse: Vec<u16> = "FreeViewerTeilrahmen\0".encode_utf16().collect();
        let rot = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00_00_00_FF));
        let wc = WNDCLASSW {
            lpfnWndProc: Some(rahmen_proc),
            lpszClassName: PCWSTR(klasse.as_ptr()),
            hbrBackground: rot,
            ..Default::default()
        };
        // Registering twice is harmless (returns 0) - the frame can
        // open and close several times during a session.
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR(klasse.as_ptr()),
            PCWSTR(klasse.as_ptr()),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            None,
            None,
        ) else {
            return;
        };
        // Semi-transparent: the frame should stand out, but not dazzle.
        let _ = SetLayeredWindowAttributes(
            hwnd,
            windows::Win32::Foundation::COLORREF(0),
            210,
            LWA_ALPHA,
        );
        // THE decisive trick: out of the own capture.
        // Without it the viewer would see the frame twice - once real and
        // once captured along.
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut letzte: Option<(i32, i32, i32, i32)> = None;
        loop {
            // Process messages, otherwise the window counts as hung.
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let jetzt = ziel.lock().ok().and_then(|z| *z);
            let Some((x, y, b, h)) = jetzt else {
                let _ = DestroyWindow(hwnd);
                return;
            };
            if letzte != Some((x, y, b, h)) {
                letzte = Some((x, y, b, h));
                let d = RAHMEN_DICKE;
                let aussen = CreateRectRgn(0, 0, b + 2 * d, h + 2 * d);
                let innen = CreateRectRgn(d, d, b + d, h + d);
                CombineRgn(aussen, aussen, innen, RGN_DIFF);
                let _ = DeleteObject(HGDIOBJ(innen.0));
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    x - d,
                    y - d,
                    b + 2 * d,
                    h + 2 * d,
                    SWP_NOACTIVATE,
                );
                // Windows takes over the region - do not free it ourselves.
                SetWindowRgn(hwnd, aussen, true);
            }
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
    }

    // Unused, but the warning would be annoying.
    #[allow(dead_code)]
    fn ungenutzt(_: POINT, _: &AtomicBool, _: Ordering) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohne_fenster_bleibt_die_liste_leer_statt_zu_luegen() {
        // On the build server (Linux, no screen) nothing made-up
        // may stand here - a made-up window list would be worse
        // than an empty one.
        #[cfg(not(windows))]
        assert!(liste().is_empty());
        #[cfg(windows)]
        {
            // Unter Windows darf sie voll sein, aber niemals kaputte
            // Eintraege enthalten.
            for f in liste() {
                assert!(f.kennung != 0, "Fenster ohne Kennung");
                assert!(f.breite >= 120 && f.hoehe >= 80, "Zwergfenster in der Liste");
            }
        }
    }

    #[test]
    fn lage_eines_unbekannten_fensters_ist_none() {
        assert_eq!(lage(0), None);
    }
}
