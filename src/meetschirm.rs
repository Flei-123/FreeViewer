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

/// Der Bereich, aus dem gesendet wird - als Anteil des Bildschirms.
/// (0,0,1,1) = der ganze Schirm.
pub type Bereich = (f32, f32, f32, f32);

pub const GANZ: Bereich = (0.0, 0.0, 1.0, 1.0);

/// Aus den Wuenschen ALLER Zuschauer einen Bereich machen, der gesendet wird.
///
/// WARUM ueberhaupt: zoomt ein Zuschauer hinein, bekommt er sonst nur
/// hochgerechnete Bildpunkte. Schneidet der SENDER stattdessen genau diesen
/// Ausschnitt aus seiner nativen Aufnahme und schickt ihn in der GLEICHEN
/// Kodiergroesse, kostet das keine zusaetzliche Bandbreite - das Bild wird
/// aber wirklich schaerfer, weil echte Bildpunkte uebertragen werden.
///
/// WARUM eine Vereinigung: es koennen mehrere zuschauen. Wuerde man nur
/// einem folgen, saehen die anderen den falschen Ausschnitt. Die Huelle um
/// alle Wuensche ist der einzige Bereich, mit dem JEDER richtig liegt.
///
/// Das Ergebnis ist im Anteilsraum immer QUADRATISCH (also im echten Bild
/// seitenverhaeltnisgleich) - sonst waere das gesendete Bild verzerrt - und
/// bekommt etwas Rand, damit kleine Mausbewegungen nicht dauernd ein neues
/// Zuschneiden ausloesen.
pub fn bereich_vereinen(wuensche: &[Bereich]) -> Bereich {
    let echte: Vec<&Bereich> = wuensche
        .iter()
        .filter(|(_, _, w, h)| *w > 0.001 && *h > 0.001)
        .collect();
    if echte.is_empty() {
        return GANZ;
    }
    let mut x0 = 1.0f32;
    let mut y0 = 1.0f32;
    let mut x1 = 0.0f32;
    let mut y1 = 0.0f32;
    for (x, y, w, h) in echte {
        x0 = x0.min(x.clamp(0.0, 1.0));
        y0 = y0.min(y.clamp(0.0, 1.0));
        x1 = x1.max((x + w).clamp(0.0, 1.0));
        y1 = y1.max((y + h).clamp(0.0, 1.0));
    }
    // Rand dazu: 12 % der Kantenlaenge auf jeder Seite.
    let mut b = (x1 - x0).max(0.02);
    let mut h = (y1 - y0).max(0.02);
    let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    b *= 1.24;
    h *= 1.24;
    // Gleiche Kantenlaenge im Anteilsraum = gleiches Seitenverhaeltnis wie
    // der Bildschirm. Alles andere waere ein verzerrtes Bild.
    let k = b.max(h).min(1.0);
    // Lohnt sich nicht: fast der ganze Schirm -> lieber ohne Zuschnitt, dann
    // greift der schnelle Weg ueber die Grafikkarte.
    if k > 0.92 {
        return GANZ;
    }
    let x = (mx - k * 0.5).clamp(0.0, 1.0 - k);
    let y = (my - k * 0.5).clamp(0.0, 1.0 - k);
    (x, y, k, k)
}

/// Lohnt ein Wechsel des gesendeten Bereichs? Winzige Verschiebungen sind es
/// nicht wert - jeder Wechsel kostet ein Schluesselbild.
pub fn bereich_lohnt_wechsel(alt: Bereich, neu: Bereich) -> bool {
    let d = (alt.0 - neu.0).abs().max((alt.1 - neu.1).abs());
    let s = (alt.2 - neu.2).abs();
    // 4 % Verschiebung oder 6 % Groessenaenderung.
    d > 0.04 || s > 0.06
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
    bild_nach_nv12_teil(px, sb, sh, bgra, GANZ, zb, zh, out)
}

