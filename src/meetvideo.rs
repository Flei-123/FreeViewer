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

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;

/// Erzeugt Bilder in RGB (3 Bytes je Punkt).
pub struct Muster {
    pub width_px: u32,
    pub height_px: u32,
    step: u32,
}

impl Muster {
    pub fn neu(width_px: u32, height_px: u32) -> Muster {
        Muster {
            width_px,
            height_px,
            step: 0,
        }
    }

    /// An image: colour bars that wander, plus a bright block that runs from
    /// left to right. This way the cross-check shows at once whether
    /// something really moves and not just a still image arriving.
    pub fn next(&mut self) -> Vec<u8> {
        let (w, h) = (self.width_px as usize, self.height_px as usize);
        let mut rgb = vec![0u8; w * h * 3];
        let offset = (self.step * 4) as usize;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let bar = ((x + offset) / 80) % 6;
                let (r, g, b) = match bar {
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
        let bx = if w > kante { (self.step as usize * 7) % (w - kante) } else { 0 };
        let by = h.saturating_sub(kante) / 2;
        for y in by..(by + kante).min(h) {
            for x in bx..(bx + kante).min(w) {
                let i = (y * w + x) * 3;
                rgb[i] = 255;
                rgb[i + 1] = 255;
                rgb[i + 2] = 255;
            }
        }
        self.step = self.step.wrapping_add(1);
        rgb
    }
}

/// Kodiert Bilder nach H.264.
pub struct VideoEncoder {
    enc: crate::h264::Encoder,
    nv12: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub images: u64,
}

impl VideoEncoder {
    pub fn neu(width_px: u32, height_px: u32, fps: u32, bitrate: u32) -> Result<VideoEncoder> {
        let enc = crate::h264::Encoder::new(width_px, height_px, fps, bitrate)
            .map_err(|e| anyhow!("H.264-Kodierer: {}", e))?;
        Ok(VideoEncoder {
            enc,
            nv12: Vec::new(),
            width_px,
            height_px,
            images: 0,
        })
    }

    /// Feed in a finished NV12 image (as the camera delivers it) -
    /// saves the RGB->NV12 conversion completely.
    pub fn nv12_frame(&mut self, nv12: &[u8]) -> Result<Vec<crate::h264::Chunk>> {
        let expected = (self.width_px as usize) * (self.height_px as usize) * 3 / 2;
        if nv12.len() < expected {
            return Err(anyhow!(
                "NV12 zu klein: {} statt {}",
                nv12.len(),
                expected
            ));
        }
        let aus = self
            .enc
            .encode(&nv12[..expected])
            .map_err(|e| anyhow!("kodieren: {}", e))?;
        self.images += 1;
        Ok(aus)
    }

    /// Encode the next image as a keyframe. Important when
    /// sharing the screen: whoever joins later otherwise sees only mush or nothing
    /// until the next keyframe.
    pub fn keyframe(&mut self) {
        self.enc.request_keyframe();
    }

    /// Ein RGB-Bild hineingeben, fertige H.264-Pakete herausbekommen.
    pub fn frame_box(&mut self, rgb: &[u8]) -> Result<Vec<crate::h264::Chunk>> {
        crate::h264::rgb_to_nv12(rgb, self.width_px, self.height_px, &mut self.nv12);
        let aus = self
            .enc
            .encode(&self.nv12)
            .map_err(|e| anyhow!("kodieren: {}", e))?;
        self.images += 1;
        Ok(aus)
    }
}


/// Decodes the H.264 frames of the others - one separate
/// decoder per participant, because each can send a different size.
pub struct VideoDecoder {
    people: std::collections::HashMap<u64, crate::h264::Decoder>,
    /// Letztes fertiges Bild je Participant: (Width, Hoehe, RGBA).
    pub images: std::collections::HashMap<u64, (u32, u32, Vec<u8>)>,
    pub counted: u64,
    pub last_error: String,
    /// Counts up per participant as soon as a NEW image is finished. The
    /// UI reloads the graphics card only then - otherwise it would
    /// upload every image several times.
    pub stand: std::collections::HashMap<u64, u64>,
}

impl Default for VideoDecoder {
    fn default() -> Self {
        Self::neu()
    }
}

impl VideoDecoder {
    pub fn neu() -> VideoDecoder {
        VideoDecoder {
            people: std::collections::HashMap::new(),
            images: std::collections::HashMap::new(),
            counted: 0,
            last_error: String::new(),
            stand: std::collections::HashMap::new(),
        }
    }

    /// Feed in a frame. Returns true if it turned into an image.
    pub fn frame_box(&mut self, peer: u64, daten: &[u8]) -> bool {
        let dec = match self.people.entry(peer) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                // The others send whatever they want (phone in portrait,
                // screen at 1920x1080) - the decoder takes the
                // size from the stream, otherwise you only see a section.
                match crate::h264::Decoder::new_auto(WIDTH, HEIGHT) {
                    Ok(d) => e.insert(d),
                    Err(err) => {
                        self.last_error = format!("Dekodierer: {}", err);
                        return false;
                    }
                }
            }
        };
        let mut rgba = Vec::new();
        match dec.decode(daten, &mut rgba) {
            Ok(Some((w, h))) => {
                self.images.insert(peer, (w, h, rgba));
                *self.stand.entry(peer).or_insert(0) += 1;
                self.counted += 1;
                true
            }
            Ok(None) => false,
            Err(e) => {
                self.last_error = format!("dekodieren: {}", e);
                false
            }
        }
    }

    pub fn forget(&mut self, peer: u64) {
        self.people.remove(&peer);
        self.images.remove(&peer);
        self.stand.remove(&peer);
    }

    /// How bright is the last image of a participant (0..1)? This lets
    /// a test without a screen check whether something
    /// visible really arrives and not just black areas.
    pub fn brightness(&self, peer: u64) -> f32 {
        match self.images.get(&peer) {
            None => 0.0,
            Some((_, _, px)) => {
                if px.is_empty() {
                    return 0.0;
                }
                let total: u64 = px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum();
                let n = (px.len() / 4).max(1) as u64;
                (total as f32 / n as f32) / (3.0 * 255.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_has_the_right_size_and_moves() {
        let mut m = Muster::neu(WIDTH, HEIGHT);
        let a = m.next();
        let b = m.next();
        assert_eq!(a.len(), (WIDTH * HEIGHT * 3) as usize);
        assert_ne!(a, b, "the frame does not move");
        // It is not black.
        let hell = a.iter().filter(|v| **v > 200).count();
        assert!(hell > 1000, "Bild zu dunkel: {}", hell);
    }

    #[test]
    fn nv12_conversion_fits_the_size() {
        let m = Muster::neu(64, 32).next_for_test();
        let mut nv12 = Vec::new();
        crate::h264::rgb_to_nv12(&m, 64, 32, &mut nv12);
        // NV12 = Y (w*h) + UV (w*h/2)
        assert_eq!(nv12.len(), 64 * 32 * 3 / 2);
    }
}

#[cfg(test)]
impl Muster {
    fn next_for_test(mut self) -> Vec<u8> {
        self.next()
    }
}
