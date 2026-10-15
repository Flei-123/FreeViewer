//! Native meeting - stage 2d: the REAL camera.
//!
//! Until now a test pattern went into the meeting (stage 2b). Here the image
//! now comes from the camera - on Windows via Media Foundation, i.e.
//! exactly the machinery the browser uses too. The capture thread
//! only ever holds the NEWEST frame ready; whoever picks up more slowly
//! skips frames instead of building up a backlog.
//!
//! What is delivered is tightly packed NV12 in exactly the requested size.
//! If the camera cannot do the format/size, it is cropped here
//! and scaled down - so the H.264 encoder always gets what it
//! expects, whichever camera is plugged in.
//!
//! macOS follows in stage 5 (AVFoundation), Linux has nothing to do here.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Eine gefundene Kamera.
#[derive(Clone, Debug, PartialEq)]
pub struct Geraet {
    pub name: String,
    /// Eindeutiger Pfad des Geraetes (Windows: symbolischer Link).
    pub id: String,
}

/// A frame in tightly packed NV12 (Y plane, then interleaved UV).
#[derive(Clone)]
pub struct Bild {
    pub breite: u32,
    pub hoehe: u32,
    pub nv12: Vec<u8>,
}

/// Copy NV12 with row stride into tightly packed NV12.
///
/// Media Foundation often delivers rows wider than the image (stride), because
/// the graphics card likes it that way. The encoder wants it tightly packed.
pub fn nv12_packen(src: &[u8], stride: usize, w: u32, h: u32, out: &mut Vec<u8>) -> bool {
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 || stride < wu {
        return false;
    }
    // Y: h Zeilen, UV: h/2 Zeilen - beides mit demselben Zeilenabstand.
    let noetig = stride * hu + stride * (hu / 2);
    if src.len() < noetig {
        return false;
    }
    out.clear();
    out.reserve(wu * hu * 3 / 2);
    for y in 0..hu {
        let a = y * stride;
        out.extend_from_slice(&src[a..a + wu]);
    }
    let uv0 = stride * hu;
    for y in 0..hu / 2 {
        let a = uv0 + y * stride;
        out.extend_from_slice(&src[a..a + wu]);
    }
    true
}

/// Derive the REAL image size from buffer size and row stride.
///
/// Media Foundation may report one size in the media type and then deliver
/// another - measured on 06.08.2026: announced 640x360 NV12,
/// delivered a buffer with 1 382 400 bytes and row stride 1280, i.e. in
/// truth 1280x720. Whoever believes the report copies the top left
/// quarter (image looks zoomed in) and reads the colour plane from the middle of the
/// luma (image turns green/magenta). Row stride and buffer size
/// do not lie - so we compute the truth from them.
///
/// Returns: the size with which the buffer MAY be read.
pub fn echte_groesse(len: usize, stride: usize, w: u32, h: u32) -> Option<(u32, u32)> {
    let (wu, hu) = (w as usize, h as usize);
    if stride == 0 || wu == 0 || hu == 0 || stride < wu {
        return None;
    }
    // Eine NV12-Bildzeile kostet stride Bytes Helligkeit plus stride/2 Bytes
    // Farbe - zusammen stride*3/2. Daraus folgt, wie viele Zeilen WIRKLICH
    // im Puffer liegen.
    let je_zeile = stride * 3 / 2;
    if je_zeile == 0 {
        return None;
    }
    let zeilen = ((len / je_zeile) as u32) & !1;
    if zeilen < 2 || (zeilen as usize) < hu {
        // Less data than reported: the buffer does not lie there with this
        // row stride (usually tightly packed). Do NOT guess here - the
        // flat fallback reads it correctly.
        return None;
    }
    if zeilen as usize == hu {
        return Some((w, h)); // Meldung und Puffer passen zusammen
    }
    // MORE rows than reported -> the media type lied. Then the
    // row stride is also the real width (cameras deliver NV12 without
    // padding bytes) and the row count the real height.
    Some(((stride as u32) & !1, zeilen))
}

/// Take a centred section in the target ratio and scale it down to the target size
/// (area average, so it does not flicker).
///
/// Why crop instead of squash: a 16:9 image squashed into a 4:3 window
/// makes long faces. Zoom/Teams crop as well.
pub fn nv12_zuschneiden_skalieren(
    src: &[u8],
    sw: u32,
    sh: u32,
    zw: u32,
    zh: u32,
    out: &mut Vec<u8>,
) -> bool {
    if sw < 2 || sh < 2 || zw < 2 || zh < 2 {
        return false;
    }
    let (swu, shu) = (sw as usize, sh as usize);
    if src.len() < swu * shu * 3 / 2 {
        return false;
    }
    let (zwu, zhu) = (zw as usize, zh as usize);
    // Section in the target ratio, always even edges (NV12 needs that).
    let (mut cw, mut ch) = (swu, shu);
    if sw as u64 * zh as u64 > zw as u64 * sh as u64 {
        cw = ((shu * zwu) / zhu) & !1;
    } else {
        ch = ((swu * zhu) / zwu) & !1;
    }
    let cw = cw.max(2).min(swu) & !1;
    let ch = ch.max(2).min(shu) & !1;
    let cx = ((swu - cw) / 2) & !1;
    let cy = ((shu - ch) / 2) & !1;

    out.clear();
    out.resize(zwu * zhu * 3 / 2, 0);

    // ---- Fast path: exact halving ----
    //
    // The most common case is 1280x720 -> 640x360 (the camera can only do
    // 720p, 360p is needed). The general path below recomputes the source bounds for
    // EVERY target point - measured 46 ms per frame in the
    // debug build, which would make 30 frames per second impossible. With exact
    // halving the four source points are fixed, though; that saves all the
    // index arithmetic.
    if cw == zwu * 2 && ch == zhu * 2 {
        let uv_src = swu * shu;
        let uv_dst = zwu * zhu;
        for ty in 0..zhu {
            let z0 = (cy + ty * 2) * swu + cx;
            let z1 = z0 + swu;
            let zeile = ty * zwu;
            for tx in 0..zwu {
                let a = z0 + tx * 2;
                let b = z1 + tx * 2;
                let summe = src[a] as u32 + src[a + 1] as u32 + src[b] as u32 + src[b + 1] as u32;
                out[zeile + tx] = (summe >> 2) as u8;
            }
        }
        // Colour: already at half resolution, so 2x2 colour pairs.
        let sw2 = swu; // Bytes je Farbzeile = Breite (je Punktpaar U und V)
        let (cx2, cy2) = (cx / 2, cy / 2);
        for ty in 0..zhu / 2 {
            let z0 = uv_src + (cy2 + ty * 2) * sw2 + cx2 * 2;
            let z1 = z0 + sw2;
            let zeile = uv_dst + ty * zwu;
            for tx in 0..zwu / 2 {
                let a = z0 + tx * 4;
                let b = z1 + tx * 4;
                let u = src[a] as u32 + src[a + 2] as u32 + src[b] as u32 + src[b + 2] as u32;
                let v = src[a + 1] as u32 + src[a + 3] as u32 + src[b + 1] as u32 + src[b + 3] as u32;
                out[zeile + tx * 2] = (u >> 2) as u8;
                out[zeile + tx * 2 + 1] = (v >> 2) as u8;
            }
        }
        return true;
    }

    // --- Y ---
    for ty in 0..zhu {
        let y0 = cy + ty * ch / zhu;
        let y1 = (cy + (ty + 1) * ch / zhu).max(y0 + 1).min(cy + ch);
        for tx in 0..zwu {
            let x0 = cx + tx * cw / zwu;
            let x1 = (cx + (tx + 1) * cw / zwu).max(x0 + 1).min(cx + cw);
            let mut summe = 0u32;
            let mut n = 0u32;
            for y in y0..y1 {
                let row = y * swu;
                for x in x0..x1 {
                    summe += src[row + x] as u32;
                    n += 1;
                }
            }
            out[ty * zwu + tx] = (summe / n.max(1)) as u8;
        }
    }
    // --- UV (half resolution, U and V alternating) ---
    let uv_src = swu * shu;
    let uv_dst = zwu * zhu;
    let (cw2, ch2, cx2, cy2) = (cw / 2, ch / 2, cx / 2, cy / 2);
    let sw2 = swu / 2;
    for ty in 0..zhu / 2 {
        let y0 = cy2 + ty * ch2 / (zhu / 2);
        let y1 = (cy2 + (ty + 1) * ch2 / (zhu / 2)).max(y0 + 1).min(cy2 + ch2);
        for tx in 0..zwu / 2 {
            let x0 = cx2 + tx * cw2 / (zwu / 2);
            let x1 = (cx2 + (tx + 1) * cw2 / (zwu / 2)).max(x0 + 1).min(cx2 + cw2);
            let (mut su, mut sv, mut n) = (0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = uv_src + y * sw2 * 2;
                for x in x0..x1 {
                    su += src[row + x * 2] as u32;
                    sv += src[row + x * 2 + 1] as u32;
                    n += 1;
                }
            }
            let o = uv_dst + ty * zwu + tx * 2;
            out[o] = (su / n.max(1)) as u8;
            out[o + 1] = (sv / n.max(1)) as u8;
        }
    }
    true
}

