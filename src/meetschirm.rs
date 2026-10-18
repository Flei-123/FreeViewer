//! Native meeting - stage 3: share the SCREEN, without a browser.
//!
//! The image comes from the same capture that remote maintenance
//! uses (`capture.rs`: DXGI Desktop Duplication, otherwise GDI/screenshot).
//! It is brought to a sensible size, converted to NV12 and
//! then sent into the meeting through the H.264 encoder just like the camera
//! - but on its OWN track, so that the other side can tell camera
//! and screen apart (the server gets
//! `publish {screen:true}` for that).
//!
//! Why a separate track and not switching the camera: otherwise you
//! disappear from the round yourself as soon as you show something - exactly
//! what bothered Justin about other tools.
//!
//! Screen is not camera: more points, less motion. Hence a
//! larger target resolution and more bitrate than for the camera, but
//! for an UNCHANGED image only a refresh is sent every few hundred milliseconds
//! (so that a late joiner does not face
//! a black area), instead of stubbornly sending the same thing 30 times per second.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use crate::meetcam::Image;

/// Ein teilbarer Display.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub is_primary: bool,
}

/// WHAT is shared: a whole screen or exactly one window.
///
/// Why a separate type and not just two functions: the
/// capture thread must be able to open the source AGAIN when the capture
/// breaks off (resolution changed, session switched). For that it needs
/// the specification from which it opened it in the first place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// Index aus `liste()`.
    Display(usize),
    /// Window id from `crate::window::liste()`.
    Window(isize),
}

impl Source {
    fn open(&self) -> Option<Box<dyn crate::capture::Backend>> {
        match self {
            Source::Display(i) => crate::capture::open_index(true, *i),
            Source::Window(k) => crate::window::recording(*k),
        }
    }

    /// Where does the source lie on the screen? (x, y, width, height).
    /// Needed for the red frame.
    pub fn lage(&self) -> Option<(i32, i32, i32, i32)> {
        match self {
            Source::Display(i) => crate::capture::list_monitors(true)
                .get(*i)
                .map(|m| (m.x, m.y, m.w as i32, m.h as i32)),
            Source::Window(k) => crate::window::lage(*k),
        }
    }
}

/// All screens of this machine, the main screen first.
pub fn liste() -> Vec<Screen> {
    crate::capture::list_monitors(true)
        .into_iter()
        .map(|m| Screen {
            name: m.name,
            width_px: m.w,
            height_px: m.h,
            is_primary: m.primary,
        })
        .collect()
}

/// Target size: keep the aspect ratio, fit into `max_b` x `max_h`,
/// even edge lengths (NV12 needs that).
pub fn target_size(sb: u32, sh: u32, max_b: u32, max_h: u32) -> (u32, u32) {
    if sb < 2 || sh < 2 {
        return (2, 2);
    }
    let mut b = sb as f64;
    let mut h = sh as f64;
    let f = (max_b as f64 / b).min(max_h as f64 / h).min(1.0);
    b *= f;
    h *= f;
    let b = ((b.round() as u32) & !1).max(2);
    let h = ((h.round() as u32) & !1).max(2);
    (b, h)
}

/// The area that is sent from - as a fraction of the screen.
/// (0,0,1,1) = the whole screen.
pub type Region = (f32, f32, f32, f32);

pub const WHOLE: Region = (0.0, 0.0, 1.0, 1.0);

