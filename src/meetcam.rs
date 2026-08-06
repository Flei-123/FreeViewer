//! Natives Meeting - Stufe 2d: die ECHTE Kamera.
//!
//! Bisher ging ein Testmuster ins Meeting (Stufe 2b). Hier kommt das Bild
//! jetzt von der Kamera - unter Windows ueber Media Foundation, also
//! genau die Maschine, die der Browser auch benutzt. Der Aufnahmefaden
//! haelt immer nur das NEUESTE Bild bereit; wer langsamer abholt,
//! ueberspringt Bilder, statt einen Rueckstau zu bauen.
//!
//! Geliefert wird dicht gepacktes NV12 in genau der gewuenschten Groesse.
//! Kann die Kamera das Format/die Groesse nicht, wird hier zugeschnitten
//! und verkleinert - so bekommt der H.264-Kodierer immer das, was er
//! erwartet, egal welche Kamera steckt.
//!
//! macOS folgt in Stufe 5 (AVFoundation), Linux hat hier nichts zu tun.

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

/// Ein Bild in dicht gepacktem NV12 (Y-Ebene, dann UV verschraenkt).
#[derive(Clone)]
pub struct Bild {
    pub breite: u32,
    pub hoehe: u32,
    pub nv12: Vec<u8>,
}

/// NV12 mit Zeilenabstand in dicht gepacktes NV12 kopieren.
///
/// Media Foundation liefert Zeilen oft breiter als das Bild (stride), weil
/// die Grafikkarte das so mag. Der Kodierer will es dicht gepackt.
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

/// Aus Puffergroesse und Zeilenabstand die ECHTE Bildgroesse ableiten.
///
/// Media Foundation darf im Medientyp eine Groesse melden und dann eine
/// andere liefern - gemessen am 06.08.2026: angemeldet 640x360 NV12,
/// geliefert ein Puffer mit 1 382 400 Bytes und Zeilenabstand 1280, also in
/// Wahrheit 1280x720. Wer der Meldung glaubt, kopiert das linke obere
/// Viertel (Bild wirkt hineingezoomt) und liest die Farbebene mitten aus der
/// Helligkeit (Bild wird gruen/magenta). Zeilenabstand und Puffergroesse
/// luegen nicht - also rechnen wir die Wahrheit daraus aus.
///
/// Rueckgabe: die Groesse, mit der der Puffer gelesen werden DARF.
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
        // Weniger Daten als gemeldet: der Puffer liegt nicht mit diesem
        // Zeilenabstand da (meist dicht gepackt). Hier NICHT raten - der
        // flache Rueckfall liest ihn richtig.
        return None;
    }
    if zeilen as usize == hu {
        return Some((w, h)); // Meldung und Puffer passen zusammen
    }
    // MEHR Zeilen als gemeldet -> der Medientyp hat gelogen. Dann ist der
    // Zeilenabstand zugleich die echte Breite (Kameras liefern NV12 ohne
    // Fuellbytes) und die Zeilenzahl die echte Hoehe.
    Some(((stride as u32) & !1, zeilen))
}

/// Mittigen Ausschnitt im Zielverhaeltnis nehmen und auf die Zielgroesse
/// verkleinern (Flaechenmittel, damit es nicht flimmert).
///
/// Warum zuschneiden statt quetschen: ein 16:9-Bild in ein 4:3-Fenster
/// gequetscht macht lange Gesichter. Zoom/Teams schneiden ebenfalls zu.
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
    // Ausschnitt im Zielverhaeltnis, immer gerade Kanten (NV12 braucht das).
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
    // --- UV (halbe Aufloesung, U und V abwechselnd) ---
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

