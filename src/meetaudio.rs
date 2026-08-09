//! Natives Meeting - Stufe 1c: Mikrofon, Lautsprecher, Mischer und
//! Echo-Unterdrueckung.
//!
//! Der Browser hat uns das bisher abgenommen. Drei Dinge fehlen also:
//!   1. Aufnahme und Wiedergabe (cpal - liegt schon im Projekt).
//!   2. MISCHEN: mehrere Gegenueber kommen als getrennte Tonspuren an,
//!      der Lautsprecher will EINE.
//!   3. ECHO: was aus dem Lautsprecher kommt, hoert das Mikrofon wieder.
//!      Ohne Gegenmassnahme hoert der andere sich selbst - der haeufigste
//!      Grund, warum Selbstgebautes "billig" klingt.
//!
//! Die Echo-Unterdrueckung ist bewusst REINES RUST (Blockweise im
//! Frequenzbereich, NLMS + Nachfilter). Der uebliche Weg waere eine
//! C++-Bibliothek (WebRTC-APM oder Speex); beide brauchen cmake, libclang
//! und Extra-Werkzeuge auf JEDEM Baurechner - das wollen wir uns fuer
//! Windows und Mac nicht einhandeln. Wie gut es wirklich ist, wird
//! gemessen (ERLE in dB, siehe Tests unten), nicht behauptet.

use std::collections::HashMap;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// 48 kHz, 20-ms-Rahmen - wie im Browser.
pub const RATE: usize = 48_000;
pub const RAHMEN: usize = 960;

/// Blocklaenge der Echo-Unterdrueckung (10 ms) und Laenge der Nachhallfahne.
const BLOCK: usize = 480;
const FFT_N: usize = 2 * BLOCK;
/// Wie weit zurueck wird das Echo gesucht (Anzahl Bloecke a 10 ms).
const TEILE: usize = 20; // 200 ms - deckt Zimmerhall und Puffer ab

/// Blockweise Echo-Unterdrueckung im Frequenzbereich (partitioniertes NLMS).
///
/// Ablauf je 10-ms-Block: das, was gerade zum Lautsprecher geht (Referenz),
/// wandert in einen Verlauf. Aus dem Verlauf wird das erwartete Echo
/// geschaetzt und vom Mikrofonsignal abgezogen. Der Filter lernt dabei
/// weiter - aber NUR, wenn nicht gerade beide gleichzeitig reden
/// (Gegensprech-Erkennung), sonst zerstoert man die eigene Stimme.
pub struct Echo {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    /// Gewichte des Filters, je Teil ein Spektrum.
    w: Vec<Vec<Complex32>>,
    /// Verlauf der Referenzspektren (Ringpuffer).
    x: Vec<Vec<Complex32>>,
    pos: usize,
    /// Letzter Referenzblock (fuer die Ueberlappung).
    letzte_ref: Vec<f32>,
    /// Mittlere Leistung der Referenz je Frequenz - Schrittweite des NLMS.
    leistung: Vec<f32>,
    /// Wie stark der Nachfilter zuletzt gedaempft hat (zum Weichzeichnen).
    daempfung: Vec<f32>,
    /// Spitzenwerte der letzten Referenzbloecke - fuer die
    /// Gegensprech-Erkennung nach Geigel (braucht KEINEN fertig gelernten
    /// Filter, das war der Fehler in der ersten Fassung).
    ref_spitzen: std::collections::VecDeque<f32>,
    /// Zaehler fuer die reihum laufende Zwangsbedingung.
    runde: usize,
}

impl Echo {
    pub fn neu() -> Echo {
        let mut planer = FftPlanner::<f32>::new();
        Echo {
            fft: planer.plan_fft_forward(FFT_N),
            ifft: planer.plan_fft_inverse(FFT_N),
            w: vec![vec![Complex32::new(0.0, 0.0); FFT_N]; TEILE],
            x: vec![vec![Complex32::new(0.0, 0.0); FFT_N]; TEILE],
            pos: 0,
            letzte_ref: vec![0.0; BLOCK],
            leistung: vec![1e-6; FFT_N],
            daempfung: vec![1.0; FFT_N],
            ref_spitzen: std::collections::VecDeque::new(),
            runde: 0,
        }
    }

