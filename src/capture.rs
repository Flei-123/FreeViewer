//! Screen capture backends.
//!
//! Two implementations live here:
//!
//! * `dxgi` - DXGI Desktop Duplication (Windows 8+). The desktop compositor
//!   hands us the finished frame as a GPU texture, tells us *which* rectangles
//!   changed and blocks until a new frame actually exists. We only read back
//!   the changed rectangles over PCIe, so an idle desktop costs almost
//!   nothing. This is the same API Parsec/OBS use.
//! * `fallback` - the old `xcap` screenshot path. Used on non-Windows, in
//!   session 0 (services have no interactive desktop) and whenever the
//!   duplication API refuses to start.
//!
//! The capture thread owns the backend; it is deliberately not `Send`.

use std::time::Instant;

/// What one `next()` call produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Next {
    /// A new picture is in the backend buffer.
    Frame,
    /// Nothing changed on screen (only the cursor may have moved).
    Unchanged,
    /// The capture broke down (display mode change, session switch, ...).
    Lost,
}

pub trait Backend {
    /// Blocks for at most `timeout_ms` waiting for a new frame.
    fn next(&mut self, timeout_ms: u32) -> Next;
    /// Current picture: (pixels, width, height, `true` if BGRA instead of RGBA).
    fn frame(&self) -> (&[u8], u32, u32, bool);
    /// Physical size of the captured screen.
    fn size(&self) -> (u32, u32);
    /// Position of the captured screen inside the virtual desktop.
    fn origin(&self) -> (i32, i32);
    /// Mouse position in desktop coordinates plus visibility.
    fn cursor(&self) -> (i32, i32, bool);
    fn name(&self) -> &'static str;
    /// Scales the current frame to `dw` x `dh` **on the GPU**.
    ///
    /// `nv12 = false` returns packed RGB, `nv12 = true` returns NV12 (the
    /// format every hardware H.264 encoder wants) - in that case the GPU also
    /// does the colour conversion and only 1.5 bytes per pixel travel back
    /// over PCIe instead of 3. `None` means no hardware scaler is available,
    /// then the caller falls back to the CPU path.
    fn scaled(&mut self, dw: u32, dh: u32, nv12: bool) -> Option<&[u8]> {
        self.scaled_teil(dw, dh, nv12, (0.0, 0.0, 1.0, 1.0))
    }
    /// Wie `scaled`, aber nur ein AUSSCHNITT des Bildschirms (Anteile 0..1).
    ///
    /// WARUM auf der Grafikkarte statt im Hauptspeicher: sobald der
    /// GPU-Skalierer laeuft, wird das Vollbild NICHT mehr ins RAM
    /// zurueckgelesen (das spart PCIe-Bandbreite). Wer dann `frame()`
    /// benutzt, bekommt einen alten oder leeren Puffer - genau daran lag es,
    /// dass der scharfe Zoom ab einer gewissen Stufe schwarz wurde. Der
    /// Videoprozessor kann den Ausschnitt selbst, das kostet nichts extra.
    fn scaled_teil(
        &mut self,
        _dw: u32,
        _dh: u32,
        _nv12: bool,
        _teil: (f32, f32, f32, f32),
    ) -> Option<&[u8]> {
        None
    }
    /// True while the GPU path is in use (diagnostics).
    fn gpu_scaling(&self) -> bool {
        false
    }
}

/// One screen that can be shared. Index 0 is always the primary monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorDesc {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
}

/// Every screen attached to this machine, primary first. The order is the
/// index space used by `open_index` and by the `SetMonitor` message.
pub fn list_monitors(prefer_fast: bool) -> Vec<MonitorDesc> {
    #[cfg(windows)]
    if prefer_fast {
        let list = dxgi::describe();
        if !list.is_empty() {
            return list;
        }
    }
    let _ = prefer_fast;
    fallback::describe()
}

/// Opens the best available backend for the primary monitor.
pub fn open(prefer_fast: bool) -> Option<Box<dyn Backend>> {
    open_index(prefer_fast, 0)
}

/// Opens the best available backend for screen `index` of `list_monitors`.
pub fn open_index(prefer_fast: bool, index: usize) -> Option<Box<dyn Backend>> {
    #[cfg(windows)]
    if prefer_fast {
        match dxgi::Dxgi::new_index(index) {
            Ok(d) => return Some(Box::new(d)),
            Err(e) => {
                log_line(&format!("dxgi unavailable: {}", e));
            }
        }
    }
    // The screenshot path opens fine in places where it cannot actually
    // deliver (secure desktop), so it has to prove itself once.
    if let Ok(mut s) = fallback::Shots::new_index(index) {
        if delivers(&mut s) {
            return Some(Box::new(s));
        }
        log_line("xcap liefert keine Bilder - versuche den GDI-Weg");
    }
    #[cfg(windows)]
    match gdi::GdiCap::new_index(index) {
        Ok(g) => return Some(Box::new(g)),
        Err(e) => log_line(&format!("gdi unavailable: {}", e)),
    }
    None
}

/// Does this backend really hand out pixels? One grab is enough to find out.
fn delivers(b: &mut impl Backend) -> bool {
    matches!(b.next(400), Next::Frame) && !b.frame().0.is_empty()
}

pub fn log_line(s: &str) {
    use std::io::Write;
    let path = crate::ident::config_dir().join("capture.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", s);
    }
}

// --------------------------------------------------------------------- gdi --

