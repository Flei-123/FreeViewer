//! Natives Meeting - Stufe 3: den BILDSCHIRM teilen, ohne Browser.
//!
//! Das Bild kommt aus derselben Aufnahme, die auch die Fernwartung
//! benutzt (`capture.rs`: DXGI Desktop Duplication, sonst GDI/Screenshot).
//! Es wird auf eine sinnvolle Groesse gebracht, nach NV12 gewandelt und
//! dann genau wie die Kamera durch den H.264-Kodierer ins Meeting
//! geschickt - nur auf einer EIGENEN Spur, damit die Gegenseite Kamera
//! und Bildschirm auseinanderhalten kann (der Server bekommt dafuer
//! `publish {screen:true}`).
//!
//! Warum eine eigene Spur und nicht die Kamera umschalten: sonst
//! verschwindet man selbst aus der Runde, sobald man etwas zeigt - genau
//! das hat Justin an anderen Werkzeugen gestoert.
//!
//! Bildschirm ist nicht Kamera: mehr Punkte, weniger Bewegung. Deshalb
//! groessere Zielaufloesung und mehr Bitrate als bei der Kamera, dafuer
//! wird bei einem UNVERAENDERTEN Bild nur alle paar Hundert Millisekunden
//! ein Auffrischer geschickt (damit ein spaeter Dazugekommener nicht vor
//! einer schwarzen Flaeche sitzt), statt stur 30-mal je Sekunde dasselbe.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use crate::meetcam::Bild;

/// Ein teilbarer Bildschirm.
#[derive(Clone, Debug, PartialEq)]
pub struct Schirm {
    pub name: String,
    pub breite: u32,
    pub hoehe: u32,
    pub primaer: bool,
}

/// Alle Bildschirme dieses Rechners, der Hauptbildschirm zuerst.
pub fn liste() -> Vec<Schirm> {
    crate::capture::list_monitors(true)
        .into_iter()
        .map(|m| Schirm {
            name: m.name,
            breite: m.w,
            hoehe: m.h,
            primaer: m.primary,
        })
        .collect()
}