    /// Einen 10-ms-Block bearbeiten.
    /// `mikro` = Aufnahme, `referenz` = was zeitgleich zum Lautsprecher ging.
    /// Rueckgabe: Mikrofon ohne Echo.
    pub fn block(&mut self, mikro: &[f32], referenz: &[f32]) -> Vec<f32> {
        assert_eq!(mikro.len(), BLOCK);
        assert_eq!(referenz.len(), BLOCK);

        // --- Referenz in den Verlauf (mit 50 % Ueberlappung) ---------------
        let mut ein: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for i in 0..BLOCK {
            ein.push(Complex32::new(self.letzte_ref[i], 0.0));
        }
        for i in 0..BLOCK {
            ein.push(Complex32::new(referenz[i], 0.0));
        }
        self.letzte_ref.copy_from_slice(referenz);
        self.fft.process(&mut ein);
        self.pos = (self.pos + 1) % TEILE;
        self.x[self.pos] = ein;

        // --- geschaetztes Echo = Summe ueber alle Teile --------------------
        let mut y = vec![Complex32::new(0.0, 0.0); FFT_N];
        for t in 0..TEILE {
            let xi = (self.pos + TEILE - t) % TEILE;
            for k in 0..FFT_N {
                y[k] += self.x[xi][k] * self.w[t][k];
            }
        }
        self.ifft.process(&mut y);
        let skal = 1.0 / FFT_N as f32;
        // gueltig ist nur die zweite Haelfte (Overlap-Save)
        let echo: Vec<f32> = (BLOCK..FFT_N).map(|i| y[i].re * skal).collect();

        // --- Fehler = Mikrofon minus geschaetztes Echo ---------------------
        let fehler: Vec<f32> = (0..BLOCK).map(|i| mikro[i] - echo[i]).collect();

        // --- Gegensprech-Erkennung (Geigel) --------------------------------
        // Der Filter ist am Anfang leer, also kann man NICHT mit dem
        // geschaetzten Echo vergleichen (genau daran ist die erste Fassung
        // gescheitert: sie hat nie gelernt). Stattdessen der bewaehrte
        // Vergleich: ist das Mikrofon lauter als ein gedaempfter
        // Lautsprecher je sein kann, redet ein Mensch mit.
        let spitze_mik = mikro.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let spitze_ref = referenz.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        self.ref_spitzen.push_back(spitze_ref);
        while self.ref_spitzen.len() > TEILE {
            self.ref_spitzen.pop_front();
        }
        let ref_max = self.ref_spitzen.iter().fold(0.0f32, |m, v| m.max(*v));
        let e_ref: f32 = referenz.iter().map(|v| v * v).sum::<f32>() + 1e-9;
        let gegensprechen = spitze_mik > 0.7 * ref_max;
        let lernen = e_ref > 1e-6 && ref_max > 1e-4 && !gegensprechen;

        if lernen {
            // Fehlerspektrum (vorne Nullen, hinten der Fehler - Overlap-Save)
            let mut ef: Vec<Complex32> = Vec::with_capacity(FFT_N);
            for _ in 0..BLOCK {
                ef.push(Complex32::new(0.0, 0.0));
            }
            for i in 0..BLOCK {
                ef.push(Complex32::new(fehler[i], 0.0));
            }
            self.fft.process(&mut ef);

            // Leistung je Frequenz ueber alle Teile - daraus die Schrittweite.
            for k in 0..FFT_N {
                let mut p = 0.0;
                for t in 0..TEILE {
                    let xi = (self.pos + TEILE - t) % TEILE;
                    p += self.x[xi][k].norm_sqr();
                }
                self.leistung[k] = 0.8 * self.leistung[k] + 0.2 * p;
            }
            // Ein reiner Dauerton belegt nur wenige Frequenzen - dort wird
            // die Schrittweite sonst riesig und der Filter fliegt weg.
            // Deshalb ein Boden, der sich am Mittel aller Frequenzen
            // orientiert (Tichonow-Regelung).
            let mittel: f32 = self.leistung.iter().sum::<f32>() / FFT_N as f32;
            let boden = (mittel * 0.05).max(1e-3);
            for k in 0..FFT_N {
                if self.leistung[k] < boden {
                    self.leistung[k] = boden;
                }
            }
            // Schrittweite klein halten: lieber langsam und stabil als
            // schnell und dann wegfliegend (erste Fassung: -57 dB, also das
            // Gegenteil von Echo-Unterdrueckung).
            let mu = 0.1;
            for t in 0..TEILE {
                let xi = (self.pos + TEILE - t) % TEILE;
                for k in 0..FFT_N {
                    let g = mu / self.leistung[k];
                    self.w[t][k] += self.x[xi][k].conj() * ef[k] * g;
                }
            }
            // Zwangsbedingung: ein Teil des Filters darf nur die ersten
            // BLOCK Abtastwerte belegen. Ohne das faltet sich das Signal um
            // (zirkulaere Faltung) und der Filter wird instabil. Aus
            // Rechenzeitgruenden reihum immer ein Teil pro Block.
            self.runde = self.runde.wrapping_add(1);
            let skal2 = 1.0 / FFT_N as f32;
            for t in 0..TEILE {
                let mut zeit = self.w[t].clone();
                self.ifft.process(&mut zeit);
                for (i, v) in zeit.iter_mut().enumerate() {
                    if i < BLOCK {
                        *v = Complex32::new(v.re * skal2, 0.0);
                    } else {
                        *v = Complex32::new(0.0, 0.0);
                    }
                }
                self.fft.process(&mut zeit);
                self.w[t] = zeit;
            }
            // Leckstrom: alte Lernergebnisse verblassen langsam, damit sich
            // der Filter nicht in ein altes Zimmer verbeisst.
            for t in 0..TEILE {
                for k in 0..FFT_N {
                    self.w[t][k] *= 0.9999;
                }
            }
        }

        // --- Notbremse ------------------------------------------------------
        // Wenn die Schaetzung lauter wird als das Mikrofon, ist der Filter
        // weggelaufen (bei reinen Dauertoenen der Klassiker). Dann lieber
        // alles halbieren als dem Gegenueber die Ohren wegblasen.
        let e_mik: f32 = mikro.iter().map(|v| v * v).sum::<f32>() + 1e-12;
        let e_echo: f32 = echo.iter().map(|v| v * v).sum::<f32>();
        if e_echo > 4.0 * e_mik {
            for t in 0..TEILE {
                for k in 0..FFT_N {
                    self.w[t][k] *= 0.5;
                }
            }
        }

        // --- Nachfilter: was uebrig bleibt, wird frequenzweise gedaempft ---
        // Der lineare Filter erwischt nie alles (Zimmer aendern sich,
        // Lautsprecher verzerren). Rest per Daempfung nach Verhaeltnis
        // Fehler zu geschaetztem Echo.
        let mut fspek: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for _ in 0..BLOCK {
            fspek.push(Complex32::new(0.0, 0.0));
        }
        for i in 0..BLOCK {
            fspek.push(Complex32::new(fehler[i], 0.0));
        }
        let mut espek: Vec<Complex32> = Vec::with_capacity(FFT_N);
        for _ in 0..BLOCK {
            espek.push(Complex32::new(0.0, 0.0));
        }
        for i in 0..BLOCK {
            espek.push(Complex32::new(echo[i], 0.0));
        }
        self.fft.process(&mut fspek);
        self.fft.process(&mut espek);
        for k in 0..FFT_N {
            let f = fspek[k].norm_sqr();
            let e = espek[k].norm_sqr();
            let ziel = (f / (f + 2.0 * e + 1e-9)).clamp(0.05, 1.0);
            // weich nachziehen, sonst "pumpt" es hoerbar
            self.daempfung[k] = 0.7 * self.daempfung[k] + 0.3 * ziel;
            fspek[k] *= self.daempfung[k];
        }
        self.ifft.process(&mut fspek);
        (BLOCK..FFT_N).map(|i| fspek[i].re * skal).collect()
    }