/// Laeuft im Hintergrund und haelt das neueste Kamerabild bereit.
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
    /// Das neueste Bild abholen (und aus dem Puffer nehmen). Kommt None,
    /// gibt es seit dem letzten Abholen kein neues.
    pub fn neuestes(&self) -> Option<Bild> {
        self.neu.lock().ok().and_then(|mut b| b.take())
    }

    /// Wie viele Bilder hat die Kamera bisher geliefert.
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
        win::liste()
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

/// Kamera oeffnen. `id` leer = erste Kamera. Geliefert wird immer genau
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
        std::thread::Builder::new()
            .name("kamera".into())
            .spawn(move || win::schleife(id, breite, hoehe, fps, tx, n2, z2, s2, f2))
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

/// RGB (3 Bytes je Punkt) auf `zb` x `zh` bringen und nach NV12 wandeln.
///
/// Kameras liefern selten genau die gewuenschte Groesse. Statt zu strecken
/// wird der groesstmoegliche MITTIGE Ausschnitt im Zielverhaeltnis genommen
/// und dann per Flaechenmittel verkleinert - so bleibt das Gesicht rund und
/// der Text scharf.
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


/// Kamera-Diagnose (nur Windows). Schreibt cam-roh.png / cam-fertig.png
/// nach `ordner` und liefert den Messbericht als Text.
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