/// Zielgroesse: Seitenverhaeltnis behalten, in `max_b` x `max_h` passen,
/// gerade Kantenlaengen (NV12 braucht das).
pub fn zielgroesse(sb: u32, sh: u32, max_b: u32, max_h: u32) -> (u32, u32) {
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

/// Ein aufgenommenes Bild (BGRA oder RGBA) auf `zb` x `zh` verkleinern und
/// nach NV12 wandeln. Flaechenmittel statt Naechster-Nachbar - sonst
/// flimmert Text beim Verkleinern unlesbar.
pub fn bild_nach_nv12(
    px: &[u8],
    sb: u32,
    sh: u32,
    bgra: bool,
    zb: u32,
    zh: u32,
    out: &mut Vec<u8>,
) -> bool {
    let (sbu, shu, zbu, zhu) = (sb as usize, sh as usize, zb as usize, zh as usize);
    // Unter 2x2 ist nichts zu holen - lieber ehrlich ablehnen, als aus
    // einem einzigen Punkt ein Bild zu erfinden.
    if sbu < 2 || shu < 2 || zbu < 2 || zhu < 2 || px.len() < sbu * shu * 4 {
        return false;
    }
    // Zwischenschritt RGB, weil rgb_to_nv12 genau das erwartet.
    let mut rgb = vec![0u8; zbu * zhu * 3];
    let (ri, gi, bi) = if bgra { (2usize, 1usize, 0usize) } else { (0usize, 1usize, 2usize) };
    for ty in 0..zhu {
        let y0 = ty * shu / zhu;
        let y1 = ((ty + 1) * shu / zhu).max(y0 + 1).min(shu);
        for tx in 0..zbu {
            let x0 = tx * sbu / zbu;
            let x1 = ((tx + 1) * sbu / zbu).max(x0 + 1).min(sbu);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let zeile = y * sbu * 4;
                for x in x0..x1 {
                    let i = zeile + x * 4;
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

/// Laufende Bildschirmaufnahme. Haelt immer nur das NEUESTE Bild bereit.
pub struct Aufnahme {
    pub name: String,
    pub breite: u32,
    pub hoehe: u32,
    /// Welcher Bildschirm (Index aus `liste()`).
    pub index: usize,
    neu: Arc<Mutex<Option<Bild>>>,
    zaehler: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fehler: Arc<Mutex<String>>,
}

impl Aufnahme {
    /// Das neueste Bild abholen (und aus dem Puffer nehmen).
    pub fn neuestes(&self) -> Option<Bild> {
        self.neu.lock().ok().and_then(|mut b| b.take())
    }
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

impl Drop for Aufnahme {
    fn drop(&mut self) {
        self.stoppen();
    }
}

/// Bildschirm `index` oeffnen und im Hintergrund aufnehmen.
///
/// `max_b`/`max_h` begrenzen die Uebertragungsgroesse (Seitenverhaeltnis
/// bleibt erhalten), `fps` ist die Obergrenze der Bildrate.
pub fn oeffnen(index: usize, max_b: u32, max_h: u32, fps: u32) -> Result<Aufnahme> {
    let fps = fps.clamp(1, 60);
    let neu: Arc<Mutex<Option<Bild>>> = Arc::new(Mutex::new(None));
    let zaehler = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let fehler = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(String, u32, u32), String>>();
    let (n2, z2, s2, f2) = (neu.clone(), zaehler.clone(), stop.clone(), fehler.clone());
    std::thread::Builder::new()
        .name("meetschirm".into())
        .spawn(move || schleife(index, max_b, max_h, fps, tx, n2, z2, s2, f2))
        .map_err(|e| anyhow!("Bildschirmfaden: {}", e))?;
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok((name, b, h))) => Ok(Aufnahme {
            name,
            breite: b,
            hoehe: h,
            index,
            neu,
            zaehler,
            stop,
            fehler,
        }),
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err(anyhow!("Bildschirmaufnahme antwortet nicht"))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn schleife(
    index: usize,
    max_b: u32,
    max_h: u32,
    fps: u32,
    tx: std::sync::mpsc::Sender<std::result::Result<(String, u32, u32), String>>,
    neu: Arc<Mutex<Option<Bild>>>,
    zaehler: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    fehler: Arc<Mutex<String>>,
) {
    let mut backend = match crate::capture::open_index(true, index) {
        Some(b) => b,
        None => {
            let _ = tx.send(Err("kein Bildschirm aufnehmbar".to_string()));
            return;
        }
    };
    let (sb, sh) = backend.size();
    let (zb, zh) = zielgroesse(sb, sh, max_b, max_h);
    let anzeige = crate::capture::list_monitors(true)
        .get(index)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| format!("Bildschirm {}", index + 1));
    let name = format!("{} ({}x{} -> {}x{}, {})", anzeige, sb, sh, zb, zh, backend.name());
    if tx.send(Ok((name, zb, zh))).is_err() {
        return;
    }

    let takt = (1000 / fps).max(8);
    let mut nv12: Vec<u8> = Vec::new();
    // Auffrischer: auch ohne Aenderung alle 400 ms ein Bild, sonst sieht
    // ein spaet Dazugekommener nichts.
    let mut letzte_lieferung = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut verluste = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let was = backend.next(takt);
        let liefern = match was {
            crate::capture::Next::Frame => true,
            crate::capture::Next::Unchanged => {
                letzte_lieferung.elapsed() >= std::time::Duration::from_millis(400)
            }
            crate::capture::Next::Lost => {
                verluste += 1;
                if let Ok(mut f) = fehler.lock() {
                    *f = format!("Aufnahme abgerissen ({}. Mal) - neu geoeffnet", verluste);
                }
                match crate::capture::open_index(true, index) {
                    Some(b) => {
                        backend = b;
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        continue;
                    }
                    None => {
                        if let Ok(mut f) = fehler.lock() {
                            *f = "Bildschirm nicht mehr aufnehmbar".into();
                        }
                        return;
                    }
                }
            }
        };
        if !liefern {
            continue;
        }
        // Schneller Weg: die Grafikkarte skaliert und wandelt selbst.
        let fertig = match backend.scaled(zb, zh, true) {
            Some(daten) if daten.len() >= (zb as usize * zh as usize * 3 / 2) => {
                nv12.clear();
                nv12.extend_from_slice(&daten[..(zb as usize * zh as usize * 3 / 2)]);
                true
            }
            _ => {
                let (px, sb2, sh2, bgra) = backend.frame();
                bild_nach_nv12(px, sb2, sh2, bgra, zb, zh, &mut nv12)
            }
        };
        if !fertig {
            continue;
        }
        letzte_lieferung = std::time::Instant::now();
        zaehler.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut b) = neu.lock() {
            *b = Some(Bild {
                breite: zb,
                hoehe: zh,
                nv12: nv12.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zielgroesse_behaelt_das_verhaeltnis_und_wird_gerade() {
        let (b, h) = zielgroesse(3840, 2160, 1920, 1080);
        assert_eq!((b, h), (1920, 1080));
        let (b, h) = zielgroesse(1366, 768, 1920, 1080);
        assert_eq!((b, h), (1366, 768), "kleiner Schirm wird nicht hochskaliert");
        let (b, h) = zielgroesse(2560, 1080, 1920, 1080);
        assert!(b <= 1920 && h <= 1080, "{}x{}", b, h);
        assert!(b % 2 == 0 && h % 2 == 0, "ungerade Kante {}x{}", b, h);
        // 21:9 bleibt 21:9 (auf 1 % genau)
        let v = b as f64 / h as f64;
        assert!((v - 2560.0 / 1080.0).abs() < 0.03, "Verhaeltnis {}", v);
    }

    #[test]
    fn bild_nach_nv12_hat_die_richtige_groesse_und_farbe() {
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
        assert!(bild_nach_nv12(&px, sb, sh, true, 32, 16, &mut nv12));
        assert_eq!(nv12.len(), 32 * 16 * 3 / 2);
        // Y von Rot liegt bei etwa 82 (BT.601) - deutlich weg von 0 und 255.
        let y0 = nv12[0];
        assert!((60..110).contains(&y0), "Y von Rot ist {}", y0);
    }

    #[test]
    fn bild_nach_nv12_mittelt_beim_verkleinern() {
        // Links schwarz, rechts weiss: nach dem Verkleinern muss links
        // dunkel und rechts hell bleiben (kein Versatz, kein Einheitsgrau).
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
        assert!(bild_nach_nv12(&px, sb, sh, false, 16, 8, &mut nv12));
        let links = nv12[0];
        let rechts = nv12[15];
        assert!(links < 40, "links sollte dunkel sein: {}", links);
        assert!(rechts > 200, "rechts sollte hell sein: {}", rechts);
    }

    #[test]
    fn zu_kleine_eingaben_werden_abgelehnt_statt_zu_stuerzen() {
        let mut out = Vec::new();
        assert!(!bild_nach_nv12(&[0u8; 4], 1, 1, true, 32, 16, &mut out));
        assert!(!bild_nach_nv12(&[], 0, 0, false, 32, 16, &mut out));
    }

    #[test]
    fn liste_stuerzt_nicht_ab() {
        // Auf einem Server ohne Bildschirm muss das eine Liste (ggf. leer)
        // geben und nicht knallen.
        let l = liste();
        println!("Bildschirme: {}", l.len());
    }
}