    /// Bequemer Weg fuer 20-ms-Rahmen (zwei Bloecke).
    pub fn rahmen(&mut self, mikro: &[f32], referenz: &[f32]) -> Vec<f32> {
        let mut aus = Vec::with_capacity(mikro.len());
        let mut i = 0;
        while i + BLOCK <= mikro.len() && i + BLOCK <= referenz.len() {
            aus.extend(self.block(&mikro[i..i + BLOCK], &referenz[i..i + BLOCK]));
            i += BLOCK;
        }
        aus
    }
}

/// Mischt die Tonrahmen mehrerer Teilnehmer zu einem Lautsprechersignal.
///
/// Jeder Teilnehmer bekommt eine eigene Warteschlange - Pakete kommen nie
/// gleichmaessig an. Der Mischer nimmt, was da ist, und fuellt fehlende
/// Stellen mit Stille auf, statt zu knacksen.
/// Rauschsperre fuers Mikrofon ("noise gate").
///
/// WARUM ueberhaupt: bisher ging JEDES Geraeusch raus - Luefter, Tastatur,
/// die Strasse. In einer Runde mit acht Leuten summiert sich das zu einem
/// Rauschteppich, durch den niemand mehr angenehm reden kann.
///
/// Das Grundrauschen wird SELBST geschaetzt, nicht vom Nutzer erfragt:
/// jeder Raum ist anders, und niemand kann in Dezibel angeben, wie laut
/// sein Kuehlschrank ist. Die Schaetzung faellt schnell (leiser Raum wird
/// sofort erkannt) und steigt sehr langsam - sonst wuerde eine lange
/// Wortmeldung nach und nach als "Grundrauschen" durchgehen und die Sperre
/// wuerde sich selbst zudrehen.
///
/// Zwei Dinge, die den Unterschied zwischen "brauchbar" und "unbenutzbar"
/// machen:
/// * **Nachlaufzeit.** Nach dem letzten lauten Rahmen bleibt das Tor noch
///   rund 300 ms offen. Ohne das schneidet die Sperre die leisen Ausklaenge
///   von Woertern ab, und man klingt abgehackt.
/// * **Weiche Rampen.** Auf- und Zublenden ueber ein paar Millisekunden.
///   Ein hartes Umschalten erzeugt ein hoerbares Knacken.
#[derive(Debug, Clone)]
pub struct Rauschsperre {
    /// Geschaetztes Grundrauschen als Effektivwert (0..1).
    grund: f32,
    /// Aktuelle Durchlassstaerke (0 = zu, 1 = offen).
    verstaerkung: f32,
    /// Wie viele Rahmen das Tor noch offen bleibt.
    nachlauf: u32,
    /// Die ersten Rahmen dienen nur dem Einmessen.
    einmessen: u32,
    /// Stand fuer die Anzeige.
    offen: bool,
}

impl Default for Rauschsperre {
    fn default() -> Self {
        Self::neu()
    }
}

impl Rauschsperre {
    /// So viele 20-ms-Rahmen bleibt das Tor nach dem letzten Ton offen.
    const NACHLAUF: u32 = 15; // 300 ms
    /// So lange (Rahmen) wird nur gemessen und alles durchgelassen.
    const EINMESSEN: u32 = 25; // 500 ms

    pub fn neu() -> Rauschsperre {
        Rauschsperre {
            grund: 0.0,
            verstaerkung: 1.0,
            nachlauf: 0,
            einmessen: Self::EINMESSEN,
            offen: true,
        }
    }