/// Wie `bild_nach_nv12`, aber nur ein AUSSCHNITT der Aufnahme (Anteile
/// 0..1). Damit wird beim Hineinzoomen nicht der ganze Bildschirm
/// verkleinert, sondern der interessante Teil in voller Schaerfe gesendet -
/// bei gleicher Kodiergroesse und damit gleicher Bandbreite.
#[allow(clippy::too_many_arguments)]
pub fn bild_nach_nv12_teil(
    px: &[u8],
    sb: u32,
    sh: u32,
    bgra: bool,
    teil: Bereich,
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
    // Ausschnitt in Bildpunkte umrechnen. Alles bleibt im Bild - ein
    // Ausschnitt, der hinausragt, wuerde am Rand Muell zeigen.
    let (ax, ay, aw, ah) = teil;
    let ox = ((ax.clamp(0.0, 1.0) * sbu as f32) as usize).min(sbu.saturating_sub(2));
    let oy = ((ay.clamp(0.0, 1.0) * shu as f32) as usize).min(shu.saturating_sub(2));
    let aw = ((aw.clamp(0.0, 1.0) * sbu as f32) as usize).max(2).min(sbu - ox);
    let ah = ((ah.clamp(0.0, 1.0) * shu as f32) as usize).max(2).min(shu - oy);

    // Zwischenschritt RGB, weil rgb_to_nv12 genau das erwartet.
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

/// Wo steht der Mauszeiger AUF diesem Bildschirm - als Anteil 0..1?
///
/// WARUM als Anteil und nicht in Bildpunkten: der Zuschauer bekommt ein
/// verkleinertes Bild und weiss nichts von der echten Aufloesung. Ein
/// Anteil passt immer, egal wie stark verkleinert wurde.
///
/// None = der Zeiger ist gerade auf einem ANDEREN Bildschirm; dann waere
/// jede Zahl gelogen.
pub fn zeiger_anteil(index: usize) -> Option<(f32, f32)> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let m = crate::capture::list_monitors(true).into_iter().nth(index)?;
        if m.w < 2 || m.h < 2 {
            return None;
        }
        let mut p = POINT::default();
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
    /// Welcher Teil des Bildschirms gesendet wird (Anteile). Der
    /// Aufnahmefaden liest das bei JEDEM Bild - so wirkt ein Wechsel sofort,
    /// ohne die Aufnahme neu zu oeffnen.
    bereich: Arc<Mutex<Bereich>>,
}