/// Runs in the background and keeps the newest camera frame ready.
pub struct Kamera {
    pub name: String,
    pub breite: u32,
    pub hoehe: u32,
    neu: Arc<Mutex<Option<Bild>>>,
    zaehler: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fehler: Arc<Mutex<String>>,
}

impl Kamera {
    /// Pick up the newest frame (and take it out of the buffer). If None comes,
    /// there has been no new one since the last pickup.
    pub fn neuestes(&self) -> Option<Bild> {
        self.neu.lock().ok().and_then(|mut b| b.take())
    }

    /// How many frames the camera has delivered so far.
    pub fn aufgenommen(&self) -> u64 {
        self.zaehler.load(Ordering::Relaxed)
    }

    pub fn fehler(&self) -> String {
        self.fehler.lock().map(|f| f.clone()).unwrap_or_default()
    }

    pub fn stoppen(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Kamera {
    fn drop(&mut self) {
        self.stoppen();
    }
}

/// Alle Kameras des Rechners.
pub fn liste() -> Vec<Geraet> {
    #[cfg(windows)]
    {
        // Media Foundation is the main path and therefore comes first.
        // After that comes DirectShow: the OBS virtual camera registers ONLY
        // there and was completely missing from the list so far. What Media
        // Foundation already knows is not listed twice - otherwise
        // webcams would appear twice, and you would choose the worse
        // of the two paths.
        let mut aus = win::liste();
        for g in crate::camdshow::liste() {
            let doppelt = aus
                .iter()
                .any(|x| x.name.eq_ignore_ascii_case(&g.name));
            if !doppelt {
                aus.push(g);
            }
        }
        aus
    }
    #[cfg(target_os = "macos")]
    {
        mac::liste()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Vec::new()
    }
}

/// Open the camera. `id` empty = first camera. What is always delivered is exactly
/// `breite` x `hoehe` in NV12.
pub fn oeffnen(id: Option<String>, breite: u32, hoehe: u32, fps: u32) -> Result<Kamera> {
    let breite = breite & !1;
    let hoehe = hoehe & !1;
    if breite < 2 || hoehe < 2 {
        return Err(anyhow!("unsinnige Bildgroesse"));
    }
    #[cfg(windows)]
    {
        let neu: Arc<Mutex<Option<Bild>>> = Arc::new(Mutex::new(None));
        let zaehler = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let fehler = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<String, String>>();
        let (n2, z2, s2, f2) = (neu.clone(), zaehler.clone(), stop.clone(), fehler.clone());
        // If the id carries the DirectShow prefix, it belongs to a
        // source that Media Foundation does not know at all (OBS & co.).
        let ueber_dshow = id
            .as_deref()
            .is_some_and(|x| x.starts_with(crate::camdshow::PRAEFIX));
        std::thread::Builder::new()
            .name("kamera".into())
            .spawn(move || {
                if ueber_dshow {
                    crate::camdshow::schleife(id, breite, hoehe, fps, tx, n2, z2, s2, f2)
                } else {
                    win::schleife(id, breite, hoehe, fps, tx, n2, z2, s2, f2)
                }
            })
            .map_err(|e| anyhow!("Kamerafaden: {}", e))?;
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(name)) => Ok(Kamera {
                name,
                breite,
                hoehe,
                neu,
                zaehler,
                stop,
                fehler,
            }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(anyhow!("Kamera antwortet nicht"))
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let neu: Arc<Mutex<Option<Bild>>> = Arc::new(Mutex::new(None));
        let zaehler = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let fehler = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<String, String>>();
        let (n2, z2, s2, f2) = (neu.clone(), zaehler.clone(), stop.clone(), fehler.clone());
        std::thread::Builder::new()
            .name("kamera".into())
            .spawn(move || mac::schleife(id, breite, hoehe, fps, tx, n2, z2, s2, f2))
            .map_err(|e| anyhow!("Kamerafaden: {}", e))?;
        match rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(Ok(name)) => Ok(Kamera {
                name,
                breite,
                hoehe,
                neu,
                zaehler,
                stop,
                fehler,
            }),
            Ok(Err(e)) => Err(anyhow!(e)),
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(anyhow!("Kamera antwortet nicht"))
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (id, fps);
        Err(anyhow!("Kamera auf dieser Plattform nicht angebunden"))
    }
}

/// Bring RGB (3 bytes per point) to `zb` x `zh` and convert to NV12.
///
/// Cameras rarely deliver exactly the requested size. Instead of stretching,
/// the largest possible CENTRED section in the target ratio is taken
/// and then scaled down by area average - so the face stays round and
/// the text sharp.
pub fn rgb_nach_nv12(rgb: &[u8], sb: u32, sh: u32, zb: u32, zh: u32, out: &mut Vec<u8>) -> bool {
    let (sbu, shu, zbu, zhu) = (sb as usize, sh as usize, zb as usize, zh as usize);
    if sbu < 2 || shu < 2 || zbu < 2 || zhu < 2 || rgb.len() < sbu * shu * 3 {
        return false;
    }
    // Mittiger Ausschnitt im Zielverhaeltnis.
    let (mut ab, mut ah) = (sbu, shu);
    if sbu * zhu > zbu * shu {
        ab = shu * zbu / zhu;
    } else {
        ah = sbu * zhu / zbu;
    }
    let ab = ab.max(2).min(sbu);
    let ah = ah.max(2).min(shu);
    let (ox, oy) = ((sbu - ab) / 2, (shu - ah) / 2);
    let mut klein = vec![0u8; zbu * zhu * 3];
    for ty in 0..zhu {
        let y0 = oy + ty * ah / zhu;
        let y1 = (oy + ((ty + 1) * ah / zhu)).max(y0 + 1).min(shu);
        for tx in 0..zbu {
            let x0 = ox + tx * ab / zbu;
            let x1 = (ox + ((tx + 1) * ab / zbu)).max(x0 + 1).min(sbu);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let zeile = y * sbu * 3;
                for x in x0..x1 {
                    let i = zeile + x * 3;
                    r += rgb[i] as u32;
                    g += rgb[i + 1] as u32;
                    b += rgb[i + 2] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (ty * zbu + tx) * 3;
            klein[o] = (r / n) as u8;
            klein[o + 1] = (g / n) as u8;
            klein[o + 2] = (b / n) as u8;
        }
    }
    crate::h264::rgb_to_nv12(&klein, zb, zh, out);
    true
}


/// Camera diagnostics (Windows only). Writes cam-roh.png / cam-fertig.png
/// to `ordner` and returns the measurement report as text.
pub fn diagnose(id: Option<String>, breite: u32, hoehe: u32, fps: u32, ordner: &str) -> String {
    #[cfg(windows)]
    {
        win::diagnose(id, breite & !1, hoehe & !1, fps, ordner)
    }
    #[cfg(not(windows))]
    {
        let _ = (id, breite, hoehe, fps, ordner);
        "Kamera-Diagnose gibt es nur unter Windows".to_string()
    }
}

// ---------------------------------------------------------------- macOS ----

/// Camera on the Mac. AVFoundation is Objective-C - instead of building a
/// delegate class ourselves, nokhwa takes care of that (only linked in for this
/// platform). The rest is the same flow as on Windows:
/// own thread, only ever the NEWEST frame, tightly packed NV12.
#[cfg(target_os = "macos")]
mod mac {
    use super::{rgb_nach_nv12, Bild, Geraet};
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    };
    use nokhwa::{query, Camera};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    pub fn liste() -> Vec<Geraet> {
        match query(ApiBackend::AVFoundation) {
            Ok(v) => v
                .into_iter()
                .map(|c| Geraet {
                    name: c.human_name(),
                    id: c.index().to_string(),
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn schleife(
        id: Option<String>,
        breite: u32,
        hoehe: u32,
        fps: u32,
        tx: std::sync::mpsc::Sender<std::result::Result<String, String>>,
        neu: Arc<Mutex<Option<Bild>>>,
        zaehler: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        fehler: Arc<Mutex<String>>,
    ) {
        // Which camera? Without a specification the first one.
        //
        // IMPORTANT: the selection in the window delivers a NAME (as
        // liste() reports it), not a number. Earlier only a number was
        // read here - every name thereby fell back to camera 0, and on the
        // Mac the camera simply could not be switched. Hence search
        // by name first, then by number, then the first.
        let welche = match id.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            Some(w) => match w.parse::<u32>() {
                Ok(n) => CameraIndex::Index(n),
                Err(_) => {
                    let klein = w.to_lowercase();
                    let treffer = query(ApiBackend::AVFoundation)
                        .ok()
                        .and_then(|v| {
                            v.into_iter().find(|c| {
                                let n = c.human_name().to_lowercase();
                                n == klein || n.contains(&klein) || klein.contains(&n)
                            })
                        })
                        .map(|c| c.index().clone());
                    match treffer {
                        Some(i) => i,
                        None => CameraIndex::Index(0),
                    }
                }
            },
            None => CameraIndex::Index(0),
        };
        // Closest setting to the wanted size and frame rate. NV12 is
        // what cameras can do most often; nokhwa converts if necessary.
        let wunsch = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
            CameraFormat::new(Resolution::new(breite, hoehe), FrameFormat::NV12, fps),
        ));
        let mut kamera = match Camera::new(welche, wunsch) {
            Ok(k) => k,
            Err(e) => {
                let _ = tx.send(Err(format!("Kamera nicht zu oeffnen: {}", e)));
                return;
            }
        };
        if let Err(e) = kamera.open_stream() {
            let _ = tx.send(Err(format!("Kamera startet nicht: {}", e)));
            return;
        }
        let auf = kamera.resolution();
        let name = format!(
            "{} ({}x{} -> {}x{})",
            kamera.info().human_name(),
            auf.width(),
            auf.height(),
            breite,
            hoehe
        );
        if tx.send(Ok(name)).is_err() {
            return;
        }
        let mut nv12: Vec<u8> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let rahmen = match kamera.frame() {
                Ok(r) => r,
                Err(e) => {
                    if let Ok(mut f) = fehler.lock() {
                        *f = format!("Kamera liefert nichts: {}", e);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
            };
            let bild = match rahmen.decode_image::<RgbFormat>() {
                Ok(b) => b,
                Err(e) => {
                    if let Ok(mut f) = fehler.lock() {
                        *f = format!("Bild nicht lesbar: {}", e);
                    }
                    continue;
                }
            };
            let (sb, sh) = (bild.width(), bild.height());
            if !rgb_nach_nv12(bild.as_raw(), sb, sh, breite, hoehe, &mut nv12) {
                continue;
            }
            zaehler.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut b) = neu.lock() {
                *b = Some(Bild {
                    breite,
                    hoehe,
                    nv12: nv12.clone(),
                });
            }
        }
        let _ = kamera.stop_stream();
    }
}

// ------------------------------------------------------------- windows -----

#[cfg(windows)]
mod win {
    use super::{Bild, Geraet};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, Once};
    use windows::core::{Interface, GUID, PWSTR};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
    };

    static MF_INIT: Once = Once::new();

    fn mf_startup() {
        // SAFETY: MFStartup has no pointer arguments and is guarded by `Once`, so it runs at most
        // once per process; its result is ignored on purpose (later Media Foundation calls then
        // fail with an error that is propagated).
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
        });
    }

    fn pack(a: u32, b: u32) -> u64 {
        ((a as u64) << 32) | b as u64
    }

    // SAFETY: caller contract: `a` is a live IMFActivate and `schluessel` a valid GUID reference.
    // `p` and `len` are local out-slots. On success GetAllocatedString returns a NUL-terminated
    // CoTaskMem string that we own: it is copied into a Rust String and freed exactly once with
    // CoTaskMemFree.
    unsafe fn text(a: &IMFActivate, schluessel: &GUID) -> String {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        if a.GetAllocatedString(schluessel, &mut p, &mut len).is_err() {
            return String::new();
        }
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }

    // SAFETY: caller contract: COM must be initialised on the calling thread, and `f` must not keep
    // the borrowed IMFActivate beyond the call (clone it if needed). MFEnumDeviceSources allocates
    // the array `list` (`count` entries of Option<IMFActivate>) with CoTaskMemAlloc and gives
    // ownership to us; it is checked for null, every index stays below `count`, entries are only
    // borrowed while `f` runs, then each entry is released once with drop_in_place and the array is
    // freed once with CoTaskMemFree; nothing from `list` is used after that.
    /// List cameras. Calls `f` for each one; returns the first
    /// Some result of `f` (so you can search OR list in one go).
    unsafe fn mit_kameras<T>(mut f: impl FnMut(&IMFActivate, &str, &str) -> Option<T>) -> Option<T> {
        mf_startup();
        let mut attrs: Option<IMFAttributes> = None;
        if MFCreateAttributes(&mut attrs, 1).is_err() {
            return None;
        }
        let attrs = attrs?;
        if attrs
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .is_err()
        {
            return None;
        }
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        if MFEnumDeviceSources(&attrs, &mut list, &mut count).is_err() || list.is_null() {
            return None;
        }
        let mut gefunden = None;
        for i in 0..count as usize {
            if gefunden.is_none() {
                if let Some(a) = (*list.add(i)).as_ref() {
                    let name = text(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
                    let id = text(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK);
                    gefunden = f(a, &name, &id);
                }
            }
        }
        for i in 0..count as usize {
            std::ptr::drop_in_place(list.add(i));
        }
        CoTaskMemFree(Some(list as *const _));
        gefunden
    }

    pub fn liste() -> Vec<Geraet> {
        let mut out: Vec<Geraet> = Vec::new();
        // SAFETY: `mit_kameras` only needs COM/Media Foundation on this thread, which the callers
        // of `liste` set up or which are initialised by `mf_startup`; the closure borrows the local
        // `out` and no raw pointer leaves the block.
        unsafe {
            // Never return Some -> runs through all devices.
            mit_kameras::<()>(|_, name, id| {
                out.push(Geraet {
                    name: name.to_string(),
                    id: id.to_string(),
                });
                None
            });
        }
        out
    }

    /// The capture thread. Lives until `stop` is set.
    #[allow(clippy::too_many_arguments)]
    pub fn schleife(
        id: Option<String>,
        zw: u32,
        zh: u32,
        fps: u32,
        tx: std::sync::mpsc::Sender<std::result::Result<String, String>>,
        neu: Arc<Mutex<Option<Bild>>>,
        zaehler: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        fehler: Arc<Mutex<String>>,
    ) {
        // SAFETY: this runs on its own capture thread: CoInitializeEx before any COM call and
        // CoUninitialize after `schleife_inner` has returned, so every COM interface created there
        // (all locals of that function) has already been dropped when COM is torn down. Known nit:
        // CoUninitialize also runs if CoInitializeEx failed (see roadmap); it cannot fail on a
        // fresh thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let r = schleife_inner(id, zw, zh, fps, &tx, &neu, &zaehler, &stop, &fehler);
            if let Err(e) = r {
                // If the start already failed, the caller has not yet received
                // anything - the message has to go to them.
                let _ = tx.send(Err(e.clone()));
                if let Ok(mut f) = fehler.lock() {
                    *f = e;
                }
            }
            CoUninitialize();
        }
    }

    // SAFETY: caller contract: COM is initialised on the current thread until return; the function
    // owns every interface it creates and drops them before returning. All out-slots (`attrs`,
    // `flags`, `ts`, `sample`) are locals that outlive the calls. Pixel data only leaves through
    // `bild_holen`, which copies into `roh` before the sample is released; the later accesses to
    // `roh`/`fertig` are safe slice code.
    #[allow(clippy::too_many_arguments)]
    unsafe fn schleife_inner(
        id: Option<String>,
        zw: u32,
        zh: u32,
        fps: u32,
        tx: &std::sync::mpsc::Sender<std::result::Result<String, String>>,
        neu: &Arc<Mutex<Option<Bild>>>,
        zaehler: &Arc<AtomicU64>,
        stop: &Arc<AtomicBool>,
        fehler: &Arc<Mutex<String>>,
    ) -> std::result::Result<(), String> {
        mf_startup();
        let gesucht = id.unwrap_or_default();
        let treffer = mit_kameras(|a, name, gid| {
            if gesucht.is_empty()
                || gid.eq_ignore_ascii_case(&gesucht)
                || name.to_lowercase().contains(&gesucht.to_lowercase())
            {
                match a.ActivateObject::<IMFMediaSource>() {
                    Ok(src) => Some(Ok((src, name.to_string()))),
                    Err(e) => Some(Err(format!("Kamera {} laesst sich nicht oeffnen: {}", name, e))),
                }
            } else {
                None
            }
        });
        let (quelle, name) = match treffer {
            Some(Ok(t)) => t,
            Some(Err(e)) => return Err(e),
            None => return Err("keine Kamera gefunden".to_string()),
        };

        // The reader may convert (colour format, size) - otherwise we would have
        // to decode MJPEG ourselves.
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2).map_err(|e| format!("Attribute: {}", e))?;
        let attrs = attrs.ok_or_else(|| "Attribute leer".to_string())?;
        let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
        let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
        let leser = MFCreateSourceReaderFromMediaSource(&quelle, &attrs)
            .map_err(|e| format!("Leser: {}", e))?;
        let strom = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        // FIRST take a format that the camera REALLY can do.
        //
        // Earlier the wanted size was simply set. The reader says
        // "yes" to it - and still delivers the native image (measured:
        // 640x360 announced, 1280x720 delivered). The result was a
        // cut-up, green image. A native format cannot
        // lie; scaling down is done afterwards with our own, tested
        // calculation path.
        let mut skalieren = true;
        let mut gesetzt = false;
        if let Some(t) = bestes_format(&leser, strom, zw, zh, fps) {
            if leser.SetCurrentMediaType(strom, None, &t).is_ok() {
                gesetzt = true;
            }
        }
        if !gesetzt {
            // Fallback: only set the colour format, leave the size open.
            let mt2 = MFCreateMediaType().map_err(|e| format!("Medientyp2: {}", e))?;
            mt2.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|e| format!("Haupttyp2: {}", e))?;
            mt2.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                .map_err(|e| format!("Untertyp2: {}", e))?;
            leser
                .SetCurrentMediaType(strom, None, &mt2)
                .map_err(|e| format!("Kamera kann kein NV12: {}", e))?;
        }
        let _ = leser.SetStreamSelection(strom, true);

        // What does the camera really deliver now?
        let (mut qw, mut qh) = groesse(&leser, strom).unwrap_or((zw, zh));
        skalieren = qw != zw || qh != zh;
        let _ = tx.send(Ok(name));

        let mut roh: Vec<u8> = Vec::new();
        let mut fertig: Vec<u8> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample: Option<IMFSample> = None;
            if let Err(e) = leser.ReadSample(
                strom,
                0,
                None,
                Some(&mut flags),
                Some(&mut ts),
                Some(&mut sample),
            ) {
                if let Ok(mut f) = fehler.lock() {
                    *f = format!("lesen: {}", e);
                }
                break;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                if let Ok(mut f) = fehler.lock() {
                    *f = "Kamera hat den Strom beendet".into();
                }
                break;
            }
            if flags
                & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                    | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32)
                != 0
            {
                if let Some((w, h)) = groesse(&leser, strom) {
                    qw = w;
                    qh = h;
                    skalieren = qw != zw || qh != zh;
                }
            }
            let s = match sample {
                Some(s) => s,
                None => continue, // Kamera hat gerade nichts (Pause/Tick)
            };
            let (rw, rh) = match bild_holen(&s, qw, qh, &mut roh) {
                Some(g) => g,
                None => continue,
            };
            if rw != qw || rh != qh {
                // The media type lied - from now on keep calculating with the real
                // size, otherwise the scaler cuts into
                // a wrong section.
                qw = rw;
                qh = rh;
                skalieren = qw != zw || qh != zh;
            }
            let bild = if skalieren {
                if !super::nv12_zuschneiden_skalieren(&roh, qw, qh, zw, zh, &mut fertig) {
                    continue;
                }
                Bild {
                    breite: zw,
                    hoehe: zh,
                    nv12: fertig.clone(),
                }
            } else {
                Bild {
                    breite: zw,
                    hoehe: zh,
                    nv12: roh.clone(),
                }
            };
            zaehler.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut b) = neu.lock() {
                *b = Some(bild);
            }
        }
        let _ = leser.Flush(strom);
        let _ = quelle.Shutdown();
        Ok(())
    }


    /// Camera diagnostics: what the device offers, what we really get,
    /// and what the raw image looks like.
    ///
    /// WHY: Justin's image arrived green/magenta and strongly zoomed in. Whether
    /// that is down to the colour space, the row stride or the cropping can
    /// only be measured on the REAL device - guessing does not help here.
    pub fn diagnose(id: Option<String>, zw: u32, zh: u32, fps: u32, ordner: &str) -> String {
        // SAFETY: COM is initialised on this thread first and uninitialised after `diagnose_inner`
        // returned, which has dropped all its interfaces by then (same pattern as `schleife`); the
        // call has no pointer arguments of its own.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let r = diagnose_inner(id, zw, zh, fps, ordner);
            CoUninitialize();
            r
        }
    }

    fn fourcc(g: &GUID) -> String {
        // MF video subtypes are FOURCCs in a fixed GUID wrapper:
        // XXXXXXXX-0000-0010-8000-00AA00389B71 - the first four bytes
        // directly give the name (NV12, YUY2, MJPG ...).
        let d1 = g.data1.to_le_bytes();
        if g.data2 == 0 && g.data3 == 0x10 && g.data4 == [0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71] {
            let t: String = d1
                .iter()
                .map(|b| {
                    if (32..127).contains(b) {
                        *b as char
                    } else {
                        '?'
                    }
                })
                .collect();
            if d1[0] == 20 {
                return "RGB32".into();
            }
            if d1[0] == 21 {
                return "ARGB32".into();
            }
            if d1[0] == 22 {
                return "RGB24".into();
            }
            return t;
        }
        format!("{:?}", g)
    }

    // SAFETY: caller contract: `t` is a live IMFMediaType. It only reads attributes by value (GUID,
    // u64, u32) and keeps no pointers.
    unsafe fn typ_text(t: &IMFMediaType) -> String {
        let sub = t.GetGUID(&MF_MT_SUBTYPE).map(|g| fourcc(&g)).unwrap_or_else(|_| "?".into());
        let (w, h) = match t.GetUINT64(&MF_MT_FRAME_SIZE) {
            Ok(v) => ((v >> 32) as u32, (v & 0xffff_ffff) as u32),
            Err(_) => (0, 0),
        };
        let fr = match t.GetUINT64(&MF_MT_FRAME_RATE) {
            Ok(v) => {
                let (n, d) = ((v >> 32) as u32, (v & 0xffff_ffff) as u32);
                if d > 0 {
                    format!("{:.1}", n as f64 / d as f64)
                } else {
                    "?".into()
                }
            }
            Err(_) => "?".into(),
        };
        let stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|v| v as i32).unwrap_or(-1);
        format!("{} {}x{} @{} fps, stride {}", sub, w, h, fr, stride)
    }

    /// Save NV12 (tightly packed) as PNG - so the image can really be
    /// LOOKED AT instead of only comparing numbers.
    fn nv12_png(nv12: &[u8], w: u32, h: u32, pfad: &str) -> String {
        let mut rgba = Vec::new();
        if !crate::h264::nv12_to_rgba(nv12, w, h, w as usize, h, &mut rgba) {
            return format!("{}: NV12 zu kurz ({} Bytes)", pfad, nv12.len());
        }
        match image::RgbaImage::from_raw(w, h, rgba) {
            Some(img) => match img.save(pfad) {
                Ok(_) => format!("{} geschrieben ({}x{})", pfad, w, h),
                Err(e) => format!("{}: {}", pfad, e),
            },
            None => format!("{}: Puffer passt nicht", pfad),
        }
    }

    // SAFETY: caller contract: COM is initialised on the current thread until return; all
    // interfaces are created and dropped inside. The out-slots (`z0`, `pitch`, `flags`, `ts`,
    // `sample`) are locals; the 2D buffer is locked only to read its pitch and is unlocked
    // immediately without reading through the pointer. Pixel data is read only via `bild_holen`,
    // and later indexing of `roh` is bounds-checked.
    unsafe fn diagnose_inner(
        id: Option<String>,
        zw: u32,
        zh: u32,
        fps: u32,
        ordner: &str,
    ) -> String {
        let mut log = String::new();
        macro_rules! sag {
            ($($a:tt)*) => {{ let z = format!($($a)*); println!("{}", z); log.push_str(&z); log.push('\n'); }};
        }
        mf_startup();
        let gesucht = id.unwrap_or_default();
        let treffer = mit_kameras(|a, name, gid| {
            if gesucht.is_empty()
                || gid.eq_ignore_ascii_case(&gesucht)
                || name.to_lowercase().contains(&gesucht.to_lowercase())
            {
                match a.ActivateObject::<IMFMediaSource>() {
                    Ok(src) => Some(Ok((src, name.to_string()))),
                    Err(e) => Some(Err(format!("{}: {}", name, e))),
                }
            } else {
                None
            }
        });
        let (quelle, name) = match treffer {
            Some(Ok(t)) => t,
            Some(Err(e)) => return format!("Kamera nicht zu oeffnen: {}", e),
            None => return "keine Kamera gefunden".to_string(),
        };
        sag!("Kamera: {}", name);
        sag!("Wunsch: {}x{} @{} fps NV12", zw, zh, fps);

        let mut attrs: Option<IMFAttributes> = None;
        if MFCreateAttributes(&mut attrs, 2).is_err() {
            return "Attribute".into();
        }
        let attrs = attrs.unwrap();
        let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
        let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
        let leser = match MFCreateSourceReaderFromMediaSource(&quelle, &attrs) {
            Ok(l) => l,
            Err(e) => return format!("Leser: {}", e),
        };
        let strom = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        sag!("--- was die Kamera von sich aus kann ---");
        for i in 0..64u32 {
            match leser.GetNativeMediaType(strom, i) {
                Ok(t) => sag!("  [{:2}] {}", i, typ_text(&t)),
                Err(_) => break,
            }
        }

        // Genau derselbe Ablauf wie im Aufnahmefaden.
        let mut skalieren = false;
        let mt = MFCreateMediaType().unwrap();
        let _ = mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video);
        let _ = mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12);
        let _ = mt.SetUINT64(&MF_MT_FRAME_SIZE, pack(zw, zh));
        if fps > 0 {
            let _ = mt.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1));
        }
        match leser.SetCurrentMediaType(strom, None, &mt) {
            Ok(_) => sag!("Wunschgroesse angenommen (ACHTUNG: sagt nichts ueber die echten Daten)"),
            Err(e) => {
                skalieren = true;
                sag!("Wunschgroesse abgelehnt ({}) -> nur NV12 setzen", e);
                let mt2 = MFCreateMediaType().unwrap();
                let _ = mt2.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video);
                let _ = mt2.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12);
                if let Err(e) = leser.SetCurrentMediaType(strom, None, &mt2) {
                    return format!("{}\nKamera kann kein NV12: {}", log, e);
                }
            }
        }
        // And now the new path: a format that the camera REALLY can do.
        match bestes_format(&leser, strom, zw, zh, fps) {
            Some(t) => {
                sag!("bestes natives NV12: {}", typ_text(&t));
                match leser.SetCurrentMediaType(strom, None, &t) {
                    Ok(_) => {
                        skalieren = true;
                        sag!("natives Format gesetzt");
                    }
                    Err(e) => sag!("natives Format abgelehnt: {}", e),
                }
            }
            None => sag!("kein natives NV12 gefunden"),
        }
        let _ = leser.SetStreamSelection(strom, true);
        if let Ok(cur) = leser.GetCurrentMediaType(strom) {
            sag!("JETZT eingestellt: {}", typ_text(&cur));
        }
        let (qw, qh) = groesse(&leser, strom).unwrap_or((zw, zh));
        if qw != zw || qh != zh {
            skalieren = true;
        }
        sag!("Quelle {}x{}, Ziel {}x{}, skalieren={}", qw, qh, zw, zh, skalieren);

        // Discard a few frames (exposure), then look at one closely.
        let mut roh: Vec<u8> = Vec::new();
        let mut fertig: Vec<u8> = Vec::new();
        let mut gesehen = 0;
        for runde in 0..60 {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample: Option<IMFSample> = None;
            if leser
                .ReadSample(strom, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))
                .is_err()
            {
                sag!("ReadSample fehlgeschlagen");
                break;
            }
            let s = match sample {
                Some(s) => s,
                None => continue,
            };
            gesehen += 1;
            if gesehen < 12 {
                continue;
            }
            // --- Puffer genau vermessen ---
            let buf = match s.ConvertToContiguousBuffer() {
                Ok(b) => b,
                Err(e) => {
                    sag!("kein zusammenhaengender Puffer: {}", e);
                    break;
                }
            };
            let cur_len = buf.GetCurrentLength().unwrap_or(0);
            let max_len = buf.GetMaxLength().unwrap_or(0);
            let dicht = (qw as usize) * (qh as usize) * 3 / 2;
            sag!("--- Puffer (Runde {}) ---", runde);
            sag!("  GetCurrentLength {}  GetMaxLength {}  dicht waere {}", cur_len, max_len, dicht);
            let mut pitch_i = 0i32;
            match buf.cast::<IMF2DBuffer>() {
                Ok(b2) => {
                    let mut z0: *mut u8 = std::ptr::null_mut();
                    let mut pitch = 0i32;
                    if b2.Lock2D(&mut z0, &mut pitch).is_ok() {
                        pitch_i = pitch;
                        sag!("  IMF2DBuffer: pitch {}", pitch);
                        let _ = b2.Unlock2D();
                    } else {
                        sag!("  IMF2DBuffer: Lock2D scheitert");
                    }
                }
                Err(_) => sag!("  kein IMF2DBuffer"),
            }
            sag!(
                "  Weg im Code: {}",
                if pitch_i > 0 && cur_len as usize >= (pitch_i as usize) * (qh as usize) * 3 / 2 {
                    "2D-Pfad (nv12_packen)"
                } else {
                    "flacher Rueckfall (Lock)"
                }
            );
            let (qw, qh) = match bild_holen(&s, qw, qh, &mut roh) {
                Some(g) => g,
                None => {
                    sag!("  bild_holen: FEHLGESCHLAGEN");
                    break;
                }
            };
            sag!("  bild_holen ok, {} Bytes, ECHTE Groesse {}x{}", roh.len(), qw, qh);
            // Metrics: is the colour really around 128?
            let yn = (qw as usize) * (qh as usize);
            let ys: u64 = roh[..yn].iter().map(|v| *v as u64).sum();
            let us: u64 = roh[yn..].iter().step_by(2).map(|v| *v as u64).sum();
            let vs: u64 = roh[yn + 1..].iter().step_by(2).map(|v| *v as u64).sum();
            let n2 = (roh.len() - yn) / 2;
            sag!(
                "  Mittelwerte: Y {:.1}  U {:.1}  V {:.1}  (U/V sollten nahe 128 liegen)",
                ys as f64 / yn as f64,
                us as f64 / n2 as f64,
                vs as f64 / n2 as f64
            );
            sag!("  {}", nv12_png(&roh, qw, qh, &format!("{}\\cam-roh.png", ordner)));
            if (qw != zw || qh != zh)
                && super::nv12_zuschneiden_skalieren(&roh, qw, qh, zw, zh, &mut fertig)
            {
                sag!("  {}", nv12_png(&fertig, zw, zh, &format!("{}\\cam-fertig.png", ordner)));
            }
            break;
        }
        let _ = leser.Flush(strom);
        let _ = quelle.Shutdown();
        log
    }

    // SAFETY: caller contract: `leser` is a live IMFSourceReader and `strom` a stream index. The
    // loop is bounded (64 entries) and only reads attributes (GUID/u64) of the returned media
    // types, which are owned COM objects.
    /// Looks among the formats the camera supports natively for
    /// the best NV12 for the target size.
    ///
    /// Rating, in this order:
    ///   1. same aspect ratio as the target (otherwise it has to be cropped
    ///      - then something of the room is missing left and right),
    ///   2. at least as large as the target (upscaling brings nothing),
    ///   3. as SMALL as possible above that (saves CPU time when scaling down),
    ///   4. as close as possible to the wanted frame rate.
    unsafe fn bestes_format(
        leser: &IMFSourceReader,
        strom: u32,
        zw: u32,
        zh: u32,
        fps: u32,
    ) -> Option<IMFMediaType> {
        let ziel = zw as f64 / zh.max(1) as f64;
        let mut bester: Option<(i64, IMFMediaType)> = None;
        for i in 0..64u32 {
            let t = match leser.GetNativeMediaType(strom, i) {
                Ok(t) => t,
                Err(_) => break,
            };
            if t.GetGUID(&MF_MT_SUBTYPE).ok() != Some(MFVideoFormat_NV12) {
                continue;
            }
            let (w, h) = match t.GetUINT64(&MF_MT_FRAME_SIZE) {
                Ok(v) => ((v >> 32) as u32, (v & 0xffff_ffff) as u32),
                Err(_) => continue,
            };
            if w < 2 || h < 2 {
                continue;
            }
            let bilder = t
                .GetUINT64(&MF_MT_FRAME_RATE)
                .ok()
                .and_then(|v| {
                    let (n, d) = ((v >> 32) as u32, (v & 0xffff_ffff) as u32);
                    if d > 0 {
                        Some(n as f64 / d as f64)
                    } else {
                        None
                    }
                })
                .unwrap_or(30.0);
            // Smaller is better -> negative points for area and distance.
            let verhaeltnis = w as f64 / h as f64;
            let mut punkte: i64 = 0;
            if (verhaeltnis - ziel).abs() < 0.02 {
                punkte += 4_000_000; // gleiches Bildverhaeltnis: kein Zuschnitt
            }
            if w >= zw && h >= zh {
                punkte += 2_000_000; // gross genug, nur verkleinern
            }
            punkte -= (w as i64 * h as i64) / 100; // je kleiner, desto besser
            punkte -= ((bilder - fps.max(1) as f64).abs() * 1000.0) as i64;
            if bester.as_ref().map(|(p, _)| punkte > *p).unwrap_or(true) {
                bester = Some((punkte, t));
            }
        }
        bester.map(|(_, t)| t)
    }

    // SAFETY: caller contract: `leser` is a live IMFSourceReader and `strom` a stream index; it
    // only reads attributes of the owned media type and keeps no pointers.
    unsafe fn groesse(leser: &IMFSourceReader, strom: u32) -> Option<(u32, u32)> {
        let cur = leser.GetCurrentMediaType(strom).ok()?;
        let sz = cur.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let w = (sz >> 32) as u32;
        let h = (sz & 0xffff_ffff) as u32;
        if w < 2 || h < 2 {
            return None;
        }
        Some((w & !1, h & !1))
    }

    // SAFETY: caller contract: `s` is a live IMFSample. Path A: after a successful Lock2D, `zeile0`
    // is valid for the buffer's contiguous length (`len` from GetCurrentLength, or GetMaxLength as
    // fallback) and `pitch > 0` is checked; the slice length `noetig = stride * rh * 3 / 2` is
    // verified to be <= `len` before from_raw_parts, and the buffer is unlocked after the closure
    // ran. Path B: Lock returns the data pointer and `flach` (the current length), and both slices
    // (`dicht` and `noetig`) are only built after checking `<= flach`; Unlock follows on every path
    // after a successful Lock and the pointer is not used afterwards. The data is copied into
    // `out`, so nothing refers to the locked memory after return.
    /// Copies a sample into tightly packed NV12 and returns the size
    /// that REALLY is in it.
    ///
    /// WHY not simply believe the media type: measured on 06.08.2026 on
    /// the "Surface Camera Front", the source reader reports 640x360 NV12
    /// and then delivers a buffer of 1 382 400 bytes with row stride 1280 -
    /// i.e. in truth a 1280x720 image. Whoever believes the type copies the
    /// first 640 bytes of 360 rows (= top left quarter, the image
    /// looks strongly zoomed in) and looks for the colour plane at 1280*360 -
    /// in the middle of the luma. That is exactly where Justin's green/magenta
    /// images came from. Row stride and buffer size do not lie, the type does.
    unsafe fn bild_holen(s: &IMFSample, w: u32, h: u32, out: &mut Vec<u8>) -> Option<(u32, u32)> {
        let (wu, hu) = (w as usize, h as usize);
        let buf = s.ConvertToContiguousBuffer().ok()?;
        let len = buf
            .GetCurrentLength()
            .ok()
            .filter(|v| *v > 0)
            .or_else(|| buf.GetMaxLength().ok())
            .unwrap_or(0) as usize;

        // --- Path A: rows with stride (the usual case) ---
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            let mut zeile0: *mut u8 = std::ptr::null_mut();
            let mut pitch = 0i32;
            if b2.Lock2D(&mut zeile0, &mut pitch).is_ok() {
                let ergebnis = (|| {
                    if pitch <= 0 || zeile0.is_null() {
                        return None;
                    }
                    let stride = pitch as usize;
                    let (rw, rh) = super::echte_groesse(len, stride, w, h)?;
                    let noetig = stride * (rh as usize) * 3 / 2;
                    if len < noetig {
                        return None;
                    }
                    let q = std::slice::from_raw_parts(zeile0, noetig);
                    if super::nv12_packen(q, stride, rw, rh, out) {
                        Some((rw, rh))
                    } else {
                        None
                    }
                })();
                let _ = b2.Unlock2D();
                if let Some(g) = ergebnis {
                    return Some(g);
                }
            }
        }

        // --- Path B: lock flat. Lock() returns tightly packed
        // data for 2D buffers. Here too: if the length does not fit the
        // reported size, the media type lied.
        let mut p: *mut u8 = std::ptr::null_mut();
        let mut flach = 0u32;
        if buf.Lock(&mut p, None, Some(&mut flach)).is_err() || p.is_null() {
            return None;
        }
        let flach = flach as usize;
        let dicht = wu * hu * 3 / 2;
        let ergebnis = if flach >= dicht && flach < dicht * 2 {
            out.clear();
            out.extend_from_slice(std::slice::from_raw_parts(p, dicht));
            Some((w, h))
        } else if flach >= dicht * 2 {
            // Clearly more data than expected: assume the same aspect ratio
            // and compute the real size (n*n*3/2 = flat).
            let punkte = flach * 2 / 3;
            let faktor = (punkte as f64 / (wu * hu) as f64).sqrt();
            let rw = (((wu as f64 * faktor).round() as u32) & !1).max(2);
            let rh = (((hu as f64 * faktor).round() as u32) & !1).max(2);
            let noetig = (rw as usize) * (rh as usize) * 3 / 2;
            if flach >= noetig {
                out.clear();
                out.extend_from_slice(std::slice::from_raw_parts(p, noetig));
                Some((rw, rh))
            } else {
                None
            }
        } else {
            None
        };
        let _ = buf.Unlock();
        ergebnis
    }
}