/// Turn the wishes of ALL viewers into one area that is sent.
///
/// WHY at all: if a viewer zooms in, they otherwise only get
/// extrapolated pixels. If the SENDER instead cuts exactly this
/// section out of its native capture and sends it at the SAME
/// encode size, that costs no additional bandwidth - but the image
/// really gets sharper, because real pixels are transmitted.
///
/// WHY a union: several people may be watching. If only one were
/// followed, the others would see the wrong section. The hull around
/// all wishes is the only area with which EVERYONE is right.
///
/// The result is always SQUARE in fraction space (so in the real image
/// the same aspect ratio) - otherwise the sent image would be distorted - and
/// gets some margin so that small mouse movements do not constantly trigger a new
/// crop.
pub fn region_merge(wishes: &[Region]) -> Region {
    let real: Vec<&Region> = wishes
        .iter()
        .filter(|(_, _, w, h)| *w > 0.001 && *h > 0.001)
        .collect();
    if real.is_empty() {
        return WHOLE;
    }
    let mut x0 = 1.0f32;
    let mut y0 = 1.0f32;
    let mut x1 = 0.0f32;
    let mut y1 = 0.0f32;
    for (x, y, w, h) in real {
        x0 = x0.min(x.clamp(0.0, 1.0));
        y0 = y0.min(y.clamp(0.0, 1.0));
        x1 = x1.max((x + w).clamp(0.0, 1.0));
        y1 = y1.max((y + h).clamp(0.0, 1.0));
    }
    // Add margin: 12 % of the edge length on each side.
    let mut b = (x1 - x0).max(0.02);
    let mut h = (y1 - y0).max(0.02);
    let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    b *= 1.24;
    h *= 1.24;
    // Same edge length in fraction space = same aspect ratio as
    // the screen. Anything else would be a distorted image.
    let k = b.max(h).min(1.0);
    // Not worth it: almost the whole screen -> better without a crop, then
    // the fast path via the graphics card applies.
    if k > 0.92 {
        return WHOLE;
    }
    let x = (mx - k * 0.5).clamp(0.0, 1.0 - k);
    let y = (my - k * 0.5).clamp(0.0, 1.0 - k);
    (x, y, k, k)
}

/// Is a change of the sent area worthwhile? Tiny shifts are not
/// worth it - every change costs a keyframe.
pub fn region_worth_switch(alt: Region, neu: Region) -> bool {
    let d = (alt.0 - neu.0).abs().max((alt.1 - neu.1).abs());
    let s = (alt.2 - neu.2).abs();
    // 4 % Verschiebung oder 6 % Groessenaenderung.
    d > 0.04 || s > 0.06
}

/// Scale a captured image (BGRA or RGBA) down to `zb` x `zh` and
/// convert to NV12. Area average instead of nearest neighbour - otherwise
/// text flickers unreadably when scaled down.
pub fn image_to_nv12(
    px: &[u8],
    sb: u32,
    sh: u32,
    bgra: bool,
    zb: u32,
    zh: u32,
    out: &mut Vec<u8>,
) -> bool {
    image_to_nv12_part(px, sb, sh, bgra, WHOLE, zb, zh, out)
}