impl Aufnahme {
    /// Nur diesen Ausschnitt senden (Anteile 0..1). GANZ = wieder alles.
    pub fn bereich_setzen(&self, b: Bereich) {
        if let Ok(mut g) = self.bereich.lock() {
            *g = b;
        }
    }
    pub fn bereich(&self) -> Bereich {
        self.bereich.lock().map(|g| *g).unwrap_or(GANZ)
    }

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
    let bereich = Arc::new(Mutex::new(GANZ));
    let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(String, u32, u32), String>>();
    let (n2, z2, s2, f2) = (neu.clone(), zaehler.clone(), stop.clone(), fehler.clone());
    let b2 = bereich.clone();
    std::thread::Builder::new()
        .name("meetschirm".into())
        .spawn(move || schleife(index, max_b, max_h, fps, tx, n2, z2, s2, f2, b2))
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
            bereich,
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
    bereich: Arc<Mutex<Bereich>>,
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
        let teil = bereich.lock().map(|g| *g).unwrap_or(GANZ);
        let fertig = if teil == GANZ {
            // Schneller Weg: die Grafikkarte skaliert und wandelt selbst.
            match backend.scaled(zb, zh, true) {
                Some(daten) if daten.len() >= (zb as usize * zh as usize * 3 / 2) => {
                    nv12.clear();
                    nv12.extend_from_slice(&daten[..(zb as usize * zh as usize * 3 / 2)]);
                    true
                }
                _ => {
                    let (px, sb2, sh2, bgra) = backend.frame();
                    bild_nach_nv12(px, sb2, sh2, bgra, zb, zh, &mut nv12)
                }
            }
        } else {
            // Mit Ausschnitt kann die Grafikkarte nicht helfen - dafuer sind
            // es WENIGER Quellpunkte als beim ganzen Schirm, der Zuschnitt
            // kostet also nicht mehr, sondern weniger Rechenzeit.
            let (px, sb2, sh2, bgra) = backend.frame();
            bild_nach_nv12_teil(px, sb2, sh2, bgra, teil, zb, zh, &mut nv12)
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
    #[test]
    fn ohne_wunsch_geht_der_ganze_schirm_raus() {
        assert_eq!(bereich_vereinen(&[]), GANZ);
        assert_eq!(bereich_vereinen(&[(0.0, 0.0, 1.0, 1.0)]), GANZ);
        // Fast alles ist auch alles - sonst faellt der schnelle Weg ueber die
        // Grafikkarte fuer nichts weg.
        assert_eq!(bereich_vereinen(&[(0.02, 0.02, 0.9, 0.9)]), GANZ);
    }

    #[test]
    fn ein_zuschauer_bekommt_seinen_ausschnitt_mit_rand() {
        // Mitte, ein Viertel der Kante.
        let (x, y, w, h) = bereich_vereinen(&[(0.375, 0.375, 0.25, 0.25)]);
        assert!((w - h).abs() < 1e-6, "muss quadratisch bleiben");
        assert!(w > 0.25 && w < 0.4, "Rand fehlt oder ist zu gross: {}", w);
        // immer noch mittig
        assert!((x + w / 2.0 - 0.5).abs() < 0.01);
        assert!((y + h / 2.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn zwei_zuschauer_bekommen_die_huelle_um_beide() {
        let a = (0.05, 0.05, 0.1, 0.1);
        let b = (0.55, 0.55, 0.1, 0.1);
        let (x, y, w, h) = bereich_vereinen(&[a, b]);
        assert!((w - h).abs() < 1e-6);
        // Beide Wuensche muessen wirklich drin liegen - sonst saehe einer
        // von beiden den falschen Ausschnitt.
        assert!(x <= a.0 + 1e-6 && y <= a.1 + 1e-6, "erster faellt heraus");
        assert!(
            x + w >= b.0 + b.2 - 1e-6 && y + h >= b.1 + b.3 - 1e-6,
            "zweiter faellt heraus"
        );
    }

    #[test]
    fn bereich_bleibt_immer_im_bild() {
        for wunsch in [
            (0.0, 0.0, 0.2, 0.2),
            (0.9, 0.9, 0.2, 0.2),
            (-0.5, 0.8, 0.3, 0.3),
        ] {
            let (x, y, w, h) = bereich_vereinen(&[wunsch]);
            assert!(x >= -1e-6 && y >= -1e-6, "links/oben raus: {:?}", (x, y));
            assert!(x + w <= 1.0 + 1e-6 && y + h <= 1.0 + 1e-6, "rechts/unten raus");
        }
    }

    #[test]
    fn kleine_zuckungen_loesen_keinen_wechsel_aus() {
        let a = (0.30, 0.30, 0.30, 0.30);
        assert!(!bereich_lohnt_wechsel(a, (0.31, 0.31, 0.30, 0.30)));
        assert!(bereich_lohnt_wechsel(a, (0.40, 0.30, 0.30, 0.30)));
        assert!(bereich_lohnt_wechsel(a, (0.30, 0.30, 0.20, 0.20)));
    }

    /// Der Kern der Sache: aus einem Ausschnitt kommt WIRKLICH nur dieser
    /// Ausschnitt - und in voller Zielgroesse, also mit echten Bildpunkten.
    #[test]
    fn ausschnitt_sendet_nur_den_ausschnitt() {
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
        // Nur die RECHTE Haelfte senden -> alles muss hell sein.
        assert!(bild_nach_nv12_teil(&px, sb, sh, true, (0.5, 0.0, 0.5, 0.5), 16, 16, &mut nv12));
        let y = &nv12[..16 * 16];
        assert!(
            y.iter().all(|v| *v > 200),
            "der Ausschnitt zeigt nicht die rechte Haelfte"
        );
        // Und die LINKE Haelfte -> alles dunkel.
        assert!(bild_nach_nv12_teil(&px, sb, sh, true, (0.0, 0.0, 0.5, 0.5), 16, 16, &mut nv12));
        let y = &nv12[..16 * 16];
        assert!(y.iter().all(|v| *v < 60), "linker Ausschnitt ist nicht dunkel");
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

    /// Stufe 5b: die GANZE Kette auf dieser Maschine - Bildschirm aufnehmen,
    /// verkleinern, nach NV12, durch den H.264-Kodierer. Laeuft auf Windows
    /// und Mac echt; wo es keinen Bildschirm gibt (Server), wird ehrlich
    /// uebersprungen statt falsch gruen zu melden.
    #[test]
    fn bildschirm_bis_h264_durch() {
        let schirme = liste();
        if schirme.is_empty() {
            println!("kein Bildschirm auf dieser Maschine - uebersprungen");
            return;
        }
        println!("Bildschirm 0: {:?}", schirme[0]);
        let auf = match oeffnen(0, 1280, 720, 10) {
            Ok(a) => a,
            Err(e) => {
                println!("Aufnahme nicht moeglich ({}) - uebersprungen", e);
                return;
            }
        };
        let (zb, zh) = (auf.breite, auf.hoehe);
        assert!(zb % 2 == 0 && zh % 2 == 0, "ungerade Kante {}x{}", zb, zh);
        assert!(zb <= 1280 && zh <= 720, "zu gross: {}x{}", zb, zh);
        let start = std::time::Instant::now();
        let mut bild = None;
        while start.elapsed() < std::time::Duration::from_secs(5) && bild.is_none() {
            bild = auf.neuestes();
            if bild.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        let Some(b) = bild else {
            println!(
                "kein Bild binnen 5 s (Fehler: {}) - uebersprungen",
                auf.fehler()
            );
            return;
        };
        assert_eq!(b.breite, zb);
        assert_eq!(b.hoehe, zh);
        assert_eq!(
            b.nv12.len(),
            zb as usize * zh as usize * 3 / 2,
            "NV12-Laenge passt nicht"
        );
        // Wie hell ist das Bild wirklich? Bei gesperrtem Rechner ist es
        // schwarz - das ist kein Fehler, aber es soll dastehen.
        let ysum: u64 = b.nv12[..(zb as usize * zh as usize)]
            .iter()
            .map(|v| *v as u64)
            .sum();
        let hell = ysum as f64 / (zb as f64 * zh as f64);
        println!(
            "aufgenommen {}x{}, {} Bilder, mittlere Helligkeit {:.1}",
            zb,
            zh,
            auf.aufgenommen(),
            hell
        );
        // Und jetzt durch den Kodierer - das ist der Teil, der auf dem Mac
        // neu ist.
        if !crate::h264::available() {
            println!("kein H.264-Kodierer - Kette hier nicht pruefbar");
            return;
        }
        let mut enc = crate::h264::Encoder::new(zb, zh, 10, 3_000_000).expect("Kodierer");
        let mut bytes = 0usize;
        let mut schluessel = 0;
        // Media Foundation gibt den ersten Rahmen NICHT sofort heraus (die
        // Kette laeuft erst an) - VideoToolbox schon. Deshalb ein paar Bilder
        // nachschieben, statt aus einem einzigen Bild eine Aussage zu machen.
        for _ in 0..15 {
            for c in enc.encode(&b.nv12).expect("kodieren") {
                bytes += c.data.len();
                if c.key {
                    schluessel += 1;
                }
            }
            if bytes > 0 && schluessel > 0 {
                break;
            }
        }
        println!("H.264: {} Bytes, {} Schluesselbilder", bytes, schluessel);
        assert!(bytes > 0, "Bildschirmbild liess sich nicht kodieren");
        assert!(schluessel > 0, "kein Schluesselbild am Anfang");
        auf.stoppen();
    }

    #[test]
    fn liste_stuerzt_nicht_ab() {
        // Auf einem Server ohne Bildschirm muss das eine Liste (ggf. leer)
        // geben und nicht knallen.
        let l = liste();
        println!("Bildschirme: {}", l.len());
    }
}