/// Last resort: plain GDI `BitBlt`.
///
/// Slower than everything else (the whole screen travels through the CPU
/// every frame), but it is the only thing that still works on the secure
/// desktop - the lock and login screen refuse Desktop Duplication with
/// "access denied", and that is exactly where unattended access has to work.
///
/// Which device context works there is not obvious, so the constructor simply
/// tries them: the desktop DC of this thread first (that one follows the
/// desktop the process was started on), then a fresh display driver DC, each
/// with and without `CAPTUREBLT`. The first combination that really copies
/// pixels wins and is written into the log.
#[cfg(windows)]
mod gdi {
    use super::{Backend, Next};
    use anyhow::{anyhow, Result};
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, HWND};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDCW, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
        HBITMAP, HDC, HGDIOBJ, ROP_CODE, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorInfo, GetSystemMetrics, CURSORINFO, CURSOR_SHOWING, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    pub struct GdiCap {
        screen: HDC,
        /// true = made with CreateDC (needs DeleteDC), false = GetDC
        owned: bool,
        rop: ROP_CODE,
        mem: HDC,
        bmp: HBITMAP,
        bits: *mut u8,
        buf: Vec<u8>,
        w: u32,
        h: u32,
        x: i32,
        y: i32,
        logged: bool,
    }

    impl GdiCap {
        pub fn new_index(index: usize) -> Result<Self> {
            let list = super::list_monitors(false);
            let (x, y, w, h) = match list.get(index) {
                Some(m) if m.w > 0 && m.h > 0 => (m.x, m.y, m.w, m.h),
                _ => unsafe {
                    (
                        GetSystemMetrics(SM_XVIRTUALSCREEN),
                        GetSystemMetrics(SM_YVIRTUALSCREEN),
                        GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
                        GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
                    )
                },
            };
            unsafe {
                let desktop_dc = GetDC(HWND::default());
                let display_dc = CreateDCW(w!("DISPLAY"), None, None, None);
                if desktop_dc.is_invalid() && display_dc.is_invalid() {
                    return Err(anyhow!("kein Bildschirm-DC zu bekommen"));
                }
                let base = if !desktop_dc.is_invalid() {
                    desktop_dc
                } else {
                    display_dc
                };
                let mem = CreateCompatibleDC(base);
                if mem.is_invalid() {
                    return Err(anyhow!("CreateCompatibleDC fehlgeschlagen"));
                }
                let mut info = BITMAPINFO::default();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    // negative = top down, same row order as everybody else
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                };
                let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
                let bmp = match CreateDIBSection(base, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = DeleteDC(mem);
                        return Err(anyhow!("CreateDIBSection: {}", e));
                    }
                };
                SelectObject(mem, HGDIOBJ(bmp.0));

                let both = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
                let mut chosen: Option<(HDC, bool, ROP_CODE)> = None;
                let mut last_err = 0u32;
                for (dc, owned) in [(desktop_dc, false), (display_dc, true)] {
                    if dc.is_invalid() {
                        continue;
                    }
                    for rop in [both, SRCCOPY] {
                        if BitBlt(mem, 0, 0, w as i32, h as i32, dc, x, y, rop).is_ok() {
                            chosen = Some((dc, owned, rop));
                            break;
                        }
                        last_err = GetLastError().0;
                    }
                    if chosen.is_some() {
                        break;
                    }
                }
                match chosen {
                    Some((dc, owned, rop)) => {
                        super::log_line(&format!(
                            "gdi: Quelle {}, rop {:#x}, {}x{} bei {},{}",
                            if owned { "CreateDC(DISPLAY)" } else { "GetDC(desktop)" },
                            rop.0,
                            w,
                            h,
                            x,
                            y
                        ));
                        // give back whichever handle we do not keep
                        if owned && !desktop_dc.is_invalid() {
                            ReleaseDC(HWND::default(), desktop_dc);
                        }
                        if !owned && !display_dc.is_invalid() {
                            let _ = DeleteDC(display_dc);
                        }
                        Ok(Self {
                            screen: dc,
                            owned,
                            rop,
                            mem,
                            bmp,
                            bits: bits as *mut u8,
                            buf: vec![0u8; (w as usize) * (h as usize) * 4],
                            w,
                            h,
                            x,
                            y,
                            logged: false,
                        })
                    }
                    None => {
                        let _ = DeleteObject(HGDIOBJ(bmp.0));
                        let _ = DeleteDC(mem);
                        if !desktop_dc.is_invalid() {
                            ReleaseDC(HWND::default(), desktop_dc);
                        }
                        if !display_dc.is_invalid() {
                            let _ = DeleteDC(display_dc);
                        }
                        Err(anyhow!("BitBlt geht nicht (Fehler {})", last_err))
                    }
                }
            }
        }
    }

    impl Backend for GdiCap {
        fn next(&mut self, _timeout_ms: u32) -> Next {
            unsafe {
                if BitBlt(
                    self.mem,
                    0,
                    0,
                    self.w as i32,
                    self.h as i32,
                    self.screen,
                    self.x,
                    self.y,
                    self.rop,
                )
                .is_err()
                {
                    if !self.logged {
                        self.logged = true;
                        super::log_line(&format!(
                            "gdi: BitBlt fehlgeschlagen (Fehler {})",
                            GetLastError().0
                        ));
                    }
                    return Next::Lost;
                }
                if self.bits.is_null() {
                    return Next::Lost;
                }
                std::ptr::copy_nonoverlapping(self.bits, self.buf.as_mut_ptr(), self.buf.len());
            }
            Next::Frame
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
            "gdi"
        }
    }

    impl Drop for GdiCap {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
                let _ = DeleteDC(self.mem);
                if self.owned {
                    let _ = DeleteDC(self.screen);
                } else {
                    ReleaseDC(HWND::default(), self.screen);
                }
            }
        }
    }
}
// ---------------------------------------------------------------- fallback --

mod fallback {
    use super::{Backend, Next};
    use anyhow::{anyhow, Result};

    pub struct Shots {
        idx: usize,
        buf: Vec<u8>,
        w: u32,
        h: u32,
        origin: (i32, i32),
    }

