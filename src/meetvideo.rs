//! Natives Meeting - Stufe 2b: Bild senden.
//!
//! Der Weg ist derselbe wie bei der Fernwartung, nur das Ziel ist ein
//! anderes: Bild besorgen -> nach NV12 wandeln -> H.264 kodieren (Windows
//! Media Foundation, spaeter macOS VideoToolbox) -> als RTP ins Meeting.
//! Der Kodierer liegt schon im Projekt (src/h264.rs) und wird hier nur
//! anders gefuettert.
//!
//! Warum ein Testmuster? Weil sich damit die GANZE Kette messen laesst,
//! ohne dass eine Kamera angeschlossen sein muss: der Browser bekommt ein
//! Bild, das sich nachweislich bewegt, und wir koennen zaehlen, was
//! ankommt. Die Kamera haengt spaeter an derselben Stelle.

use anyhow::{anyhow, Result};

pub const BREITE: u32 = 640;
pub const HOEHE: u32 = 360;

/// Erzeugt Bilder in RGB (3 Bytes je Punkt).
pub struct Muster {
    pub breite: u32,
    pub hoehe: u32,
    schritt: u32,
}

impl Muster {
    pub fn neu(breite: u32, hoehe: u32) -> Muster {
        Muster {
            breite,
            hoehe,
            schritt: 0,
        }
    }

    /// Ein Bild: Farbbalken, die wandern, plus ein heller Block, der von
    /// links nach rechts laeuft. So sieht man in der Gegenprobe sofort, ob
    /// sich wirklich etwas bewegt und nicht nur ein Standbild ankommt.
    pub fn naechstes(&mut self) -> Vec<u8> {
        let (w, h) = (self.breite as usize, self.hoehe as usize);
        let mut rgb = vec![0u8; w * h * 3];
        let versatz = (self.schritt * 4) as usize;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let balken = ((x + versatz) / 80) % 6;
                let (r, g, b) = match balken {
                    0 => (220, 40, 40),
                    1 => (220, 200, 40),
                    2 => (40, 200, 80),
                    3 => (40, 160, 220),
                    4 => (140, 60, 200),
                    _ => (230, 230, 230),
                };
                rgb[i] = r;
                rgb[i + 1] = g;
                rgb[i + 2] = b;
            }
        }
        // wandernder heller Block
        let kante = (w.min(h) / 6).max(4);
        let bx = if w > kante { (self.schritt as usize * 7) % (w - kante) } else { 0 };
        let by = h.saturating_sub(kante) / 2;
        for y in by..(by + kante).min(h) {
            for x in bx..(bx + kante).min(w) {
                let i = (y * w + x) * 3;
                rgb[i] = 255;
                rgb[i + 1] = 255;
                rgb[i + 2] = 255;
            }
        }
        self.schritt = self.schritt.wrapping_add(1);
        rgb
    }
}

/// Kodiert Bilder nach H.264.
pub struct Kodierer {
    enc: crate::h264::Encoder,
    nv12: Vec<u8>,
    pub breite: u32,
    pub hoehe: u32,
    pub bilder: u64,
}

impl Kodierer {
    pub fn neu(breite: u32, hoehe: u32, fps: u32, bitrate: u32) -> Result<Kodierer> {
        let enc = crate::h264::Encoder::new(breite, hoehe, fps, bitrate)
            .map_err(|e| anyhow!("H.264-Kodierer: {}", e))?;
        Ok(Kodierer {
            enc,
            nv12: Vec::new(),
            breite,
            hoehe,
            bilder: 0,
        })
    }

    /// Ein RGB-Bild hineingeben, fertige H.264-Pakete herausbekommen.
    pub fn rahmen(&mut self, rgb: &[u8]) -> Result<Vec<crate::h264::Chunk>> {
        crate::h264::rgb_to_nv12(rgb, self.breite, self.hoehe, &mut self.nv12);
        let aus = self
            .enc
            .encode(&self.nv12)
            .map_err(|e| anyhow!("kodieren: {}", e))?;
        self.bilder += 1;
        Ok(aus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muster_hat_die_richtige_groesse_und_bewegt_sich() {
        let mut m = Muster::neu(BREITE, HOEHE);
        let a = m.naechstes();
        let b = m.naechstes();
        assert_eq!(a.len(), (BREITE * HOEHE * 3) as usize);
        assert_ne!(a, b, "das Bild bewegt sich nicht");
        // Es ist nicht schwarz.
        let hell = a.iter().filter(|v| **v > 200).count();
        assert!(hell > 1000, "Bild zu dunkel: {}", hell);
    }

    #[test]
    fn nv12_wandlung_passt_in_der_groesse() {
        let m = Muster::neu(64, 32).naechstes_fuer_test();
        let mut nv12 = Vec::new();
        crate::h264::rgb_to_nv12(&m, 64, 32, &mut nv12);
        // NV12 = Y (w*h) + UV (w*h/2)
        assert_eq!(nv12.len(), 64 * 32 * 3 / 2);
    }
}

#[cfg(test)]
impl Muster {
    fn naechstes_fuer_test(mut self) -> Vec<u8> {
        self.naechstes()
    }
}