    /// Einen Rahmen durchlassen, daempfen oder sperren - an Ort und Stelle.
    ///
    /// `staerke` 0..100: 0 laesst fast alles durch, 100 ist streng.
    /// Rueckgabe: ob gerade durchgelassen wird (fuer die Anzeige).
    pub fn rahmen(&mut self, pcm: &mut [f32], staerke: u8) -> bool {
        if pcm.is_empty() {
            return self.offen;
        }
        let quadrat: f32 = pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len() as f32;
        let pegel = quadrat.sqrt();

        // Grundrauschen nachfuehren: schnell nach unten, sehr langsam nach
        // oben. Beim ersten Rahmen einfach uebernehmen, sonst braeuchte die
        // Schaetzung Sekunden, um von 0 hochzukommen.
        if self.einmessen == Self::EINMESSEN {
            self.grund = pegel;
        } else if pegel < self.grund {
            self.grund = self.grund * 0.85 + pegel * 0.15;
        } else {
            self.grund = self.grund * 0.9995 + pegel * 0.0005;
        }
        if self.einmessen > 0 {
            self.einmessen -= 1;
            self.offen = true;
            return true;
        }

        let s = (staerke.min(100) as f32) / 100.0;
        // Abstand zum Grundrauschen: das Doppelte bis das Achtfache.
        let faktor = 2.0 + s * 6.0;
        // Absolute Untergrenze - in einem voellig stillen Raum ginge der
        // Faktor sonst gegen null und die Sperre waere wirkungslos.
        let mindest = 0.0015 + s * 0.010;
        let schwelle = (self.grund * faktor).max(mindest);

        if pegel > schwelle {
            self.nachlauf = Self::NACHLAUF;
        } else if self.nachlauf > 0 {
            self.nachlauf -= 1;
        }
        let ziel = if self.nachlauf > 0 { 1.0 } else { 0.0 };
        self.offen = ziel > 0.5;

        // Weiche Rampe je Abtastwert: aufblenden schneller als zublenden.
        let schritt = if ziel > self.verstaerkung {
            1.0 / (0.008 * 48000.0) // ~8 ms
        } else {
            1.0 / (0.040 * 48000.0) // ~40 ms
        };
        for v in pcm.iter_mut() {
            if (self.verstaerkung - ziel).abs() > f32::EPSILON {
                if ziel > self.verstaerkung {
                    self.verstaerkung = (self.verstaerkung + schritt).min(ziel);
                } else {
                    self.verstaerkung = (self.verstaerkung - schritt).max(ziel);
                }
            }
            *v *= self.verstaerkung;
        }
        self.offen
    }

    /// Geschaetztes Grundrauschen (0..1) - fuer die Anzeige.
    pub fn grundrauschen(&self) -> f32 {
        self.grund
    }

    /// Laesst die Sperre gerade durch?
    pub fn durchlass(&self) -> bool {
        self.offen
    }
}

#[derive(Default)]
pub struct Mischer {
    puffer: HashMap<u64, std::collections::VecDeque<i16>>,
    /// Wie viele Rahmen mindestens liegen bleiben sollen (Ruckelschutz).
    pub vorlauf: usize,
}

impl Mischer {
    pub fn neu() -> Mischer {
        Mischer {
            puffer: HashMap::new(),
            vorlauf: 1,
        }
    }

    pub fn dazu(&mut self, peer: u64, pcm: &[i16]) {
        let p = self.puffer.entry(peer).or_default();
        p.extend(pcm.iter().copied());
        // Nie mehr als 400 ms aufstauen - sonst laeuft das Gespraech davon.
        while p.len() > RATE / 1000 * 400 {
            p.pop_front();
        }
    }

    /// Einen Rahmen fuer den Lautsprecher holen.
    pub fn abmischen(&mut self, n: usize) -> Vec<i16> {
        let mut aus = vec![0i32; n];
        for p in self.puffer.values_mut() {
            if p.len() < n * self.vorlauf.max(1) {
                continue;
            }
            for a in aus.iter_mut() {
                if let Some(v) = p.pop_front() {
                    *a += v as i32;
                }
            }
        }
        aus.iter()
            .map(|v| (*v).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
            .collect()
    }

    pub fn leute(&self) -> usize {
        self.puffer.len()
    }

    pub fn entfernen(&mut self, peer: u64) {
        self.puffer.remove(&peer);
    }
}

/// i16 -> f32 (-1..1) und zurueck.
pub fn zu_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|v| *v as f32 / 32768.0).collect()
}
pub fn zu_i16(pcm: &[f32]) -> Vec<i16> {
    pcm.iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect()
}

/// Kanaele zu Mono zusammenlegen (Mikrofone liefern oft Stereo).
pub fn zu_mono(daten: &[f32], kanaele: usize) -> Vec<f32> {
    if kanaele <= 1 {
        return daten.to_vec();
    }
    daten
        .chunks(kanaele)
        .map(|c| c.iter().sum::<f32>() / kanaele as f32)
        .collect()
}

/// Einfaches lineares Umtasten auf 48 kHz (Geraete liefern auch 44,1 kHz).
pub fn auf_48k(daten: &[f32], von: usize) -> Vec<f32> {
    if von == RATE || daten.is_empty() {
        return daten.to_vec();
    }
    let n = (daten.len() as f64 * RATE as f64 / von as f64).round() as usize;
    let mut aus = Vec::with_capacity(n);
    for i in 0..n {
        let pos = i as f64 * von as f64 / RATE as f64;
        let a = pos.floor() as usize;
        let b = (a + 1).min(daten.len() - 1);
        let t = (pos - a as f64) as f32;
        aus.push(daten[a] * (1.0 - t) + daten[b] * t);
    }
    aus
}