/// Kamera auf dem Mac. AVFoundation ist Objective-C - statt selbst eine
/// Delegate-Klasse zu bauen, uebernimmt das nokhwa (nur fuer diese
/// Plattform eingebunden). Der Rest ist derselbe Ablauf wie unter Windows:
/// eigener Faden, immer nur das NEUESTE Bild, dicht gepacktes NV12.
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
        // Welche Kamera? Ohne Angabe die erste.
        let welche = match id.as_deref().and_then(|s| s.trim().parse::<u32>().ok()) {
            Some(n) => CameraIndex::Index(n),
            None => CameraIndex::Index(0),
        };
        // Naechstbeste Einstellung zu Wunschgroesse und -bildrate. NV12 ist
        // das, was Kameras am haeufigsten koennen; nokhwa rechnet notfalls um.
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
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
        });
    }

    fn pack(a: u32, b: u32) -> u64 {
        ((a as u64) << 32) | b as u64
    }

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

    /// Kameras auflisten. Ruft `f` fuer jede auf; liefert `f`s erstes
    /// Some-Ergebnis (so laesst sich in einem Rutsch suchen ODER listen).
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
        unsafe {
            // Nie Some liefern -> laeuft durch alle Geraete.
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

    /// Der Aufnahmefaden. Lebt so lange, bis `stop` gesetzt wird.
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
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let r = schleife_inner(id, zw, zh, fps, &tx, &neu, &zaehler, &stop, &fehler);
            if let Err(e) = r {
                // Wenn der Start schon scheiterte, hat der Aufrufer noch
                // nichts bekommen - die Meldung muss zu ihm.
                let _ = tx.send(Err(e.clone()));
                if let Ok(mut f) = fehler.lock() {
                    *f = e;
                }
            }
            CoUninitialize();
        }
    }

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

        // Der Leser darf umrechnen (Farbformat, Groesse) - sonst muessten
        // wir MJPEG selbst dekodieren.
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2).map_err(|e| format!("Attribute: {}", e))?;
        let attrs = attrs.ok_or_else(|| "Attribute leer".to_string())?;
        let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
        let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
        let leser = MFCreateSourceReaderFromMediaSource(&quelle, &attrs)
            .map_err(|e| format!("Leser: {}", e))?;
        let strom = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        // ZUERST ein Format nehmen, das die Kamera WIRKLICH kann.
        //
        // Frueher wurde einfach die Wunschgroesse gesetzt. Der Leser sagt
        // dazu "ja" - und liefert trotzdem das native Bild (gemessen:
        // 640x360 angemeldet, 1280x720 geliefert). Das Ergebnis war ein
        // zerschnittenes, gruenes Bild. Ein natives Format kann nicht
        // luegen; verkleinert wird danach mit unserem eigenen, geprueften
        // Rechenweg.
        let mut skalieren = true;
        let mut gesetzt = false;
        if let Some(t) = bestes_format(&leser, strom, zw, zh, fps) {
            if leser.SetCurrentMediaType(strom, None, &t).is_ok() {
                gesetzt = true;
            }
        }
        if !gesetzt {
            // Rueckfall: nur das Farbformat festlegen, Groesse offen lassen.
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

        // Was liefert die Kamera nun wirklich?
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
                // Der Medientyp hat gelogen - ab jetzt mit der echten
                // Groesse weiterrechnen, sonst schneidet der Skalierer in
                // einen falschen Ausschnitt.
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


    /// Kamera-Diagnose: was bietet das Geraet an, was bekommen wir wirklich,
    /// und wie sieht das Rohbild aus.
    ///
    /// WARUM: Justins Bild kam gruen/magenta und stark hineingezoomt an. Ob
    /// das am Farbraum, am Zeilenabstand oder am Zuschnitt liegt, laesst
    /// sich nur am ECHTEN Geraet messen - raten hilft hier nicht.
    pub fn diagnose(id: Option<String>, zw: u32, zh: u32, fps: u32, ordner: &str) -> String {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let r = diagnose_inner(id, zw, zh, fps, ordner);
            CoUninitialize();
            r
        }
    }

    fn fourcc(g: &GUID) -> String {
        // MF-Videountertypen sind FOURCCs in einer festen GUID-Huelle:
        // XXXXXXXX-0000-0010-8000-00AA00389B71 - die ersten vier Bytes
        // ergeben direkt den Namen (NV12, YUY2, MJPG ...).
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

    /// NV12 (dicht gepackt) als PNG ablegen - damit laesst sich das Bild
    /// wirklich ANSEHEN statt nur Zahlen zu vergleichen.
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
        // Und jetzt der neue Weg: ein Format, das die Kamera WIRKLICH kann.
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

        // Ein paar Bilder verwerfen (Belichtung), dann eins genau ansehen.
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
            // Kennzahlen: liegt die Farbe wirklich um 128?
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

    /// Sucht unter den Formaten, die die Kamera von sich aus beherrscht,
    /// das beste NV12 fuer die Zielgroesse.
    ///
    /// Bewertung, in dieser Reihenfolge:
    ///   1. gleiches Seitenverhaeltnis wie das Ziel (sonst muss zugeschnitten
    ///      werden - dann fehlt links und rechts etwas vom Raum),
    ///   2. mindestens so gross wie das Ziel (hochrechnen bringt nichts),
    ///   3. moeglichst KLEIN darueber (spart Rechenzeit beim Verkleinern),
    ///   4. moeglichst nahe an der Wunschbildrate.
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
            // Kleiner ist besser -> negative Punkte fuer Flaeche und Abstand.
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

    /// Kopiert ein Sample in dicht gepacktes NV12 und liefert die Groesse,
    /// die WIRKLICH darin steckt.
    ///
    /// WARUM nicht einfach dem Medientyp glauben: gemessen am 06.08.2026 auf
    /// der "Surface Camera Front" meldet der Quell-Leser 640x360 NV12 und
    /// liefert dann einen Puffer von 1 382 400 Bytes mit Zeilenabstand 1280 -
    /// also in Wahrheit ein 1280x720-Bild. Wer dem Typ glaubt, kopiert die
    /// ersten 640 Bytes von 360 Zeilen (= linkes oberes Viertel, das Bild
    /// wirkt stark hineingezoomt) und sucht die Farbebene bei 1280*360 -
    /// mitten in der Helligkeit. Genau daher kamen Justins gruen/magenta
    /// Bilder. Zeilenabstand und Puffergroesse luegen nicht, der Typ schon.
    unsafe fn bild_holen(s: &IMFSample, w: u32, h: u32, out: &mut Vec<u8>) -> Option<(u32, u32)> {
        let (wu, hu) = (w as usize, h as usize);
        let buf = s.ConvertToContiguousBuffer().ok()?;
        let len = buf
            .GetCurrentLength()
            .ok()
            .filter(|v| *v > 0)
            .or_else(|| buf.GetMaxLength().ok())
            .unwrap_or(0) as usize;

        // --- Weg A: Zeilen mit Abstand (der uebliche Fall) ---
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

        // --- Weg B: flach sperren. Lock() liefert bei 2D-Puffern dicht
        // gepackte Daten. Auch hier gilt: passt die Laenge nicht zur
        // gemeldeten Groesse, hat der Medientyp gelogen.
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
            // Deutlich mehr Daten als erwartet: gleiches Seitenverhaeltnis
            // annehmen und die echte Groesse ausrechnen (n*n*3/2 = flach).
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
        // Y von Gruen liegt bei etwa 145 (BT.601).
        let y0 = nv12[0];
        assert!((120..175).contains(&y0), "Y von Gruen ist {}", y0);
    }

    #[test]
    fn rgb_nach_nv12_schneidet_mittig_zu() {
        // Links und rechts ein schwarzer Rand, Mitte weiss: bei 1:1-Ziel
        // muss die weisse Mitte uebrig bleiben.
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

    /// Was findet diese Maschine? Auf einem Bauknecht ohne Kamera ist eine
    /// leere Liste richtig - abstuerzen darf es nie.
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
        // Es gibt eine: dann muss der ganze Weg bis NV12 laufen.
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
        // Auf dem Server ohne Kamera muss das eine leere Liste geben.
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

    /// Der echte Messwert vom 06.08.2026 (Surface Camera Front): der
    /// Medientyp meldete 640x360, der Puffer war 1 382 400 Bytes gross bei
    /// Zeilenabstand 1280 - das ist 1280x720. Genau dieser Fall hat Justins
    /// Bild gruen und hineingezoomt gemacht.
    #[test]
    fn luegender_medientyp_wird_entlarvt() {
        assert_eq!(echte_groesse(1_382_400, 1280, 640, 360), Some((1280, 720)));
    }

    #[test]
    fn ehrlicher_medientyp_bleibt_stehen() {
        // 640x360 mit Zeilenabstand 640: passt genau.
        assert_eq!(echte_groesse(640 * 360 * 3 / 2, 640, 640, 360), Some((640, 360)));
        // Mit Fuellbytes (Zeilenabstand 768) und passend grossem Puffer.
        assert_eq!(echte_groesse(768 * 360 * 3 / 2, 768, 640, 360), Some((640, 360)));
    }

    /// Dicht gepackter Puffer bei grossem Zeilenabstand: hier darf NICHT
    /// geraten werden, sonst schneidet der 2D-Weg das Bild kaputt. Der
    /// flache Rueckfall liest ihn richtig - also None melden.
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

    /// Die Farbebene MUSS hinter der ganzen Helligkeit liegen. Wird sie zu
    /// frueh gelesen, kommt Helligkeit als Farbe an - das Bild wird gruen.
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
        // Gleichmaessiges Bild: nach dem Verkleinern muss dasselbe rauskommen.
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
        // 640x480 (4:3) -> 320x180 (16:9): oben/unten muss wegfallen,
        // die Mitte muss uebrig bleiben.
        let (sw, sh) = (640u32, 480u32);
        let mut src = vec![128u8; (sw * sh * 3 / 2) as usize];
        // obere und untere 60 Zeilen markieren - genau die, die der
        // Zuschnitt 4:3 -> 16:9 wegnehmen muss.
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
        // Ohne Kamera (Server) muss ein Fehler kommen, kein Absturz.
        match oeffnen(None, 640, 360, 15) {
            Ok(k) => {
                println!("Kamera da: {}", k.name);
                k.stoppen();
            }
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
}