    /// Same ordering as the DXGI path: primary first, then left to right.
    fn order(monitors: &[xcap::Monitor]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..monitors.len()).collect();
        order.sort_by_key(|&i| {
            (
                !monitors[i].is_primary().unwrap_or(false),
                monitors[i].x().unwrap_or(0),
                monitors[i].y().unwrap_or(0),
            )
        });
        order
    }

    pub fn describe() -> Vec<super::MonitorDesc> {
        let monitors = match xcap::Monitor::all() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        order(&monitors)
            .into_iter()
            .map(|i| super::MonitorDesc {
                name: monitors[i]
                    .name()
                    .unwrap_or_else(|_| format!("Bildschirm {}", i + 1)),
                w: monitors[i].width().unwrap_or(0),
                h: monitors[i].height().unwrap_or(0),
                x: monitors[i].x().unwrap_or(0),
                y: monitors[i].y().unwrap_or(0),
                primary: monitors[i].is_primary().unwrap_or(false),
            })
            .collect()
    }

    impl Shots {
        pub fn new_index(index: usize) -> Result<Self> {
            let monitors = xcap::Monitor::all().map_err(|e| anyhow!(e.to_string()))?;
            if monitors.is_empty() {
                return Err(anyhow!("kein Monitor"));
            }
            let ord = order(&monitors);
            let idx = ord[index.min(ord.len() - 1)];
            let w = monitors[idx].width().unwrap_or(1920);
            let h = monitors[idx].height().unwrap_or(1080);
            let origin = (
                monitors[idx].x().unwrap_or(0),
                monitors[idx].y().unwrap_or(0),
            );
            Ok(Self {
                idx,
                buf: Vec::new(),
                w,
                h,
                origin,
            })
        }
    }

    impl Backend for Shots {
        fn next(&mut self, _timeout_ms: u32) -> Next {
            // A cached Monitor handle goes stale on Windows and then returns
            // the same picture forever, so it is re-enumerated every frame.
            let mut all = match xcap::Monitor::all() {
                Ok(a) => a,
                Err(_) => return Next::Lost,
            };
            if self.idx >= all.len() {
                return Next::Lost;
            }
            let m = all.swap_remove(self.idx);
            match m.capture_image() {
                Ok(img) => {
                    self.w = img.width();
                    self.h = img.height();
                    self.buf = img.into_raw();
                    Next::Frame
                }
                Err(_) => Next::Lost,
            }
        }
        fn frame(&self) -> (&[u8], u32, u32, bool) {
            (&self.buf, self.w, self.h, false)
        }
        fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }
        fn origin(&self) -> (i32, i32) {
            self.origin
        }
        fn cursor(&self) -> (i32, i32, bool) {
            (0, 0, false)
        }
        fn name(&self) -> &'static str {
            "xcap"
        }
    }
}

// -------------------------------------------------------------------- dxgi --

#[cfg(windows)]
mod dxgi {
    use super::{Backend, Next};
    use anyhow::{anyhow, Result};
    use windows::core::Interface;
    use windows::Win32::Foundation::{HMODULE, POINT, RECT};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;

    /// Above this many changed rectangles a single full copy is cheaper.
    const MAX_RECTS: usize = 48;

    pub struct Dxgi {
        index: usize,
        device: ID3D11Device,
        ctx: ID3D11DeviceContext,
        output: IDXGIOutput1,
        dupl: IDXGIOutputDuplication,
        staging: ID3D11Texture2D,
        buf: Vec<u8>,
        w: u32,
        h: u32,
        origin: (i32, i32),
        holding: bool,
        cursor: (i32, i32, bool),
        dirty: Vec<RECT>,
        moves: Vec<DXGI_OUTDUPL_MOVE_RECT>,
        primed: bool,
        /// one cached video processor per output format
        scaler: Option<Scaler>,
        scaler_nv12: Option<Scaler>,
        gpu_ok: bool,
        last_tex: Option<ID3D11Texture2D>,
    }

    impl Drop for Dxgi {
        fn drop(&mut self) {
            unsafe {
                if self.holding {
                    let _ = self.dupl.ReleaseFrame();
                }
            }
        }
    }

    fn make_device(adapter: Option<&IDXGIAdapter1>) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
        // a specific adapter has to be paired with DRIVER_TYPE_UNKNOWN
        let base: Option<IDXGIAdapter> = match adapter {
            Some(a) => Some(a.cast()?),
            None => None,
        };
        let kind = if base.is_some() {
            D3D_DRIVER_TYPE_UNKNOWN
        } else {
            D3D_DRIVER_TYPE_HARDWARE
        };
        unsafe {
            D3D11CreateDevice(
                base.as_ref(),
                kind,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(|e| anyhow!("D3D11CreateDevice: {}", e))?;
        }
        Ok((
            device.ok_or_else(|| anyhow!("kein D3D11 device"))?,
            ctx.ok_or_else(|| anyhow!("kein D3D11 context"))?,
        ))
    }

    /// One duplicable screen together with the adapter that drives it.
    pub struct Out {
        pub adapter: IDXGIAdapter1,
        pub output: IDXGIOutput1,
        pub rect: RECT,
        pub name: String,
    }

    fn wide(s: &[u16]) -> String {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        String::from_utf16_lossy(&s[..end])
    }