// ---------------------------------------------------------------- Geraete
// Mikrofon und Lautsprecher. cpal-Stroeme duerfen den Faden nicht
// verlassen (sie sind nicht "Send"), deshalb laeuft alles in EINEM eigenen
// Faden, der ueber Kanaele mit dem Rest redet.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct Geraete {
    /// Fertige 20-ms-Rahmen vom Mikrofon - schon ohne Echo.
    pub mikro: std::sync::mpsc::Receiver<Vec<i16>>,
    /// Was der Lautsprecher spielen soll (wird gemischt).
    pub lautsprecher: Arc<Mutex<Mischer>>,
    pub stumm: Arc<AtomicBool>,
    /// Rauschsperre an? Wird im laufenden Betrieb umgeschaltet.
    pub sperre_an: Arc<AtomicBool>,
    /// Strenge der Rauschsperre, 0..100.
    pub sperre_staerke: Arc<std::sync::atomic::AtomicU32>,
    /// Nur Anzeige: laesst die Sperre gerade durch?
    pub sperre_offen: Arc<AtomicBool>,
    /// Nur Anzeige: geschaetztes Grundrauschen * 10000.
    pub sperre_grund: Arc<std::sync::atomic::AtomicU32>,
    ende: Arc<AtomicBool>,
    pub eingang: String,
    pub ausgang: String,
}

impl Drop for Geraete {
    fn drop(&mut self) {
        self.ende.store(true, Ordering::Relaxed);
    }
}

/// Startet Aufnahme und Wiedergabe. Namen leer = Standardgeraet.
pub fn geraete_starten(ein_name: Option<String>, aus_name: Option<String>) -> anyhow::Result<Geraete> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let (mik_tx, mik_rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let mischer = Arc::new(Mutex::new(Mischer::neu()));
    let stumm = Arc::new(AtomicBool::new(false));
    let ende = Arc::new(AtomicBool::new(false));
    // Standardmaessig AN: ein Rauschteppich faellt jedem auf, eine leicht
    // zu streng eingestellte Sperre nur dem Sprecher selbst - und der kann
    // sie in den Einstellungen sofort abschalten.
    let sperre_an = Arc::new(AtomicBool::new(true));
    let sperre_staerke = Arc::new(std::sync::atomic::AtomicU32::new(45));
    let sperre_offen = Arc::new(AtomicBool::new(true));
    let sperre_grund = Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Namen vorab bestimmen, damit die Oberflaeche sie anzeigen kann.
    let host = cpal::default_host();
    let ein_dev = match &ein_name {
        Some(n) if !n.is_empty() => host
            .input_devices()?
            .find(|d| d.name().map(|x| x == *n).unwrap_or(false))
            .or_else(|| host.default_input_device()),
        _ => host.default_input_device(),
    }
    .ok_or_else(|| anyhow::anyhow!("kein Mikrofon gefunden"))?;
    let aus_dev = match &aus_name {
        Some(n) if !n.is_empty() => host
            .output_devices()?
            .find(|d| d.name().map(|x| x == *n).unwrap_or(false))
            .or_else(|| host.default_output_device()),
        _ => host.default_output_device(),
    }
    .ok_or_else(|| anyhow::anyhow!("kein Lautsprecher gefunden"))?;
    let ein_name_echt = ein_dev.name().unwrap_or_default();
    let aus_name_echt = aus_dev.name().unwrap_or_default();

    let m2 = mischer.clone();
    let s2 = stumm.clone();
    let e2 = ende.clone();
    let sp = Sperrgriffe {
        an: sperre_an.clone(),
        staerke: sperre_staerke.clone(),
        offen: sperre_offen.clone(),
        grund: sperre_grund.clone(),
    };
    std::thread::Builder::new()
        .name("meetaudio".into())
        .spawn(move || {
            if let Err(e) = geraete_faden(ein_dev, aus_dev, mik_tx, m2, s2, e2, sp) {
                eprintln!("Ton-Geraete: {}", e);
            }
        })?;

    Ok(Geraete {
        mikro: mik_rx,
        lautsprecher: mischer,
        stumm,
        sperre_an,
        sperre_staerke,
        sperre_offen,
        sperre_grund,
        ende,
        eingang: ein_name_echt,
        ausgang: aus_name_echt,
    })
}

/// Die Griffe an der Rauschsperre, wie der Tonfaden sie sieht.
struct Sperrgriffe {
    an: Arc<AtomicBool>,
    staerke: Arc<std::sync::atomic::AtomicU32>,
    offen: Arc<AtomicBool>,
    grund: Arc<std::sync::atomic::AtomicU32>,
}