/// Like `image_to_nv12`, but only a SECTION of the capture (fractions
/// 0..1). This way, when zooming in, not the whole screen is
/// scaled down, but the interesting part is sent in full sharpness -
/// at the same encode size and thus the same bandwidth.
#[allow(clippy::too_many_arguments)]
pub fn image_to_nv12_part(
    px: &[u8],
    sb: u32,
    sh: u32,
    bgra: bool,
    part: Region,
    zb: u32,
    zh: u32,
    out: &mut Vec<u8>,
) -> bool {
    let (sbu, shu, zbu, zhu) = (sb as usize, sh as usize, zb as usize, zh as usize);
    // Below 2x2 there is nothing to get - better to refuse honestly than to invent
    // an image from a single point.
    if sbu < 2 || shu < 2 || zbu < 2 || zhu < 2 || px.len() < sbu * shu * 4 {
        return false;
    }
    // Convert the section to pixels. Everything stays within the image - a
    // section that sticks out would show garbage at the edge.
    let (ax, ay, aw, ah) = part;
    let ox = ((ax.clamp(0.0, 1.0) * sbu as f32) as usize).min(sbu.saturating_sub(2));
    let oy = ((ay.clamp(0.0, 1.0) * shu as f32) as usize).min(shu.saturating_sub(2));
    let aw = ((aw.clamp(0.0, 1.0) * sbu as f32) as usize).max(2).min(sbu - ox);
    let ah = ((ah.clamp(0.0, 1.0) * shu as f32) as usize).max(2).min(shu - oy);

    // Intermediate step RGB, because rgb_to_nv12 expects exactly that.
    let mut rgb = vec![0u8; zbu * zhu * 3];
    let (ri, gi, bi) = if bgra { (2usize, 1usize, 0usize) } else { (0usize, 1usize, 2usize) };
    for ty in 0..zhu {
        let y0 = oy + ty * ah / zhu;
        let y1 = (oy + (ty + 1) * ah / zhu).max(y0 + 1).min(oy + ah);
        for tx in 0..zbu {
            let x0 = ox + tx * aw / zbu;
            let x1 = (ox + (tx + 1) * aw / zbu).max(x0 + 1).min(ox + aw);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let text_row = y * sbu * 4;
                for x in x0..x1 {
                    let i = text_row + x * 4;
                    r += px[i + ri] as u32;
                    g += px[i + gi] as u32;
                    b += px[i + bi] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (ty * zbu + tx) * 3;
            rgb[o] = (r / n) as u8;
            rgb[o + 1] = (g / n) as u8;
            rgb[o + 2] = (b / n) as u8;
        }
    }
    crate::h264::rgb_to_nv12(&rgb, zb, zh, out);
    true
}

/// Where is the mouse pointer ON this screen - as a fraction 0..1?
///
/// WHY as a fraction and not in pixels: the viewer gets a
/// scaled-down image and knows nothing about the real resolution. A
/// fraction always fits, however much it was scaled down.
///
/// None = the pointer is currently on ANOTHER screen; then
/// any number would be a lie.
/// Where is the pointer WITHIN the shared source (0..1)?
///
/// For a window this is not the same as for the screen: the window
/// moves, so the position has to be fetched fresh on every call.
/// If the pointer is outside, None comes - an invented position would be
/// worse than none at all.
pub fn pointer_share_source(q: Source) -> Option<(f32, f32)> {
    match q {
        Source::Display(i) => pointer_share(i),
        Source::Window(_) => {
            #[cfg(windows)]
            {
                use windows::Win32::Foundation::POINT;
                use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
                let (x, y, b, h) = q.lage()?;
                if b < 2 || h < 2 {
                    return None;
                }
                let mut p = POINT::default();
                // SAFETY: `p` is a local POINT that the call fills in.
                if unsafe { GetCursorPos(&mut p) }.is_err() {
                    return None;
                }
                let (dx, dy) = (p.x - x, p.y - y);
                if dx < 0 || dy < 0 || dx >= b || dy >= h {
                    return None;
                }
                Some((dx as f32 / b as f32, dy as f32 / h as f32))
            }
            #[cfg(not(windows))]
            {
                None
            }
        }
    }
}

pub fn pointer_share(index: usize) -> Option<(f32, f32)> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let m = crate::capture::list_monitors(true).into_iter().nth(index)?;
        if m.w < 2 || m.h < 2 {
            return None;
        }
        let mut p = POINT::default();
        // SAFETY: `p` is a local POINT that the call fills in.
        if unsafe { GetCursorPos(&mut p) }.is_err() {
            return None;
        }
        let dx = p.x - m.x;
        let dy = p.y - m.y;
        if dx < 0 || dy < 0 || dx >= m.w as i32 || dy >= m.h as i32 {
            return None;
        }
        Some((dx as f32 / m.w as f32, dy as f32 / m.h as f32))
    }
    #[cfg(not(windows))]
    {
        let _ = index;
        None
    }
}

/// The classic arrow as an outline (points in a 12x19 grid).
///
/// WHY draw it ourselves at all: Windows does NOT deliver the mouse pointer
/// with the screen capture - the graphics card overlays it
/// on top. Whoever does nothing sees no pointer when watching; only
/// when dragging a window one flashes up, because Windows then
/// really paints the drag into the image. That is exactly what Justin observed.
const ARROW: &[(f32, f32)] = &[
    (0.0, 0.0),
    (0.0, 16.0),
    (3.6, 12.4),
    (6.2, 18.4),
    (8.6, 17.4),
    (6.0, 11.6),
    (11.0, 11.6),
];