    /// Turns "\\\\.\\DISPLAY2" into what the user sees in the display settings.
    fn friendly(device_name: &str) -> String {
        use windows::core::PCWSTR;
        use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};
        unsafe {
            let mut i = 0u32;
            loop {
                let mut dd = DISPLAY_DEVICEW {
                    cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                    ..Default::default()
                };
                if !EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0).as_bool() {
                    break;
                }
                if wide(&dd.DeviceName) == device_name {
                    let mut wname: Vec<u16> = device_name.encode_utf16().collect();
                    wname.push(0);
                    let mut mon = DISPLAY_DEVICEW {
                        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
                        ..Default::default()
                    };
                    if EnumDisplayDevicesW(PCWSTR(wname.as_ptr()), 0, &mut mon, 0).as_bool() {
                        let s = wide(&mon.DeviceString);
                        if !s.is_empty() {
                            return s;
                        }
                    }
                    let s = wide(&dd.DeviceString);
                    if !s.is_empty() {
                        return s;
                    }
                    break;
                }
                i += 1;
            }
        }
        device_name.to_string()
    }

    /// Every attached output of every adapter, primary first. Enumerating the
    /// factory (instead of just our own adapter) also finds virtual displays
    /// that live on a second, software adapter.
    pub fn enumerate() -> Result<Vec<Out>> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut list: Vec<Out> = Vec::new();
            let mut ai = 0u32;
            while let Ok(adapter) = factory.EnumAdapters1(ai) {
                let mut oi = 0u32;
                while let Ok(out) = adapter.EnumOutputs(oi) {
                    oi += 1;
                    let desc = match out.GetDesc() {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    if !desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    let o1: IDXGIOutput1 = match out.cast() {
                        Ok(o) => o,
                        Err(_) => continue,
                    };
                    let dev = wide(&desc.DeviceName);
                    list.push(Out {
                        adapter: adapter.clone(),
                        output: o1,
                        rect: desc.DesktopCoordinates,
                        name: friendly(&dev),
                    });
                }
                ai += 1;
            }
            if list.is_empty() {
                return Err(anyhow!("kein angeschlossener Ausgang"));
            }
            list.sort_by_key(|o| {
                (
                    !(o.rect.left == 0 && o.rect.top == 0),
                    o.rect.left,
                    o.rect.top,
                )
            });
            Ok(list)
        }
    }

    pub fn describe() -> Vec<super::MonitorDesc> {
        match enumerate() {
            Ok(list) => list
                .iter()
                .map(|o| super::MonitorDesc {
                    name: o.name.clone(),
                    w: (o.rect.right - o.rect.left).max(0) as u32,
                    h: (o.rect.bottom - o.rect.top).max(0) as u32,
                    x: o.rect.left,
                    y: o.rect.top,
                    primary: o.rect.left == 0 && o.rect.top == 0,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn make_tex_fmt(
        device: &ID3D11Device,
        w: u32,
        h: u32,
        usage: D3D11_USAGE,
        bind: u32,
        cpu: u32,
        format: DXGI_FORMAT,
    ) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: usage,
            BindFlags: bind,
            CPUAccessFlags: cpu,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .map_err(|e| anyhow!("CreateTexture2D: {}", e))?;
        }
        tex.ok_or_else(|| anyhow!("keine Textur"))
    }

    fn make_tex(
        device: &ID3D11Device,
        w: u32,
        h: u32,
        usage: D3D11_USAGE,
        bind: u32,
        cpu: u32,
    ) -> Result<ID3D11Texture2D> {
        make_tex_fmt(device, w, h, usage, bind, cpu, DXGI_FORMAT_B8G8R8A8_UNORM)
    }

    /// Hardware scaler: the GPU resizes the captured desktop before anything
    /// is read back over PCIe. Uses the D3D11 video processor, the same block
    /// the media pipeline uses - no shader compilation needed.
    pub struct Scaler {
        vctx: ID3D11VideoContext,
        proc: ID3D11VideoProcessor,
        src: ID3D11Texture2D,
        stage: ID3D11Texture2D,
        in_view: ID3D11VideoProcessorInputView,
        out_view: ID3D11VideoProcessorOutputView,
        pub in_size: (u32, u32),
        pub out_size: (u32, u32),
        /// true: the video processor writes NV12 (colour conversion included)
        pub nv12: bool,
        buf: Vec<u8>,
    }

    impl Scaler {
        pub fn new(
            device: &ID3D11Device,
            ctx: &ID3D11DeviceContext,
            iw: u32,
            ih: u32,
            ow: u32,
            oh: u32,
            nv12: bool,
        ) -> Result<Self> {
            unsafe {
                let vdev: ID3D11VideoDevice = device
                    .cast()
                    .map_err(|e| anyhow!("kein Video-Device: {}", e))?;
                let vctx: ID3D11VideoContext = ctx
                    .cast()
                    .map_err(|e| anyhow!("kein Video-Context: {}", e))?;
                let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    InputWidth: iw,
                    InputHeight: ih,
                    OutputFrameRate: DXGI_RATIONAL {
                        Numerator: 60,
                        Denominator: 1,
                    },
                    OutputWidth: ow,
                    OutputHeight: oh,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                };
                let enumr = vdev
                    .CreateVideoProcessorEnumerator(&desc)
                    .map_err(|e| anyhow!("VideoProcessorEnumerator: {}", e))?;
                let proc = vdev
                    .CreateVideoProcessor(&enumr, 0)
                    .map_err(|e| anyhow!("CreateVideoProcessor: {}", e))?;

                let src = make_tex(
                    device,
                    iw,
                    ih,
                    D3D11_USAGE_DEFAULT,
                    D3D11_BIND_SHADER_RESOURCE.0 as u32 | D3D11_BIND_RENDER_TARGET.0 as u32,
                    0,
                )?;
                let fmt = if nv12 {
                    DXGI_FORMAT_NV12
                } else {
                    DXGI_FORMAT_B8G8R8A8_UNORM
                };
                let dst = make_tex_fmt(
                    device,
                    ow,
                    oh,
                    D3D11_USAGE_DEFAULT,
                    D3D11_BIND_RENDER_TARGET.0 as u32,
                    0,
                    fmt,
                )?;
                let stage = make_tex_fmt(
                    device,
                    ow,
                    oh,
                    D3D11_USAGE_STAGING,
                    0,
                    D3D11_CPU_ACCESS_READ.0 as u32,
                    fmt,
                )?;

                let ivd = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                    FourCC: 0,
                    ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPIV {
                            MipSlice: 0,
                            ArraySlice: 0,
                        },
                    },
                };
                let mut in_view: Option<ID3D11VideoProcessorInputView> = None;
                vdev.CreateVideoProcessorInputView(&src, &enumr, &ivd, Some(&mut in_view))
                    .map_err(|e| anyhow!("InputView: {}", e))?;
                let ovd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                    },
                };
                let mut out_view: Option<ID3D11VideoProcessorOutputView> = None;
                vdev.CreateVideoProcessorOutputView(&dst, &enumr, &ovd, Some(&mut out_view))
                    .map_err(|e| anyhow!("OutputView: {}", e))?;

                let bytes = if nv12 {
                    ow as usize * oh as usize * 3 / 2
                } else {
                    ow as usize * oh as usize * 3
                };
                let s = Self {
                    vctx,
                    proc,
                    src,
                    stage,
                    in_view: in_view.ok_or_else(|| anyhow!("keine InputView"))?,
                    out_view: out_view.ok_or_else(|| anyhow!("keine OutputView"))?,
                    in_size: (iw, ih),
                    out_size: (ow, oh),
                    nv12,
                    buf: vec![0u8; bytes],
                };
                s.vctx.VideoProcessorSetStreamFrameFormat(
                    &s.proc,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                // Colour space bits of D3D11_VIDEO_PROCESSOR_COLOR_SPACE:
                //   bit 0    Usage          (0 = playback)
                //   bit 1    RGB_Range      (0 = full 0..255)
                //   bit 2    YCbCr_Matrix   (0 = BT.601)
                //   bit 3    xvYCC
                //   bit 4-5  Nominal_Range  (1 = 16..235, 2 = 0..255)
                //
                // The desktop arrives as full range RGB, so the input is
                // 0..255. NV12 goes out as BT.601 studio range, which is what
                // the H.264 encoder and our decoder assume - getting this
                // wrong washes the picture out.
                const FULL: u32 = 2u32 << 4;
                const STUDIO: u32 = 1u32 << 4;
                let cs_in = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: FULL };
                s.vctx.VideoProcessorSetStreamColorSpace(&s.proc, 0, &cs_in);
                let cs_out = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                    _bitfield: if nv12 { STUDIO } else { FULL },
                };
                s.vctx.VideoProcessorSetOutputColorSpace(&s.proc, &cs_out);
                super::log_line(&format!(
                    "gpu scaler {}x{} -> {}x{} ({})",
                    iw,
                    ih,
                    ow,
                    oh,
                    if nv12 { "NV12" } else { "RGB" }
                ));
                Ok(s)
            }
        }

        /// frame (GPU) -> scaled (GPU) -> staging -> packed RGB.
        pub fn scale(&mut self, ctx: &ID3D11DeviceContext, frame: &ID3D11Texture2D) -> Result<()> {
            self.scale_teil(ctx, frame, (0.0, 0.0, 1.0, 1.0))
        }

        /// Wie `scale`, aber nur ein Ausschnitt der Quelle (Anteile 0..1).
        ///
        /// Der Videoprozessor kann das von Haus aus: `SetStreamSourceRect`
        /// sagt ihm, welches Rechteck er auf die Ausgabegroesse ziehen soll.
        /// Damit bleibt beim Hineinzoomen alles auf der Grafikkarte - kein
        /// Vollbild ueber PCIe, keine Rechenzeit im Hauptprozessor, und die
        /// Bildpunkte sind ECHT statt hochgerechnet.
        pub fn scale_teil(
            &mut self,
            ctx: &ID3D11DeviceContext,
            frame: &ID3D11Texture2D,
            teil: (f32, f32, f32, f32),
        ) -> Result<()> {
            unsafe {
                ctx.CopyResource(&self.src, frame);
                let (iw, ih) = self.in_size;
                let (tx, ty, tw, th) = teil;
                let ganz = tw >= 0.999 && th >= 0.999 && tx <= 0.001 && ty <= 0.001;
                // Das Ziel IMMER ausdruecklich auf die volle Ausgabe setzen.
                //
                // Ohne diese Zeile fuellt der Videoprozessor bei gesetztem
                // Quell-Rechteck je nach Treiber gar nichts oder nur eine Ecke
                // - gemessen auf der Intel-Karte im Surface: das Bild kam als
                // einfarbige Flaeche heraus, ganz ohne Fehlermeldung.
                let (ow, oh) = self.out_size;
                let ziel = RECT { left: 0, top: 0, right: ow as i32, bottom: oh as i32 };
                self.vctx
                    .VideoProcessorSetStreamDestRect(&self.proc, 0, true, Some(&ziel));
                self.vctx
                    .VideoProcessorSetOutputTargetRect(&self.proc, true, Some(&ziel));
                if ganz {
                    // Kein Ausschnitt: die Einstellung wieder abschalten,
                    // sonst bliebe der letzte Zuschnitt fuer immer stehen.
                    self.vctx
                        .VideoProcessorSetStreamSourceRect(&self.proc, 0, false, None);
                } else {
                    // In Bildpunkte umrechnen und im Bild halten - ein
                    // Rechteck ausserhalb liefert ein schwarzes Bild.
                    let l = ((tx.clamp(0.0, 1.0) * iw as f32) as i32).clamp(0, iw as i32 - 2);
                    let t = ((ty.clamp(0.0, 1.0) * ih as f32) as i32).clamp(0, ih as i32 - 2);
                    let r = (l + (tw.clamp(0.0, 1.0) * iw as f32) as i32).clamp(l + 2, iw as i32);
                    let b = (t + (th.clamp(0.0, 1.0) * ih as f32) as i32).clamp(t + 2, ih as i32);
                    let rect = RECT { left: l, top: t, right: r, bottom: b };
                    self.vctx
                        .VideoProcessorSetStreamSourceRect(&self.proc, 0, true, Some(&rect));
                }
                let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                    Enable: true.into(),
                    OutputIndex: 0,
                    InputFrameOrField: 0,
                    PastFrames: 0,
                    FutureFrames: 0,
                    ppPastSurfaces: std::ptr::null_mut(),
                    pInputSurface: std::mem::ManuallyDrop::new(Some(self.in_view.clone())),
                    ppFutureSurfaces: std::ptr::null_mut(),
                    ppPastSurfacesRight: std::ptr::null_mut(),
                    pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
                    ppFutureSurfacesRight: std::ptr::null_mut(),
                };
                let res = self
                    .vctx
                    .VideoProcessorBlt(&self.proc, &self.out_view, 0, &[stream.clone()]);
                std::mem::ManuallyDrop::drop(&mut stream.pInputSurface);
                res.map_err(|e| anyhow!("VideoProcessorBlt: {}", e))?;

                let dst_tex: ID3D11Resource = self.out_view.GetResource()?;
                ctx.CopyResource(&self.stage, &dst_tex);

                let (ow, oh) = self.out_size;
                let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                ctx.Map(&self.stage, 0, D3D11_MAP_READ, 0, Some(&mut map))
                    .map_err(|e| anyhow!("Map: {}", e))?;
                let base = map.pData as *const u8;
                let pitch = map.RowPitch as usize;
                let (w, h) = (ow as usize, oh as usize);
                if self.nv12 {
                    // Y plane, then the interleaved UV plane right behind it.
                    // We hand out a tightly packed buffer (stride = width).
                    let need = w * h * 3 / 2;
                    if self.buf.len() != need {
                        self.buf = vec![0u8; need];
                    }
                    for y in 0..h {
                        let src = base.add(y * pitch);
                        let dst = &mut self.buf[y * w..(y + 1) * w];
                        std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), w);
                    }
                    for y in 0..h / 2 {
                        let src = base.add((h + y) * pitch);
                        let off = w * h + y * w;
                        let dst = &mut self.buf[off..off + w];
                        std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), w);
                    }
                } else {
                    if self.buf.len() != w * h * 3 {
                        self.buf = vec![0u8; w * h * 3];
                    }
                    for y in 0..h {
                        let row = base.add(y * pitch);
                        let out = &mut self.buf[y * w * 3..(y + 1) * w * 3];
                        for x in 0..w {
                            let s = row.add(x * 4);
                            out[x * 3] = *s.add(2);
                            out[x * 3 + 1] = *s.add(1);
                            out[x * 3 + 2] = *s;
                        }
                    }
                }
                ctx.Unmap(&self.stage, 0);
                Ok(())
            }
        }

        pub fn bytes(&self) -> &[u8] {
            &self.buf
        }
    }

    fn make_staging(device: &ID3D11Device, w: u32, h: u32) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        unsafe {
            device
                .CreateTexture2D(&desc, None, Some(&mut tex))
                .map_err(|e| anyhow!("CreateTexture2D: {}", e))?;
        }
        tex.ok_or_else(|| anyhow!("kein staging texture"))
    }

    impl Dxgi {
        pub fn new() -> Result<Self> {
            Self::new_index(0)
        }

        pub fn new_index(index: usize) -> Result<Self> {
            let outs = enumerate()?;
            let i = index.min(outs.len() - 1);
            let (device, ctx) = make_device(Some(&outs[i].adapter))?;
            let output = outs[i].output.clone();
            let rect = outs[i].rect;
            let w = (rect.right - rect.left).max(1) as u32;
            let h = (rect.bottom - rect.top).max(1) as u32;
            let dupl = unsafe {
                output
                    .DuplicateOutput(&device)
                    .map_err(|e| anyhow!("DuplicateOutput: {}", e))?
            };
            let staging = make_staging(&device, w, h)?;
            super::log_line(&format!(
                "dxgi ready [{}] {} {}x{} at {},{}",
                i, outs[i].name, w, h, rect.left, rect.top
            ));
            Ok(Self {
                index: i,
                device,
                ctx,
                output,
                dupl,
                staging,
                buf: vec![0u8; w as usize * h as usize * 4],
                w,
                h,
                origin: (rect.left, rect.top),
                holding: false,
                cursor: (0, 0, false),
                dirty: Vec::with_capacity(64),
                moves: Vec::with_capacity(32),
                primed: false,
                scaler: None,
                scaler_nv12: None,
                gpu_ok: std::env::var("FV_NOGPU").is_err(),
                last_tex: None,
            })
        }

        /// Rebuilds the duplication after ACCESS_LOST (resolution change, UAC
        /// prompt, session switch, ...).
        fn recreate(&mut self) -> Result<()> {
            unsafe {
                if self.holding {
                    let _ = self.dupl.ReleaseFrame();
                    self.holding = false;
                }
            }
            let outs = enumerate()?;
            let i = self.index.min(outs.len() - 1);
            let output = outs[i].output.clone();
            let rect = outs[i].rect;
            let w = (rect.right - rect.left).max(1) as u32;
            let h = (rect.bottom - rect.top).max(1) as u32;
            // a screen can move to another GPU (hot plug, hybrid graphics), in
            // that case the old device cannot duplicate it any more
            self.dupl = match unsafe { output.DuplicateOutput(&self.device) } {
                Ok(d) => d,
                Err(_) => {
                    let (device, ctx) = make_device(Some(&outs[i].adapter))?;
                    self.device = device;
                    self.ctx = ctx;
                    self.staging = make_staging(&self.device, w, h)?;
                    self.w = 0; // force the buffer rebuild below
                    unsafe { output.DuplicateOutput(&self.device)? }
                }
            };
            self.output = output;
            if w != self.w || h != self.h {
                self.staging = make_staging(&self.device, w, h)?;
                self.buf = vec![0u8; w as usize * h as usize * 4];
                self.w = w;
                self.h = h;
            }
            self.origin = (rect.left, rect.top);
            self.primed = false;
            Ok(())
        }

        fn collect_rects(&mut self) -> Result<bool> {
            self.dirty.clear();
            self.moves.clear();
            unsafe {
                // move rects first (the API guarantees they were applied first)
                let mut needed = 0u32;
                let mut cap = 32usize;
                loop {
                    self.moves.resize(cap, DXGI_OUTDUPL_MOVE_RECT::default());
                    let bytes = (cap * std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32;
                    match self
                        .dupl
                        .GetFrameMoveRects(bytes, self.moves.as_mut_ptr(), &mut needed)
                    {
                        Ok(()) => {
                            let n = needed as usize / std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>();
                            self.moves.truncate(n);
                            break;
                        }
                        Err(e) if e.code() == DXGI_ERROR_MORE_DATA && cap < 4096 => {
                            cap *= 4;
                        }
                        Err(_) => {
                            self.moves.clear();
                            break;
                        }
                    }
                }

                let mut cap = 64usize;
                loop {
                    self.dirty.resize(cap, RECT::default());
                    let bytes = (cap * std::mem::size_of::<RECT>()) as u32;
                    match self
                        .dupl
                        .GetFrameDirtyRects(bytes, self.dirty.as_mut_ptr(), &mut needed)
                    {
                        Ok(()) => {
                            let n = needed as usize / std::mem::size_of::<RECT>();
                            self.dirty.truncate(n);
                            break;
                        }
                        Err(e) if e.code() == DXGI_ERROR_MORE_DATA && cap < 8192 => {
                            cap *= 4;
                        }
                        Err(_) => {
                            self.dirty.clear();
                            return Ok(false); // unknown -> full copy
                        }
                    }
                }
            }
            Ok(true)
        }

        /// Copies the changed regions into the CPU buffer.
        fn read_back(&mut self, src: &ID3D11Texture2D, full: bool) -> Result<()> {
            let stride = self.w as usize * 4;
            unsafe {
                if full {
                    self.ctx.CopyResource(&self.staging, src);
                } else {
                    for r in self.dirty.iter().chain(
                        self.moves
                            .iter()
                            .map(|m| &m.DestinationRect)
                            .collect::<Vec<_>>()
                            .into_iter(),
                    ) {
                        let bx = D3D11_BOX {
                            left: r.left.max(0) as u32,
                            top: r.top.max(0) as u32,
                            front: 0,
                            right: (r.right.max(0) as u32).min(self.w),
                            bottom: (r.bottom.max(0) as u32).min(self.h),
                            back: 1,
                        };
                        if bx.right <= bx.left || bx.bottom <= bx.top {
                            continue;
                        }
                        self.ctx.CopySubresourceRegion(
                            &self.staging,
                            0,
                            bx.left,
                            bx.top,
                            0,
                            src,
                            0,
                            Some(&bx),
                        );
                    }
                }

                let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                self.ctx
                    .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut map))
                    .map_err(|e| anyhow!("Map: {}", e))?;
                let src_ptr = map.pData as *const u8;
                let pitch = map.RowPitch as usize;

                let copy_rows = |dst: &mut [u8], y0: usize, y1: usize, x0: usize, x1: usize| {
                    let bytes = (x1 - x0) * 4;
                    for y in y0..y1 {
                        let s = src_ptr.add(y * pitch + x0 * 4);
                        let d = &mut dst[y * stride + x0 * 4..y * stride + x0 * 4 + bytes];
                        std::ptr::copy_nonoverlapping(s, d.as_mut_ptr(), bytes);
                    }
                };

                if full {
                    copy_rows(&mut self.buf, 0, self.h as usize, 0, self.w as usize);
                } else {
                    let rects: Vec<RECT> = self
                        .dirty
                        .iter()
                        .copied()
                        .chain(self.moves.iter().map(|m| m.DestinationRect))
                        .collect();
                    for r in rects {
                        let x0 = r.left.max(0) as usize;
                        let x1 = (r.right.max(0) as usize).min(self.w as usize);
                        let y0 = r.top.max(0) as usize;
                        let y1 = (r.bottom.max(0) as usize).min(self.h as usize);
                        if x1 <= x0 || y1 <= y0 {
                            continue;
                        }
                        copy_rows(&mut self.buf, y0, y1, x0, x1);
                    }
                }
                self.ctx.Unmap(&self.staging, 0);
            }
            Ok(())
        }
    }

    impl Backend for Dxgi {
        fn next(&mut self, timeout_ms: u32) -> Next {
            unsafe {
                if self.holding {
                    let _ = self.dupl.ReleaseFrame();
                    self.holding = false;
                }
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res: Option<IDXGIResource> = None;
                match self.dupl.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                    Ok(()) => {}
                    Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Next::Unchanged,
                    Err(e) => {
                        super::log_line(&format!("AcquireNextFrame: {}", e));
                        if self.recreate().is_err() {
                            return Next::Lost;
                        }
                        return Next::Unchanged;
                    }
                }
                self.holding = true;

                if info.LastMouseUpdateTime != 0 {
                    let p: POINT = info.PointerPosition.Position;
                    self.cursor = (
                        p.x + self.origin.0,
                        p.y + self.origin.1,
                        info.PointerPosition.Visible.as_bool(),
                    );
                }
                // no new pixels, only the pointer moved
                if info.LastPresentTime == 0 && self.primed {
                    return Next::Unchanged;
                }

                let tex: ID3D11Texture2D = match res.as_ref().and_then(|r| r.cast().ok()) {
                    Some(t) => t,
                    None => return Next::Unchanged,
                };

                let known = self.collect_rects().unwrap_or(false);
                let count = self.dirty.len() + self.moves.len();
                let full = !self.primed || !known || count == 0 || count > MAX_RECTS;
                if !full && count == 0 {
                    return Next::Unchanged;
                }

                // Private GPU copy: the duplication surface is gone again
                // after ReleaseFrame, the hardware scaler still needs it.
                // Only the changed rectangles are copied - GPU to GPU.
                if self.gpu_ok {
                    let same = self
                        .last_tex
                        .as_ref()
                        .map(|t| {
                            let mut d = D3D11_TEXTURE2D_DESC::default();
                            t.GetDesc(&mut d);
                            d.Width == self.w && d.Height == self.h
                        })
                        .unwrap_or(false);
                    if !same {
                        self.last_tex = make_tex(
                            &self.device,
                            self.w,
                            self.h,
                            D3D11_USAGE_DEFAULT,
                            D3D11_BIND_SHADER_RESOURCE.0 as u32,
                            0,
                        )
                        .ok();
                    }
                    if let Some(dst) = self.last_tex.clone() {
                        if full || !same {
                            self.ctx.CopyResource(&dst, &tex);
                        } else {
                            let rects: Vec<RECT> = self
                                .dirty
                                .iter()
                                .copied()
                                .chain(self.moves.iter().map(|m| m.DestinationRect))
                                .collect();
                            for r in rects {
                                let bx = D3D11_BOX {
                                    left: r.left.max(0) as u32,
                                    top: r.top.max(0) as u32,
                                    front: 0,
                                    right: (r.right.max(0) as u32).min(self.w),
                                    bottom: (r.bottom.max(0) as u32).min(self.h),
                                    back: 1,
                                };
                                if bx.right <= bx.left || bx.bottom <= bx.top {
                                    continue;
                                }
                                self.ctx.CopySubresourceRegion(
                                    &dst, 0, bx.left, bx.top, 0, &tex, 0, Some(&bx),
                                );
                            }
                        }
                    }
                    if self.scaler.is_some() || self.scaler_nv12.is_some() {
                        // nothing travels over PCIe at full resolution
                        self.primed = true;
                        return Next::Frame;
                    }
                }
                if self.read_back(&tex, full).is_err() {
                    return Next::Lost;
                }
                self.primed = true;
                Next::Frame
            }
        }

        fn frame(&self) -> (&[u8], u32, u32, bool) {
            (&self.buf, self.w, self.h, true)
        }
        fn size(&self) -> (u32, u32) {
            (self.w, self.h)
        }
        fn origin(&self) -> (i32, i32) {
            self.origin
        }
        fn cursor(&self) -> (i32, i32, bool) {
            self.cursor
        }
        fn name(&self) -> &'static str {
            "dxgi"
        }

        fn scaled_teil(
            &mut self,
            dw: u32,
            dh: u32,
            nv12: bool,
            teil: (f32, f32, f32, f32),
        ) -> Option<&[u8]> {
            if !self.gpu_ok {
                return None;
            }
            let frame = self.last_tex.clone()?;
            let (iw, ih) = (self.w, self.h);
            let slot = if nv12 {
                &mut self.scaler_nv12
            } else {
                &mut self.scaler
            };
            let need = slot
                .as_ref()
                .map(|s| s.in_size != (iw, ih) || s.out_size != (dw, dh))
                .unwrap_or(true);
            if need {
                match Scaler::new(&self.device, &self.ctx, iw, ih, dw, dh, nv12) {
                    Ok(s) => *slot = Some(s),
                    Err(e) => {
                        super::log_line(&format!("gpu scaler aus: {}", e));
                        self.gpu_ok = false;
                        self.scaler = None;
                        self.scaler_nv12 = None;
                        return None;
                    }
                }
            }
            let ctx = self.ctx.clone();
            let err = match slot.as_mut() {
                Some(s) => s.scale_teil(&ctx, &frame, teil).err(),
                None => Some(anyhow!("kein Scaler")),
            };
            if let Some(e) = err {
                super::log_line(&format!("gpu scale fehlgeschlagen: {}", e));
                self.gpu_ok = false;
                self.scaler = None;
                self.scaler_nv12 = None;
                return None;
            }
            if nv12 {
                self.scaler_nv12.as_ref().map(|s| s.bytes())
            } else {
                self.scaler.as_ref().map(|s| s.bytes())
            }
        }

        fn gpu_scaling(&self) -> bool {
            self.gpu_ok && (self.scaler.is_some() || self.scaler_nv12.is_some())
        }
    }
}

/// Small benchmark used by `--captest`: how long does one frame take?
pub fn bench(rounds: u32, prefer_fast: bool) -> String {
    let mut out = String::new();
    let mut cap = match open(prefer_fast) {
        Some(c) => c,
        None => return "kein Capture-Backend verfuegbar\n".to_string(),
    };
    let (w, h) = cap.size();
    out.push_str(&format!("backend {} {}x{}\n", cap.name(), w, h));
    let mut frames = 0u32;
    let mut unchanged = 0u32;
    let mut total = 0u128;
    let t0 = Instant::now();
    for _ in 0..rounds {
        let t = Instant::now();
        match cap.next(200) {
            Next::Frame => {
                frames += 1;
                total += t.elapsed().as_micros();
            }
            Next::Unchanged => unchanged += 1,
            Next::Lost => {
                out.push_str("capture lost\n");
                break;
            }
        }
    }
    let secs = t0.elapsed().as_secs_f32();
    out.push_str(&format!(
        "{} Frames, {} unveraendert in {:.2}s | {:.2} ms pro echtem Frame\n",
        frames,
        unchanged,
        secs,
        total as f32 / frames.max(1) as f32 / 1000.0
    ));
    out
}