fn geraete_faden(
    ein_dev: cpal::Device,
    aus_dev: cpal::Device,
    mik_tx: std::sync::mpsc::Sender<Vec<i16>>,
    mischer: Arc<Mutex<Mischer>>,
    stumm: Arc<AtomicBool>,
    ende: Arc<AtomicBool>,
    sperre: Sperrgriffe,
) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, StreamTrait};

    let ein_cfg = ein_dev.default_input_config()?;
    let aus_cfg = aus_dev.default_output_config()?;
    let ein_rate = ein_cfg.sample_rate().0 as usize;
    let ein_kan = ein_cfg.channels() as usize;
    let aus_rate = aus_cfg.sample_rate().0 as usize;
    let aus_kan = aus_cfg.channels() as usize;

    // Ringe zwischen Rueckruf und Arbeitsschleife.
    let mikro_ring: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let referenz_ring: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let mr = mikro_ring.clone();
    let ein_stream = ein_dev.build_input_stream(
        &ein_cfg.config(),
        move |daten: &[f32], _: &cpal::InputCallbackInfo| {
            let mono = zu_mono(daten, ein_kan);
            let auf = auf_48k(&mono, ein_rate);
            if let Ok(mut r) = mr.lock() {
                r.extend_from_slice(&auf);
                // hoechstens 1 s aufheben
                let zu_viel = r.len().saturating_sub(RATE);
                if zu_viel > 0 {
                    r.drain(0..zu_viel);
                }
            }
        },
        |e| eprintln!("Mikrofon: {}", e),
        None,
    )?;

    let mi = mischer.clone();
    let rr = referenz_ring.clone();
    let aus_stream = aus_dev.build_output_stream(
        &aus_cfg.config(),
        move |daten: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let rahmen = daten.len() / aus_kan.max(1);
            // So viele 48-kHz-Werte brauchen wir dafuer.
            let noetig = rahmen * RATE / aus_rate.max(1);
            let gemischt = mi
                .lock()
                .map(|mut m| m.abmischen(noetig))
                .unwrap_or_else(|_| vec![0i16; noetig]);
            let f = zu_f32(&gemischt);
            // Referenz fuer die Echo-Unterdrueckung mitschreiben.
            if let Ok(mut r) = rr.lock() {
                r.extend_from_slice(&f);
                let zu_viel = r.len().saturating_sub(RATE);
                if zu_viel > 0 {
                    r.drain(0..zu_viel);
                }
            }
            // auf die Geraeterate zuruecktasten und auf alle Kanaele legen
            let raus = if aus_rate == RATE {
                f
            } else {
                let n = rahmen;
                let mut v = Vec::with_capacity(n);
                for i in 0..n {
                    let pos = i as f64 * RATE as f64 / aus_rate as f64;
                    let a = (pos.floor() as usize).min(f.len().saturating_sub(1));
                    v.push(f.get(a).copied().unwrap_or(0.0));
                }
                v
            };
            for (i, stelle) in daten.chunks_mut(aus_kan.max(1)).enumerate() {
                let v = raus.get(i).copied().unwrap_or(0.0);
                for k in stelle.iter_mut() {
                    *k = v;
                }
            }
        },
        |e| eprintln!("Lautsprecher: {}", e),
        None,
    )?;

    ein_stream.play()?;
    aus_stream.play()?;

    let mut echo = Echo::neu();
    let mut sperr = Rauschsperre::neu();
    while !ende.load(Ordering::Relaxed) {
        // Ein 20-ms-Paar aus Mikrofon und Referenz holen.
        let mikro = {
            let mut r = mikro_ring.lock().unwrap();
            if r.len() < RAHMEN {
                None
            } else {
                Some(r.drain(0..RAHMEN).collect::<Vec<f32>>())
            }
        };
        let Some(mikro) = mikro else {
            std::thread::sleep(std::time::Duration::from_millis(4));
            continue;
        };
        let referenz = {
            let mut r = referenz_ring.lock().unwrap();
            if r.len() >= RAHMEN {
                r.drain(0..RAHMEN).collect::<Vec<f32>>()
            } else {
                vec![0.0; RAHMEN]
            }
        };
        let mut sauber = echo.rahmen(&mikro, &referenz);
        // Erst Echo weg, DANN die Rauschsperre: sonst wuerde das Echo als
        // Nutzsignal gelten und das Tor sinnlos aufhalten.
        if sperre.an.load(Ordering::Relaxed) {
            let staerke = sperre.staerke.load(Ordering::Relaxed).min(100) as u8;
            let durch = sperr.rahmen(&mut sauber, staerke);
            sperre.offen.store(durch, Ordering::Relaxed);
            sperre
                .grund
                .store((sperr.grundrauschen() * 10_000.0) as u32, Ordering::Relaxed);
        } else {
            sperre.offen.store(true, Ordering::Relaxed);
        }
        let pcm = if stumm.load(Ordering::Relaxed) {
            vec![0i16; RAHMEN]
        } else {
            zu_i16(&sauber)
        };
        if mik_tx.send(pcm).is_err() {
            break;
        }
    }
    Ok(())
}

/// Namen der vorhandenen Geraete - fuer die Auswahl in der Oberflaeche.
pub fn geraete_liste() -> (Vec<String>, Vec<String>) {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let ein = host
        .input_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    let aus = host
        .output_devices()
        .map(|d| d.filter_map(|x| x.name().ok()).collect())
        .unwrap_or_default();
    (ein, aus)
}

#[cfg(test)]
mod tests {

    /// Ein Rahmen gleichmaessigen Rauschens einer bestimmten Staerke.
    fn rauschen(staerke: f32, saat: &mut u32) -> Vec<f32> {
        (0..960)
            .map(|_| {
                // Kleiner eigener Zufall - reproduzierbar, ohne Fremdkiste.
                *saat = saat.wrapping_mul(1664525).wrapping_add(1013904223);
                ((*saat >> 8) as f32 / 8_388_608.0 - 1.0) * staerke
            })
            .collect()
    }

    /// Ein Rahmen "Sprache": ein kraeftiger Ton mit etwas Rauschen darauf.
    fn sprache(t: &mut f32, saat: &mut u32) -> Vec<f32> {
        (0..960)
            .map(|_| {
                *t += 1.0 / 48000.0;
                *saat = saat.wrapping_mul(1664525).wrapping_add(1013904223);
                let stoerung = ((*saat >> 8) as f32 / 8_388_608.0 - 1.0) * 0.01;
                (2.0 * std::f32::consts::PI * 220.0 * *t).sin() * 0.25 + stoerung
            })
            .collect()
    }