/// Paint the mouse pointer into a tightly packed NV12 image.
///
/// `x`/`y` is the tip in pixels. White with a dark border - so it stays
/// visible on light AS WELL AS dark ground.
pub fn pointer_paint(nv12: &mut [u8], w: u32, h: u32, x: i32, y: i32, dimension: f32) {
    let (wu, hu) = (w as usize, h as usize);
    if nv12.len() < wu * hu * 3 / 2 || dimension <= 0.0 {
        return;
    }
    let s = dimension / 19.0;
    let points: Vec<(f32, f32)> = ARROW
        .iter()
        .map(|(px, py)| (x as f32 + px * s, y as f32 + py * s))
        .collect();
    let top_y = points.iter().fold(f32::MAX, |a, p| a.min(p.1)).floor() as i32;
    let bottom = points.iter().fold(f32::MIN, |a, p| a.max(p.1)).ceil() as i32;
    // Two passes: first the dark border (somewhat thicker), then white.
    for (dick, hell) in [(1.6 * s.max(1.0), 16u8), (0.0, 235u8)] {
        for zy in top_y.max(0)..=bottom.min(hu as i32 - 1) {
            // Collect the intersections of the scanline with the outline.
            let mut xs: Vec<f32> = Vec::new();
            let mitte = zy as f32 + 0.5;
            for i in 0..points.len() {
                let (x1, y1) = points[i];
                let (x2, y2) = points[(i + 1) % points.len()];
                if (y1 <= mitte && y2 > mitte) || (y2 <= mitte && y1 > mitte) {
                    xs.push(x1 + (mitte - y1) / (y2 - y1) * (x2 - x1));
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for paar in xs.chunks(2) {
                if paar.len() < 2 {
                    break;
                }
                let von = ((paar[0] - dick).floor() as i32).max(0);
                let bis = ((paar[1] + dick).ceil() as i32).min(wu as i32 - 1);
                for zx in von..=bis {
                    nv12[zy as usize * wu + zx as usize] = hell;
                    // Set colour to neutral, otherwise the background shows through.
                    let uv = wu * hu + (zy as usize / 2) * wu + (zx as usize & !1);
                    if uv + 1 < nv12.len() {
                        nv12[uv] = 128;
                        nv12[uv + 1] = 128;
                    }
                }
            }
        }
    }
}

/// Running screen capture. Only ever holds the NEWEST frame ready.
pub struct Recording {
    pub name: String,
    pub width_px: u32,
    pub height_px: u32,
    /// Where the image comes from (screen or window).
    pub origin: Source,
    neu: Arc<Mutex<Option<Image>>>,
    ctr: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
    /// Which part of the screen is sent (fractions). The
    /// capture thread reads this on EVERY frame - so a change takes effect immediately,
    /// without reopening the capture.
    region: Arc<Mutex<Region>>,
}

impl Recording {
    /// Send only this section (fractions 0..1). GANZ = everything again.
    pub fn region_set(&self, b: Region) {
        if let Ok(mut g) = self.region.lock() {
            *g = b;
        }
    }
    pub fn region(&self) -> Region {
        self.region.lock().map(|g| *g).unwrap_or(WHOLE)
    }

    /// Pick up the newest frame (and take it out of the buffer).
    pub fn newest(&self) -> Option<Image> {
        self.neu.lock().ok().and_then(|mut b| b.take())
    }
    pub fn recorded(&self) -> u64 {
        self.ctr.load(Ordering::Relaxed)
    }
    pub fn error(&self) -> String {
        self.error.lock().map(|f| f.clone()).unwrap_or_default()
    }
    pub fn stop_capture(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop_capture();
    }
}

/// Open screen `index` and capture in the background.
///
/// `max_b`/`max_h` limit the transmission size (aspect ratio
/// is preserved), `fps` is the upper limit of the frame rate.
pub fn open(index: usize, max_b: u32, max_h: u32, fps: u32) -> Result<Recording> {
    open_source(Source::Display(index), max_b, max_h, fps)
}

/// Like `open`, but the source may also be a single window.
pub fn open_source(origin: Source, max_b: u32, max_h: u32, fps: u32) -> Result<Recording> {
    let fps = fps.clamp(1, 60);
    let neu: Arc<Mutex<Option<Image>>> = Arc::new(Mutex::new(None));
    let ctr = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(String::new()));
    let region = Arc::new(Mutex::new(WHOLE));
    let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(String, u32, u32), String>>();
    let (n2, z2, s2, f2) = (neu.clone(), ctr.clone(), stop.clone(), error.clone());
    let b2 = region.clone();
    std::thread::Builder::new()
        .name("meetschirm".into())
        .spawn(move || event_loop(origin, max_b, max_h, fps, tx, n2, z2, s2, f2, b2))
        .map_err(|e| anyhow!("Bildschirmfaden: {}", e))?;
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok((name, b, h))) => Ok(Recording {
            name,
            width_px: b,
            height_px: h,
            origin,
            neu,
            ctr,
            stop,
            error,
            region,
        }),
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err(anyhow!("Bildschirmaufnahme antwortet nicht"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn event_loop(
    origin: Source,
    max_b: u32,
    max_h: u32,
    fps: u32,
    tx: std::sync::mpsc::Sender<std::result::Result<(String, u32, u32), String>>,
    neu: Arc<Mutex<Option<Image>>>,
    ctr: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<String>>,
    region: Arc<Mutex<Region>>,
) {
    let mut backend = match origin.open() {
        Some(b) => b,
        None => {
            let _ = tx.send(Err(match origin {
                Source::Display(_) => "kein Bildschirm aufnehmbar".to_string(),
                Source::Window(_) => "dieses Fenster laesst sich nicht aufnehmen".to_string(),
            }));
            return;
        }
    };
    let (sb, sh) = backend.size();
    let (zb, zh) = target_size(sb, sh, max_b, max_h);
    let display_name = match origin {
        Source::Display(index) => crate::capture::list_monitors(true)
            .get(index)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Bildschirm {}", index + 1)),
        Source::Window(k) => crate::window::liste()
            .into_iter()
            .find(|f| f.tag == k)
            .map(|f| f.heading)
            .unwrap_or_else(|| "Fenster".to_string()),
    };
    let name = format!("{} ({}x{} -> {}x{}, {})", display_name, sb, sh, zb, zh, backend.name());
    if tx.send(Ok((name, zb, zh))).is_err() {
        return;
    }

    let beat = (1000 / fps).max(8);
    let mut nv12: Vec<u8> = Vec::new();
    // Refresh: even without change an image every 400 ms, otherwise
    // a late joiner sees nothing.
    let mut last_delivery = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut losses = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let was = backend.next(beat);
        let liefern = match was {
            crate::capture::Next::Frame => true,
            crate::capture::Next::Unchanged => {
                last_delivery.elapsed() >= std::time::Duration::from_millis(400)
            }
            crate::capture::Next::Lost => {
                losses += 1;
                if let Ok(mut f) = error.lock() {
                    *f = format!("Aufnahme abgerissen ({}. Mal) - neu geoeffnet", losses);
                }
                match origin.open() {
                    Some(b) => {
                        backend = b;
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        continue;
                    }
                    None => {
                        if let Ok(mut f) = error.lock() {
                            *f = match origin {
                                Source::Display(_) => "Bildschirm nicht mehr aufnehmbar".into(),
                                // For a window the most common reason
                                // is simply: it was closed.
                                Source::Window(_) => "Das geteilte Fenster ist weg".to_string(),
                            };
                        }
                        return;
                    }
                }
            }
        };
        if !liefern {
            continue;
        }
        let part = region.lock().map(|g| *g).unwrap_or(WHOLE);
        // Fast path: the graphics card crops, scales and converts
        // in one go.
        //
        // IMPORTANT: the fallback to backend.frame() is NOT suitable here as
        // a substitute. As soon as the GPU scaler runs, the capture deliberately no longer
        // reads the full image back into RAM (saves PCIe) -
        // frame() then returns an old or empty buffer. That is exactly why
        // the sharp zoom turned black from about 140 %.
        let done = match backend.scaled_part(zb, zh, true, part) {
            Some(daten) if daten.len() >= (zb as usize * zh as usize * 3 / 2) => {
                nv12.clear();
                nv12.extend_from_slice(&daten[..(zb as usize * zh as usize * 3 / 2)]);
                true
            }
            _ => {
                // Without the graphics card path (older device, xcap fallback)
                // the full image really lies in main memory.
                let (px, sb2, sh2, bgra) = backend.frame();
                image_to_nv12_part(px, sb2, sh2, bgra, part, zb, zh, &mut nv12)
            }
        };
        if !done {
            continue;
        }
        // Paint the mouse pointer into the image. Windows does NOT put it in
        // during capture - without this step the viewer never sees where
        // is being pointed.
        if let Some((fx, fy)) = pointer_share_source(origin) {
            let (tx, ty, tw, th) = part;
            if fx >= tx && fy >= ty && fx <= tx + tw && fy <= ty + th {
                let px = ((fx - tx) / tw.max(0.0001) * zb as f32) as i32;
                let py = ((fy - ty) / th.max(0.0001) * zh as f32) as i32;
                // When zooming in the pointer is enlarged along with it - otherwise
                // it would be tiny in the section.
                let gr = (22.0 / tw.max(0.05)).clamp(18.0, 60.0);
                pointer_paint(&mut nv12, zb, zh, px, py, gr);
            }
        }
        last_delivery = std::time::Instant::now();
        ctr.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut b) = neu.lock() {
            *b = Some(Image {
                width_px: zb,
                height_px: zh,
                nv12: nv12.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_size_keeps_ratio_and_becomes_even() {
        let (b, h) = target_size(3840, 2160, 1920, 1080);
        assert_eq!((b, h), (1920, 1080));
        let (b, h) = target_size(1366, 768, 1920, 1080);
        assert_eq!((b, h), (1366, 768), "small screen is not scaled up");
        let (b, h) = target_size(2560, 1080, 1920, 1080);
        assert!(b <= 1920 && h <= 1080, "{}x{}", b, h);
        assert!(b % 2 == 0 && h % 2 == 0, "ungerade Kante {}x{}", b, h);
        // 21:9 bleibt 21:9 (auf 1 % genau)
        let v = b as f64 / h as f64;
        assert!((v - 2560.0 / 1080.0).abs() < 0.03, "Verhaeltnis {}", v);
    }

    #[test]
    /// The pointer must REALLY get into the image - and only where it
    /// belongs. Without this test it is only noticed by the viewer.
    #[test]
    fn pointer_lands_in_image() {
        let (w, h) = (64u32, 48u32);
        let mut nv12 = vec![100u8; (w * h * 3 / 2) as usize];
        pointer_paint(&mut nv12, w, h, 20, 10, 19.0);
        let y = &nv12[..(w * h) as usize];
        // At the tip there must be something light or dark, no longer 100.
        let environment: Vec<u8> = (10..14)
            .flat_map(|zy| (20..24).map(move |zx| (zy, zx)))
            .map(|(zy, zx)| y[zy * w as usize + zx])
            .collect();
        assert!(
            environment.iter().any(|v| *v != 100),
            "nothing was drawn at the pointer tip"
        );
        // Weit weg darf nichts angefasst worden sein.
        assert_eq!(y[40 * w as usize + 60], 100, "painted outside");
        // White must occur (the body of the arrow).
        assert!(y.iter().any(|v| *v > 200), "kein heller Pfeilkoerper");
        assert!(y.iter().any(|v| *v < 40), "kein dunkler Rand");
    }

    #[test]
    fn pointer_never_paints_beyond_the_edge() {
        let (w, h) = (32u32, 24u32);
        let length = (w * h * 3 / 2) as usize;
        for (x, y) in [(-5i32, -5i32), (31, 23), (0, 0), (100, 100)] {
            let mut nv12 = vec![50u8; length];
            pointer_paint(&mut nv12, w, h, x, y, 19.0);
            assert_eq!(nv12.len(), length, "Puffer veraendert");
        }
    }

    #[test]
    fn pointer_leaves_too_small_buffers_alone() {
        let mut klein = vec![7u8; 10];
        pointer_paint(&mut klein, 64, 48, 5, 5, 19.0);
        assert!(klein.iter().all(|v| *v == 7));
    }

    #[test]
    fn without_wish_the_whole_screen_goes_out() {
        assert_eq!(region_merge(&[]), WHOLE);
        assert_eq!(region_merge(&[(0.0, 0.0, 1.0, 1.0)]), WHOLE);
        // Almost everything is also everything - otherwise the fast path via the
        // graphics card is dropped for nothing.
        assert_eq!(region_merge(&[(0.02, 0.02, 0.9, 0.9)]), WHOLE);
    }

    #[test]
    fn viewer_gets_his_crop_with_border() {
        // Centre, a quarter of the edge.
        let (x, y, w, h) = region_merge(&[(0.375, 0.375, 0.25, 0.25)]);
        assert!((w - h).abs() < 1e-6, "muss quadratisch bleiben");
        assert!(w > 0.25 && w < 0.4, "border missing or too big: {}", w);
        // still centred
        assert!((x + w / 2.0 - 0.5).abs() < 0.01);
        assert!((y + h / 2.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn two_viewers_get_the_hull_around_both() {
        let a = (0.05, 0.05, 0.1, 0.1);
        let b = (0.55, 0.55, 0.1, 0.1);
        let (x, y, w, h) = region_merge(&[a, b]);
        assert!((w - h).abs() < 1e-6);
        // Both wishes must really lie inside - otherwise one
        // of the two would see the wrong section.
        assert!(x <= a.0 + 1e-6 && y <= a.1 + 1e-6, "erster faellt heraus");
        assert!(
            x + w >= b.0 + b.2 - 1e-6 && y + h >= b.1 + b.3 - 1e-6,
            "zweiter faellt heraus"
        );
    }

    #[test]
    fn region_always_stays_in_frame() {
        for wish in [
            (0.0, 0.0, 0.2, 0.2),
            (0.9, 0.9, 0.2, 0.2),
            (-0.5, 0.8, 0.3, 0.3),
        ] {
            let (x, y, w, h) = region_merge(&[wish]);
            assert!(x >= -1e-6 && y >= -1e-6, "links/oben raus: {:?}", (x, y));
            assert!(x + w <= 1.0 + 1e-6 && y + h <= 1.0 + 1e-6, "rechts/unten raus");
        }
    }

    #[test]
    fn small_twitches_trigger_no_switch() {
        let a = (0.30, 0.30, 0.30, 0.30);
        assert!(!region_worth_switch(a, (0.31, 0.31, 0.30, 0.30)));
        assert!(region_worth_switch(a, (0.40, 0.30, 0.30, 0.30)));
        assert!(region_worth_switch(a, (0.30, 0.30, 0.20, 0.20)));
    }

    /// The core of the matter: from a section REALLY only this
    /// section comes out - and in full target size, i.e. with real pixels.
    #[test]
    fn crop_sends_only_the_crop() {
        // 40x40, linke Haelfte schwarz, rechte Haelfte weiss.
        let (sb, sh) = (40u32, 40u32);
        let mut px = vec![0u8; (sb * sh * 4) as usize];
        for y in 0..sh as usize {
            for x in 20..40usize {
                let i = (y * sb as usize + x) * 4;
                px[i] = 255;
                px[i + 1] = 255;
                px[i + 2] = 255;
            }
        }
        let mut nv12 = Vec::new();
        // Send only the RIGHT half -> everything must be light.
        assert!(image_to_nv12_part(&px, sb, sh, true, (0.5, 0.0, 0.5, 0.5), 16, 16, &mut nv12));
        let y = &nv12[..16 * 16];
        assert!(
            y.iter().all(|v| *v > 200),
            "the crop does not show the right half"
        );
        // And the LEFT half -> everything dark.
        assert!(image_to_nv12_part(&px, sb, sh, true, (0.0, 0.0, 0.5, 0.5), 16, 16, &mut nv12));
        let y = &nv12[..16 * 16];
        assert!(y.iter().all(|v| *v < 60), "left crop is not dark");
    }

    #[test]
    fn image_to_nv12_has_right_size_and_color() {
        // Ein reines Rot in BGRA: B=0, G=0, R=255.
        let (sb, sh) = (64u32, 32u32);
        let mut px = vec![0u8; (sb * sh * 4) as usize];
        for p in px.chunks_mut(4) {
            p[0] = 0;
            p[1] = 0;
            p[2] = 255;
            p[3] = 255;
        }
        let mut nv12 = Vec::new();
        assert!(image_to_nv12(&px, sb, sh, true, 32, 16, &mut nv12));
        assert_eq!(nv12.len(), 32 * 16 * 3 / 2);
        // Y of red is around 82 (BT.601) - clearly away from 0 and 255.
        let y0 = nv12[0];
        assert!((60..110).contains(&y0), "Y of red is {}", y0);
    }

    #[test]
    fn image_to_nv12_averages_when_shrinking() {
        // Black on the left, white on the right: after scaling down the left must stay
        // dark and the right light (no offset, no uniform grey).
        let (sb, sh) = (64u32, 32u32);
        let mut px = vec![255u8; (sb * sh * 4) as usize];
        for y in 0..sh as usize {
            for x in 0..(sb as usize / 2) {
                let i = (y * sb as usize + x) * 4;
                px[i] = 0;
                px[i + 1] = 0;
                px[i + 2] = 0;
            }
        }
        let mut nv12 = Vec::new();
        assert!(image_to_nv12(&px, sb, sh, false, 16, 8, &mut nv12));
        let links = nv12[0];
        let right_side = nv12[15];
        assert!(links < 40, "links sollte dunkel sein: {}", links);
        assert!(right_side > 200, "rechts sollte hell sein: {}", right_side);
    }

    #[test]
    fn too_small_inputs_are_rejected_instead_of_crashing() {
        let mut out = Vec::new();
        assert!(!image_to_nv12(&[0u8; 4], 1, 1, true, 32, 16, &mut out));
        assert!(!image_to_nv12(&[], 0, 0, false, 32, 16, &mut out));
    }

    /// Stage 5b: the WHOLE chain on this machine - capture the screen,
    /// scale down, to NV12, through the H.264 encoder. Runs for real on Windows
    /// and Mac; where there is no screen (server), it is honestly
    /// skipped instead of falsely reporting green.
    #[test]
    fn display_through_h264() {
        let screens = liste();
        if screens.is_empty() {
            println!("no screen on this machine - skipped");
            return;
        }
        println!("Bildschirm 0: {:?}", screens[0]);
        let auf = match open(0, 1280, 720, 10) {
            Ok(a) => a,
            Err(e) => {
                println!("Aufnahme nicht moeglich ({}) - uebersprungen", e);
                return;
            }
        };
        let (zb, zh) = (auf.width_px, auf.height_px);
        assert!(zb % 2 == 0 && zh % 2 == 0, "ungerade Kante {}x{}", zb, zh);
        assert!(zb <= 1280 && zh <= 720, "zu gross: {}x{}", zb, zh);
        let start = std::time::Instant::now();
        let mut picture = None;
        while start.elapsed() < std::time::Duration::from_secs(5) && picture.is_none() {
            picture = auf.newest();
            if picture.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        let Some(b) = picture else {
            println!(
                "kein Bild binnen 5 s (Fehler: {}) - uebersprungen",
                auf.error()
            );
            return;
        };
        assert_eq!(b.width_px, zb);
        assert_eq!(b.height_px, zh);
        assert_eq!(
            b.nv12.len(),
            zb as usize * zh as usize * 3 / 2,
            "NV12-Laenge passt nicht"
        );
        // How bright is the image really? With a locked machine it is
        // black - that is not an error, but it should be stated.
        let ysum: u64 = b.nv12[..(zb as usize * zh as usize)]
            .iter()
            .map(|v| *v as u64)
            .sum();
        let hell = ysum as f64 / (zb as f64 * zh as f64);
        println!(
            "aufgenommen {}x{}, {} Bilder, mittlere Helligkeit {:.1}",
            zb,
            zh,
            auf.recorded(),
            hell
        );
        // And now through the encoder - that is the part that is
        // new on the Mac.
        if !crate::h264::available() {
            println!("no H.264 encoder - chain cannot be checked here");
            return;
        }
        let mut enc = crate::h264::Encoder::new(zb, zh, 10, 3_000_000).expect("Kodierer");
        let mut bytes = 0usize;
        let mut key_frames = 0;
        // Media Foundation does NOT release the first frame immediately (the
        // chain only starts up) - VideoToolbox does. Hence push a few frames
        // after instead of making a statement from a single image.
        for _ in 0..15 {
            for c in enc.encode(&b.nv12).expect("kodieren") {
                bytes += c.data.len();
                if c.key {
                    key_frames += 1;
                }
            }
            if bytes > 0 && key_frames > 0 {
                break;
            }
        }
        println!("H.264: {} Bytes, {} Schluesselbilder", bytes, key_frames);
        assert!(bytes > 0, "screen frame could not be encoded");
        assert!(key_frames > 0, "no keyframe at the start");
        auf.stop_capture();
    }

    #[test]
    fn list_does_not_crash() {
        // On a server without a screen this must give a list (possibly empty)
        // and not blow up.
        let l = liste();
        println!("Bildschirme: {}", l.len());
    }
}
