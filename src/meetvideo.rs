//! Native meeting - stage 2b: send video.
//!
//! The path is the same as for remote maintenance, only the target is a
//! different one: get image -> convert to NV12 -> encode H.264 (Windows
//! Media Foundation, later macOS VideoToolbox) -> as RTP into the meeting.
//! The encoder is already in the project (src/h264.rs) and is only
//! fed differently here.
//!
//! Why a test pattern? Because it lets the WHOLE chain be measured
//! without a camera having to be connected: the browser gets an
//! image that demonstrably moves, and we can count what
//! arrives. The camera attaches later at the same spot.

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

    /// An image: colour bars that wander, plus a bright block that runs from
    /// left to right. This way the cross-check shows at once whether
    /// something really moves and not just a still image arriving.
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

    /// Feed in a finished NV12 image (as the camera delivers it) -
    /// saves the RGB->NV12 conversion completely.
    pub fn nv12_rahmen(&mut self, nv12: &[u8]) -> Result<Vec<crate::h264::Chunk>> {
        let erwartet = (self.breite as usize) * (self.hoehe as usize) * 3 / 2;
        if nv12.len() < erwartet {
            return Err(anyhow!(
                "NV12 zu klein: {} statt {}",
                nv12.len(),
                erwartet
            ));
        }
        let aus = self
            .enc
            .encode(&nv12[..erwartet])
            .map_err(|e| anyhow!("kodieren: {}", e))?;
        self.bilder += 1;
        Ok(aus)
    }

    /// Encode the next image as a keyframe. Important when
    /// sharing the screen: whoever joins later otherwise sees only mush or nothing
    /// until the next keyframe.
    pub fn schluesselbild(&mut self) {
        self.enc.request_keyframe();
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


/// Decodes the H.264 frames of the others - one separate
/// decoder per participant, because each can send a different size.
pub struct Dekodierer {
    leute: std::collections::HashMap<u64, crate::h264::Decoder>,
    /// Letztes fertiges Bild je Teilnehmer: (Breite, Hoehe, RGBA).
    pub bilder: std::collections::HashMap<u64, (u32, u32, Vec<u8>)>,
    pub gezaehlt: u64,
    pub letzter_fehler: String,
    /// Counts up per participant as soon as a NEW image is finished. The
    /// UI reloads the graphics card only then - otherwise it would
    /// upload every image several times.
    pub stand: std::collections::HashMap<u64, u64>,
}

impl Default for Dekodierer {
    fn default() -> Self {
        Self::neu()
    }
}

impl Dekodierer {
    pub fn neu() -> Dekodierer {
        Dekodierer {
            leute: std::collections::HashMap::new(),
            bilder: std::collections::HashMap::new(),
            gezaehlt: 0,
            letzter_fehler: String::new(),
            stand: std::collections::HashMap::new(),
        }
    }

    /// Feed in a frame. Returns true if it turned into an image.
    pub fn rahmen(&mut self, peer: u64, daten: &[u8]) -> bool {
        let dec = match self.leute.entry(peer) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                // The others send whatever they want (phone in portrait,
                // screen at 1920x1080) - the decoder takes the
                // size from the stream, otherwise you only see a section.
                match crate::h264::Decoder::new_auto(BREITE, HOEHE) {
                    Ok(d) => e.insert(d),
                    Err(err) => {
                        self.letzter_fehler = format!("Dekodierer: {}", err);
                        return false;
                    }
                }
            }
        };
        let mut rgba = Vec::new();
        match dec.decode(daten, &mut rgba) {
            Ok(Some((w, h))) => {
                self.bilder.insert(peer, (w, h, rgba));
                *self.stand.entry(peer).or_insert(0) += 1;
                self.gezaehlt += 1;
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.letzter_fehler = format!("dekodieren: {}", e);
                false
            }
        }
    }

    pub fn vergessen(&mut self, peer: u64) {
        self.leute.remove(&peer);
        self.bilder.remove(&peer);
        self.stand.remove(&peer);
    }

    /// How bright is the last image of a participant (0..1)? This lets
    /// a test without a screen check whether something
    /// visible really arrives and not just black areas.
    pub fn helligkeit(&self, peer: u64) -> f32 {
        match self.bilder.get(&peer) {
            None => 0.0,
            Some((_, _, px)) => {
                if px.is_empty() {
                    return 0.0;
                }
                let summe: u64 = px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum();
                let n = (px.len() / 4).max(1) as u64;
                (summe as f32 / n as f32) / (3.0 * 255.0)
            }
        }
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
        // It is not black.
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