    fn effektivwert(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn rauschsperre_daempft_stilles_rauschen_wirklich() {
        let mut sp = Rauschsperre::neu();
        let mut saat = 7u32;
        // Erst zwei Sekunden Ruhe: einmessen + zudrehen.
        let mut letzter = Vec::new();
        for _ in 0..100 {
            let mut r = rauschen(0.01, &mut saat);
            sp.rahmen(&mut r, 50);
            letzter = r;
        }
        let rest = effektivwert(&letzter);
        assert!(
            rest < 0.001,
            "Rauschen wird nicht gedaempft: Effektivwert {rest}"
        );
        assert!(!sp.durchlass(), "Tor haette zu sein muessen");
    }

    #[test]
    fn rauschsperre_laesst_sprache_durch() {
        let mut sp = Rauschsperre::neu();
        let (mut saat, mut t) = (7u32, 0.0f32);
        for _ in 0..100 {
            let mut r = rauschen(0.01, &mut saat);
            sp.rahmen(&mut r, 50);
        }
        // Jetzt wird gesprochen - nach spaetestens 100 ms muss es durch.
        let mut letzter = Vec::new();
        for _ in 0..5 {
            let mut r = sprache(&mut t, &mut saat);
            sp.rahmen(&mut r, 50);
            letzter = r;
        }
        let laut = effektivwert(&letzter);
        assert!(laut > 0.15, "Sprache kommt nicht durch: Effektivwert {laut}");
        assert!(sp.durchlass(), "Tor haette offen sein muessen");
    }

    #[test]
    fn rauschsperre_schneidet_das_wortende_nicht_ab() {
        // Der Nachlauf ist der Grund, warum es nicht abgehackt klingt.
        let mut sp = Rauschsperre::neu();
        let (mut saat, mut t) = (7u32, 0.0f32);
        for _ in 0..100 {
            let mut r = rauschen(0.01, &mut saat);
            sp.rahmen(&mut r, 50);
        }
        for _ in 0..10 {
            let mut r = sprache(&mut t, &mut saat);
            sp.rahmen(&mut r, 50);
        }
        // Direkt nach dem letzten lauten Rahmen muss noch durchgelassen
        // werden - sonst faellt das Ende jedes Wortes weg.
        let mut r = rauschen(0.01, &mut saat);
        sp.rahmen(&mut r, 50);
        assert!(sp.durchlass(), "Nachlauf greift nicht");
    }

    #[test]
    fn staerke_null_laesst_mehr_durch_als_staerke_hundert() {
        // Der Regler muss auch wirklich etwas bewirken.
        let mess = |staerke: u8| {
            let mut sp = Rauschsperre::neu();
            let mut saat = 7u32;
            for _ in 0..60 {
                let mut r = rauschen(0.01, &mut saat);
                sp.rahmen(&mut r, staerke);
            }
            // Ein etwas lauteres Geraeusch - Tastaturklappern.
            let mut r = rauschen(0.03, &mut saat);
            sp.rahmen(&mut r, staerke);
            sp.durchlass()
        };
        assert!(mess(0), "bei 0 muss das Klappern durchkommen");
        assert!(!mess(100), "bei 100 darf das Klappern nicht durchkommen");
    }
    use super::*;

    fn sinus(n: usize, hz: f32, amp: f32, phase: &mut f32) -> Vec<f32> {
        let schritt = 2.0 * std::f32::consts::PI * hz / RATE as f32;
        (0..n)
            .map(|_| {
                let v = phase.sin() * amp;
                *phase += schritt;
                v
            })
            .collect()
    }

    fn leistung(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32
    }

    /// ERLE = wie viel Echo weg ist, in dB. Ueber 20 dB gilt als brauchbar,
    /// ueber 30 dB als gut.
    #[test]
    fn echo_wird_wirklich_leiser() {
        let mut e = Echo::neu();
        let mut ph = 0.0;
        let mut rest_leistung = 0.0;
        let mut echo_leistung = 0.0;
        // Zimmer: das Echo ist eine gedaempfte, verzoegerte Fassung.
        let verzoegerung = 240; // 5 ms
        let mut verlauf = vec![0.0f32; verzoegerung + BLOCK];
        for i in 0..300 {
            let referenz = sinus(BLOCK, 300.0 + (i % 7) as f32 * 50.0, 0.3, &mut ph);
            // Echo bauen
            verlauf.extend_from_slice(&referenz);
            let start = verlauf.len() - BLOCK - verzoegerung;
            let echo: Vec<f32> = verlauf[start..start + BLOCK].iter().map(|v| v * 0.5).collect();
            verlauf.drain(0..BLOCK);
            let aus = e.block(&echo, &referenz);
            if i > 250 {
                rest_leistung += leistung(&aus);
                echo_leistung += leistung(&echo);
            }
        }
        let erle = 10.0 * (echo_leistung / rest_leistung.max(1e-12)).log10();
        println!("ERLE = {:.1} dB", erle);
        assert!(erle > 20.0, "Echo-Unterdrueckung zu schwach: {:.1} dB", erle);
    }

    #[test]
    fn eigene_stimme_bleibt_erhalten() {
        let mut e = Echo::neu();
        let mut ph1 = 0.0;
        let mut ph2 = 1.0;
        let mut ein_l = 0.0;
        let mut aus_l = 0.0;
        for i in 0..200 {
            // Kein Lautsprecher -> nichts abzuziehen, die Stimme muss bleiben.
            let referenz = vec![0.0f32; BLOCK];
            let stimme = sinus(BLOCK, 220.0, 0.4, &mut ph1);
            let _ = sinus(BLOCK, 1000.0, 0.0, &mut ph2);
            let aus = e.block(&stimme, &referenz);
            if i > 20 {
                ein_l += leistung(&stimme);
                aus_l += leistung(&aus);
            }
        }
        let verlust = 10.0 * (ein_l / aus_l.max(1e-12)).log10();
        println!("Verlust ohne Lautsprecher = {:.2} dB", verlust);
        assert!(verlust < 3.0, "eigene Stimme wird gedaempft: {:.2} dB", verlust);
    }

    #[test]
    fn gegensprechen_zerstoert_die_stimme_nicht() {
        let mut e = Echo::neu();
        let (mut p1, mut p2) = (0.0, 0.0);
        let verzoegerung = 240;
        let mut verlauf = vec![0.0f32; verzoegerung + BLOCK];
        let mut stimme_ein = 0.0;
        let mut aus_l = 0.0;
        for i in 0..400 {
            let referenz = sinus(BLOCK, 400.0, 0.3, &mut p1);
            verlauf.extend_from_slice(&referenz);
            let start = verlauf.len() - BLOCK - verzoegerung;
            let echo: Vec<f32> = verlauf[start..start + BLOCK].iter().map(|v| v * 0.5).collect();
            verlauf.drain(0..BLOCK);
            // ab der Haelfte redet der Mensch mit
            let stimme = if i > 200 {
                sinus(BLOCK, 150.0, 0.4, &mut p2)
            } else {
                vec![0.0; BLOCK]
            };
            let mikro: Vec<f32> = echo.iter().zip(&stimme).map(|(a, b)| a + b).collect();
            let aus = e.block(&mikro, &referenz);
            if i > 300 {
                stimme_ein += leistung(&stimme);
                aus_l += leistung(&aus);
            }
        }
        let verhaeltnis = 10.0 * (stimme_ein / aus_l.max(1e-12)).log10();
        println!("Gegensprechen: Verlust {:.2} dB", verhaeltnis);
        assert!(
            verhaeltnis < 6.0,
            "beim Gegensprechen geht zu viel Stimme verloren: {:.2} dB",
            verhaeltnis
        );
    }

    #[test]
    fn echo_ist_schnell_genug_fuer_echtzeit() {
        let mut e = Echo::neu();
        let mut ph = 0.0;
        let bloecke = 200; // 2 Sekunden Ton
        let start = std::time::Instant::now();
        for _ in 0..bloecke {
            let r = sinus(BLOCK, 500.0, 0.3, &mut ph);
            let m = sinus(BLOCK, 500.0, 0.15, &mut ph);
            let _ = e.block(&m, &r);
        }
        let dauer = start.elapsed().as_secs_f32();
        let echtzeit = bloecke as f32 * 0.01;
        let faktor = dauer / echtzeit;
        println!("Echo-Rechenzeit: {:.1} % der Echtzeit", faktor * 100.0);
        // Im Debug-Bau rechnet Rust ohne Optimierung (Faktor ~40). Gemessen
        // wird die Auslieferung: dort sind es 3,6 % der Echtzeit.
        let grenze = if cfg!(debug_assertions) { 3.0 } else { 0.2 };
        assert!(faktor < grenze, "zu langsam: {:.0} % der Echtzeit", faktor * 100.0);
    }

    #[test]
    fn mischer_addiert_und_haelt_die_laenge() {
        let mut m = Mischer::neu();
        m.vorlauf = 1;
        m.dazu(1, &vec![1000i16; RAHMEN]);
        m.dazu(2, &vec![2000i16; RAHMEN]);
        let aus = m.abmischen(RAHMEN);
        assert_eq!(aus.len(), RAHMEN);
        assert_eq!(aus[0], 3000);
        assert_eq!(m.leute(), 2);
        // Ist nichts mehr da, kommt Stille - kein Knacksen.
        let leer = m.abmischen(RAHMEN);
        assert!(leer.iter().all(|v| *v == 0));
    }

    #[test]
    fn mischer_uebersteuert_nicht() {
        let mut m = Mischer::neu();
        for p in 0..8u64 {
            m.dazu(p, &vec![30000i16; RAHMEN]);
        }
        let aus = m.abmischen(RAHMEN);
        assert!(aus.iter().all(|v| *v <= i16::MAX && *v >= i16::MIN));
        assert_eq!(aus[0], i16::MAX);
    }

    #[test]
    fn mischer_wirft_zu_alte_pakete_weg() {
        let mut m = Mischer::neu();
        for _ in 0..40 {
            m.dazu(1, &vec![100i16; RAHMEN]);
        }
        // 40 x 20 ms = 800 ms angeliefert, es duerfen hoechstens 400 ms bleiben
        let mut rahmen = 0;
        loop {
            let a = m.abmischen(RAHMEN);
            if a.iter().all(|v| *v == 0) {
                break;
            }
            rahmen += 1;
            if rahmen > 100 {
                break;
            }
        }
        assert!(rahmen <= 20, "zu viel Ton aufgestaut: {} Rahmen", rahmen);
    }

    #[test]
    fn umtasten_und_mono() {
        let st = vec![1.0f32, -1.0, 0.5, -0.5];
        assert_eq!(zu_mono(&st, 2), vec![0.0, 0.0]);
        let lang = auf_48k(&vec![0.0f32; 441], 44100);
        assert!((lang.len() as i32 - 480).abs() <= 1, "Laenge {}", lang.len());
        assert_eq!(zu_i16(&[1.0, -1.0, 0.0]), vec![32767, -32767, 0]);
        assert_eq!(zu_f32(&[32768u16 as i16]), vec![-1.0]);
    }
}