#[cfg(test)]
mod tests_stufe5 {
    use super::*;

    #[test]
    fn rgb_nach_nv12_haelt_groesse_und_farbe() {
        // Reines Gruen, 40x30 -> 16x16: Ausschnitt mittig, Farbe bleibt.
        let (sb, sh) = (40u32, 30u32);
        let mut rgb = vec![0u8; (sb * sh * 3) as usize];
        for p in rgb.chunks_mut(3) {
            p[1] = 255;
        }
        let mut nv12 = Vec::new();
        assert!(rgb_nach_nv12(&rgb, sb, sh, 16, 16, &mut nv12));
        assert_eq!(nv12.len(), 16 * 16 * 3 / 2);
        // Y of green is around 145 (BT.601).
        let y0 = nv12[0];
        assert!((120..175).contains(&y0), "Y von Gruen ist {}", y0);
    }

    #[test]
    fn rgb_nach_nv12_schneidet_mittig_zu() {
        // A black border left and right, white in the middle: with a 1:1 target
        // the white middle must remain.
        let (sb, sh) = (60u32, 20u32);
        let mut rgb = vec![0u8; (sb * sh * 3) as usize];
        for y in 0..sh as usize {
            for x in 20..40usize {
                let i = (y * sb as usize + x) * 3;
                rgb[i] = 255;
                rgb[i + 1] = 255;
                rgb[i + 2] = 255;
            }
        }
        let mut nv12 = Vec::new();
        assert!(rgb_nach_nv12(&rgb, sb, sh, 8, 8, &mut nv12));
        let mitte = nv12[8 * 4 + 4];
        assert!(mitte > 200, "Mitte sollte hell sein, ist {}", mitte);
    }

    #[test]
    fn zu_kleine_eingaben_werden_abgelehnt() {
        let mut out = Vec::new();
        assert!(!rgb_nach_nv12(&[0u8; 3], 1, 1, 16, 16, &mut out));
        assert!(!rgb_nach_nv12(&[], 0, 0, 16, 16, &mut out));
    }

    /// What does this machine find? On a build box without a camera an
    /// empty list is correct - it must never crash.
    #[test]
    fn kameraliste_und_oeffnen_melden_ehrlich() {
        let l = liste();
        println!("Kameras: {}", l.len());
        for g in &l {
            println!("  {} [{}]", g.name, g.id);
        }
        if l.is_empty() {
            match oeffnen(None, 640, 360, 15) {
                Ok(_) => println!("ohne Kameraliste trotzdem eine bekommen"),
                Err(e) => println!("keine Kamera: {}", e),
            }
            return;
        }
        // There is one: then the whole path to NV12 must run.
        match oeffnen(None, 640, 360, 15) {
            Ok(k) => {
                let start = std::time::Instant::now();
                let mut bild = None;
                while start.elapsed() < std::time::Duration::from_secs(6) && bild.is_none() {
                    bild = k.neuestes();
                    if bild.is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                match bild {
                    Some(b) => {
                        assert_eq!(b.nv12.len(), 640 * 360 * 3 / 2, "NV12-Laenge");
                        let hell: u64 = b.nv12[..640 * 360].iter().map(|v| *v as u64).sum();
                        println!(
                            "Kamera {} liefert {}x{}, Helligkeit {:.1}",
                            k.name,
                            b.breite,
                            b.hoehe,
                            hell as f64 / (640.0 * 360.0)
                        );
                    }
                    None => println!("kein Bild binnen 6 s ({})", k.fehler()),
                }
                k.stoppen();
            }
            Err(e) => println!("Kamera trotz Liste nicht zu oeffnen: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liste_stuerzt_nicht_ab() {
        // On the server without a camera this must give an empty list.
        let l = liste();
        println!("Kameras: {}", l.len());
    }

    #[test]
    fn packen_entfernt_den_zeilenabstand() {
        let (w, h, stride) = (4u32, 4u32, 8usize);
        // Y-Zeilen 0..3, dann UV-Zeilen 0..1 - je Zeile 8 Bytes, davon 4 Muell.
        let mut src = vec![0u8; stride * (h as usize) * 3 / 2];
        for y in 0..6 {
            for x in 0..8 {
                src[y * stride + x] = if x < 4 { (y * 10 + x) as u8 } else { 0xEE };
            }
        }
        let mut out = Vec::new();
        assert!(nv12_packen(&src, stride, w, h, &mut out));
        assert_eq!(out.len(), 4 * 4 * 3 / 2);
        assert!(!out.contains(&0xEE), "Muell aus dem Zeilenabstand kopiert");
        assert_eq!(&out[0..4], &[0, 1, 2, 3]);
        assert_eq!(&out[16..20], &[40, 41, 42, 43]); // erste UV-Zeile
    }

    /// The real measurement from 06.08.2026 (Surface Camera Front): the
    /// media type reported 640x360, the buffer was 1 382 400 bytes at
    /// row stride 1280 - that is 1280x720. Exactly this case made Justin's
    /// image green and zoomed in.
    #[test]
    fn luegender_medientyp_wird_entlarvt() {
        assert_eq!(echte_groesse(1_382_400, 1280, 640, 360), Some((1280, 720)));
    }

    #[test]
    fn ehrlicher_medientyp_bleibt_stehen() {
        // 640x360 mit Zeilenabstand 640: passt genau.
        assert_eq!(echte_groesse(640 * 360 * 3 / 2, 640, 640, 360), Some((640, 360)));
        // With padding bytes (row stride 768) and a suitably large buffer.
        assert_eq!(echte_groesse(768 * 360 * 3 / 2, 768, 640, 360), Some((640, 360)));
    }

    /// Tightly packed buffer with a large row stride: NO guessing
    /// here, otherwise the 2D path cuts the image to pieces. The
    /// flat fallback reads it correctly - so report None.
    #[test]
    fn dicht_gepackter_puffer_geht_an_den_rueckfall() {
        assert_eq!(echte_groesse(640 * 360 * 3 / 2, 768, 640, 360), None);
    }

    #[test]
    fn unsinnige_puffer_geben_nichts_zurueck() {
        assert_eq!(echte_groesse(0, 1280, 640, 360), None);
        assert_eq!(echte_groesse(1000, 0, 640, 360), None);
        assert_eq!(echte_groesse(100, 1280, 640, 360), None);
    }

    /// The colour plane MUST lie behind the whole luma. If it is read too
    /// early, luma arrives as colour - the image turns green.
    #[test]
    fn packen_liest_die_farbe_nicht_aus_der_helligkeit() {
        let (w, h, stride) = (8u32, 6u32, 8usize);
        let mut src = vec![0u8; stride * (h as usize) * 3 / 2];
        // Helligkeit 200, Farbe 128 (= farblos).
        for v in src[..stride * h as usize].iter_mut() {
            *v = 200;
        }
        for v in src[stride * h as usize..].iter_mut() {
            *v = 128;
        }
        let mut out = Vec::new();
        assert!(nv12_packen(&src, stride, w, h, &mut out));
        let yn = (w * h) as usize;
        assert!(out[..yn].iter().all(|v| *v == 200), "Helligkeit verfaelscht");
        assert!(
            out[yn..].iter().all(|v| *v == 128),
            "Farbe kommt aus der Helligkeit - genau der Gruenstich-Fehler"
        );
    }

    #[test]
    fn packen_meldet_zu_kleine_puffer() {
        let src = vec![0u8; 10];
        let mut out = Vec::new();
        assert!(!nv12_packen(&src, 8, 4, 4, &mut out));
    }

    #[test]
    fn skalieren_haelt_die_farbe() {
        // Uniform image: after scaling down the same must come out.
        let (sw, sh) = (640u32, 480u32);
        let mut src = vec![0u8; (sw * sh * 3 / 2) as usize];
        for v in src[..(sw * sh) as usize].iter_mut() {
            *v = 120;
        }
        for v in src[(sw * sh) as usize..].iter_mut() {
            *v = 90;
        }
        let mut out = Vec::new();
        assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 320, 180, &mut out));
        assert_eq!(out.len(), 320 * 180 * 3 / 2);
        assert!(out[..320 * 180].iter().all(|v| *v == 120), "Y verfaelscht");
        assert!(out[320 * 180..].iter().all(|v| *v == 90), "UV verfaelscht");
    }

    #[test]
    fn skalieren_schneidet_mittig_zu() {
        // 640x480 (4:3) -> 320x180 (16:9): top/bottom must be dropped,
        // the middle must remain.
        let (sw, sh) = (640u32, 480u32);
        let mut src = vec![128u8; (sw * sh * 3 / 2) as usize];
        // mark the top and bottom 60 rows - exactly those that the
        // 4:3 -> 16:9 crop must remove.
        for y in 0..60 {
            for x in 0..sw as usize {
                src[y * sw as usize + x] = 0;
                src[(sh as usize - 1 - y) * sw as usize + x] = 255;
            }
        }
        let mut out = Vec::new();
        assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 320, 180, &mut out));
        // Zielverhaeltnis 16:9 -> Ausschnitt 640x360, also 60 Zeilen oben weg.
        assert!(
            out[..320 * 180].iter().all(|v| *v > 0 && *v < 255),
            "Rand nicht abgeschnitten"
        );
    }

    #[test]
    fn skalieren_merkt_unsinn() {
        let src = vec![0u8; 100];
        let mut out = Vec::new();
        assert!(!nv12_zuschneiden_skalieren(&src, 640, 480, 320, 180, &mut out));
        assert!(!nv12_zuschneiden_skalieren(&src, 0, 0, 320, 180, &mut out));
    }

    #[test]
    fn oeffnen_ohne_kamera_meldet_sauber() {
        // Without a camera (server) an error must come, not a crash.
        match oeffnen(None, 640, 360, 15) {
            Ok(k) => {
                println!("Kamera da: {}", k.name);
                k.stoppen();
            }
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
}

#[cfg(test)]
mod tempo {
    use super::*;

    /// How expensive is scaling down really? The camera delivers 1280x720,
    /// 640x360 is needed - at 30 frames per second there are 33 ms
    /// available for it, and that for EVERYTHING (scaling down, encoding,
    /// drawing). If scaling down alone is already too slow, a
    /// higher send rate is of no use.
    #[test]
    fn verkleinern_ist_schnell_genug() {
        let (sw, sh) = (1280u32, 720u32);
        let mut src = vec![0u8; (sw * sh * 3 / 2) as usize];
        for (i, v) in src.iter_mut().enumerate() {
            *v = (i % 251) as u8;
        }
        let mut out = Vec::new();
        // Einmal warmlaufen (Speicher anlegen).
        assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 640, 360, &mut out));
        // Take the FASTEST of three measurement series. The machine is building
        // on the side, so one series sometimes takes twice as long - and
        // a test that fails by chance is worse than none.
        // The fastest series is still an honest lower bound: a
        // real regression (the error found was at 46 ms) stays
        // slow in EVERY series.
        let runden = 20;
        let mut je = f64::MAX;
        for _ in 0..3 {
            let start = std::time::Instant::now();
            for _ in 0..runden {
                assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 640, 360, &mut out));
            }
            je = je.min(start.elapsed().as_secs_f64() * 1000.0 / runden as f64);
        }
        println!("1280x720 -> 640x360: {:.2} ms je Bild (schnellste von 3)", je);
        // 30 frames per second means 33 ms for EVERYTHING - scaling down,
        // encoding, sending, drawing. The shipped build is the
        // release build, so the strict limit applies there. The debug build
        // is many times slower and runs on a server that is building
        // on the side - there only the gross derailment is caught
        // (the error this test found was at 46 ms).
        let grenze = if cfg!(debug_assertions) { 30.0 } else { 3.0 };
        assert!(
            je < grenze,
            "Verkleinern braucht {:.1} ms (Grenze {:.0} ms) - zu langsam",
            je,
            grenze
        );
    }

    /// The fast path must deliver THE SAME as the general one. Otherwise
    /// the speed-up would be a silent image error.
    #[test]
    fn schneller_weg_rechnet_wie_der_allgemeine() {
        let (sw, sh) = (64u32, 36u32);
        let mut src = vec![0u8; (sw * sh * 3 / 2) as usize];
        for (i, v) in src.iter_mut().enumerate() {
            *v = ((i * 37) % 256) as u8;
        }
        // 64x36 -> 32x18 is exactly halved (fast path),
        // 64x36 -> 30x18 is not (general path) - both must work.
        let mut halb = Vec::new();
        assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 32, 18, &mut halb));
        assert_eq!(halb.len(), 32 * 18 * 3 / 2);
        // Cross-check by hand: first point = mean of the four top left.
        let erwartet = (src[0] as u32 + src[1] as u32 + src[64] as u32 + src[65] as u32) / 4;
        assert_eq!(halb[0] as u32, erwartet, "schneller Weg mittelt falsch");
        // Erste Farbe ebenso.
        let uv = (sw * sh) as usize;
        let eu = (src[uv] as u32 + src[uv + 2] as u32 + src[uv + 64] as u32 + src[uv + 66] as u32) / 4;
        assert_eq!(halb[32 * 18] as u32, eu, "schneller Weg mittelt die Farbe falsch");
        let mut krumm = Vec::new();
        assert!(nv12_zuschneiden_skalieren(&src, sw, sh, 30, 18, &mut krumm));
        assert_eq!(krumm.len(), 30 * 18 * 3 / 2);
    }
}
