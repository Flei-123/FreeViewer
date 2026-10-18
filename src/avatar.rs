//! A self-drawn avatar instead of the camera image.
//!
//! Why at all: not everyone wants to show their face (and their
//! living room) at all times, but a black tile is the other extreme -
//! you seem absent. The avatar is the middle way: it moves the mouth
//! to the own microphone level, blinks and breathes lightly. This way the
//! group sees that someone is there and who is speaking.
//!
//! Deliberately COMPLETELY self-drawn - a few ellipses and rectangles,
//! no third-party crate, no image files, no model. That costs nothing,
//! runs the same everywhere and can be checked: the tests
//! really measure pixels.
//!
//! The mouth movement comes from the level that the meeting measures anyway
//! (`NativeMeet::level`). It does NOT pretend that the lip
//! movement matches what is spoken - only a model could do that, and an
//! invented "lip reading" would be a lie.

/// What the avatar looks like. Everything is adjustable, otherwise everyone would have
/// the same face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Hautton 0..5
    pub haut: u8,
    /// Haarfarbe 0..5
    pub hair: u8,
    /// Haarform: 0 kurz, 1 lang, 2 ohne, 3 Pony
    pub form: u8,
    /// Augenfarbe 0..3
    pub eyes: u8,
    /// Background 0..5
    pub reason: u8,
    pub glasses: bool,
    pub bart: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            haut: 1,
            hair: 2,
            form: 0,
            eyes: 0,
            reason: 0,
            glasses: false,
            bart: false,
        }
    }
}

type Color = [u8; 3];

pub const SKIN_TONES: [Color; 6] = [
    [0xF7, 0xD9, 0xC4],
    [0xEA, 0xC0, 0x9C],
    [0xD2, 0xA0, 0x7A],
    [0xA9, 0x74, 0x4F],
    [0x7B, 0x4F, 0x33],
    [0x4E, 0x31, 0x1F],
];
pub const HAIR_COLORS: [Color; 6] = [
    [0x1B, 0x1B, 0x1F],
    [0x4A, 0x33, 0x24],
    [0x8B, 0x5A, 0x2B],
    [0xD8, 0xB4, 0x6A],
    [0xB0, 0x3A, 0x2E],
    [0x9A, 0x9A, 0xA6],
];
pub const EYE_COLORS: [Color; 4] = [
    [0x3A, 0x2A, 0x1E],
    [0x2F, 0x6B, 0xA8],
    [0x35, 0x7A, 0x4E],
    [0x6B, 0x5A, 0x8C],
];
pub const BASE_COLORS: [Color; 6] = [
    [0x11, 0x18, 0x28],
    [0x0E, 0x2A, 0x2E],
    [0x24, 0x16, 0x2E],
    [0x2A, 0x21, 0x12],
    [0x1B, 0x1B, 0x1B],
    [0x10, 0x2E, 0x1C],
];

pub const HAIR_SHAPES: [&str; 4] = ["kurz", "lang", "ohne", "Pony"];

fn hole(t: &[Color], i: u8) -> Color {
    t[(i as usize).min(t.len() - 1)]
}

/// Color aufhellen (>1) oder abdunkeln (<1).
fn tonen(c: Color, f: f32) -> Color {
    [
        (c[0] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[1] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[2] as f32 * f).clamp(0.0, 255.0) as u8,
    ]
}

struct Sheet<'a> {
    px: &'a mut [u8],
    b: i32,
    h: i32,
}

impl Sheet<'_> {
    fn point(&mut self, x: i32, y: i32, c: Color, coverage: f32) {
        if x < 0 || y < 0 || x >= self.b || y >= self.h || coverage <= 0.0 {
            return;
        }
        let i = (y as usize * self.b as usize + x as usize) * 3;
        let d = coverage.min(1.0);
        for k in 0..3 {
            let alt = self.px[i + k] as f32;
            self.px[i + k] = (alt + (c[k] as f32 - alt) * d) as u8;
        }
    }

    /// Filled ellipse with a soft edge (one pixel of transition) -
    /// without it a face looks like made of building blocks.
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let x0 = (cx - rx - 1.0).floor() as i32;
        let x1 = (cx + rx + 1.0).ceil() as i32;
        let y0 = (cy - ry - 1.0).floor() as i32;
        let y1 = (cy + ry + 1.0).ceil() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5 - cx) / rx;
                let dy = (y as f32 + 0.5 - cy) / ry;
                let d = dx * dx + dy * dy;
                // 1.0 = edge. The transition depends on the smaller axis.
                let soft = 1.6 / rx.min(ry).max(1.0);
                let coverage = ((1.0 - d) / soft + 0.5).clamp(0.0, 1.0);
                self.point(x, y, c, coverage);
            }
        }
    }

    /// Only the upper half of an ellipse (for hair and eyelids).
    fn half_ellipse_top(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
        let x0 = (cx - rx - 1.0).floor() as i32;
        let x1 = (cx + rx + 1.0).ceil() as i32;
        let y0 = (cy - ry - 1.0).floor() as i32;
        for y in y0..=(cy.ceil() as i32) {
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5 - cx) / rx.max(0.001);
                let dy = (y as f32 + 0.5 - cy) / ry.max(0.001);
                let d = dx * dx + dy * dy;
                let soft = 1.6 / rx.min(ry).max(1.0);
                let coverage = ((1.0 - d) / soft + 0.5).clamp(0.0, 1.0);
                self.point(x, y, c, coverage);
            }
        }
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color) {
        for y in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for x in (x0.floor() as i32)..=(x1.ceil() as i32) {
                self.point(x, y, c, 1.0);
            }
        }
    }
}

/// Draw the avatar. `out` is brought to `width_px*height_px*3` (RGB).
///
/// * `mouth` 0..1  - how far the mouth is open (from the microphone level)
/// * `blink` 0..1 - 0 open, 1 closed
/// * `breathe` -1..1 - slight up/down movement of the head
pub fn draw(
    a: &Look,
    width_px: u32,
    height_px: u32,
    mouth: f32,
    blink: f32,
    breathe: f32,
    out: &mut Vec<u8>,
) {
    let (b, h) = (width_px.max(2) as i32, height_px.max(2) as i32);
    out.clear();
    out.resize((b * h * 3) as usize, 0);
    let reason = hole(&BASE_COLORS, a.reason);
    // Slight gradient downwards - a completely flat area looks dead.
    for y in 0..h {
        let f = 1.0 + 0.25 * (y as f32 / h as f32);
        let c = tonen(reason, f);
        for x in 0..b {
            let i = ((y * b + x) * 3) as usize;
            out[i] = c[0];
            out[i + 1] = c[1];
            out[i + 2] = c[2];
        }
    }
    let mut bl = Sheet {
        px: out,
        b,
        h,
    };

    // Everything in fractions of the smaller edge, so that it fits in every
    // aspect ratio.
    let e = b.min(h) as f32;
    let mx = b as f32 * 0.5;
    let my = h as f32 * 0.5 + breathe * e * 0.008;

    let haut = hole(&SKIN_TONES, a.haut);
    let hair = hole(&HAIR_COLORS, a.hair);
    let auge = hole(&EYE_COLORS, a.eyes);

    // Schultern
    let hemd = tonen(hair, 0.55);
    bl.ellipse(mx, my + e * 0.60, e * 0.42, e * 0.26, hemd);

    // Hals
    bl.rect(
        mx - e * 0.075,
        my + e * 0.20,
        mx + e * 0.075,
        my + e * 0.42,
        tonen(haut, 0.88),
    );

    // Kopf
    let kr_x = e * 0.215;
    let kr_y = e * 0.265;
    bl.ellipse(mx - kr_x, my, e * 0.035, e * 0.05, tonen(haut, 0.94)); // Ohren
    bl.ellipse(mx + kr_x, my, e * 0.035, e * 0.05, tonen(haut, 0.94));
    bl.ellipse(mx, my, kr_x, kr_y, haut);

    // Haare
    match a.form {
        1 => {
            // lang: seitlich herunter
            bl.rect(
                mx - kr_x - e * 0.02,
                my - kr_y * 0.2,
                mx - kr_x + e * 0.06,
                my + e * 0.30,
                hair,
            );
            bl.rect(
                mx + kr_x - e * 0.06,
                my - kr_y * 0.2,
                mx + kr_x + e * 0.02,
                my + e * 0.30,
                hair,
            );
            bl.half_ellipse_top(mx, my - kr_y * 0.12, kr_x * 1.06, kr_y * 0.95, hair);
        }
        2 => {}
        3 => {
            // Pony: Kappe plus gerade Stirnfranse
            bl.half_ellipse_top(mx, my - kr_y * 0.10, kr_x * 1.04, kr_y * 0.92, hair);
            bl.rect(
                mx - kr_x * 0.95,
                my - kr_y * 0.42,
                mx + kr_x * 0.95,
                my - kr_y * 0.22,
                hair,
            );
        }
        _ => {
            bl.half_ellipse_top(mx, my - kr_y * 0.16, kr_x * 1.02, kr_y * 0.86, hair);
        }
    }

    // Eyes: white, iris, pupil - and the eyelid as a skin strip above.
    let ax = e * 0.085;
    let ay = my - e * 0.045;
    let arx = e * 0.048;
    let ary = e * 0.032;
    for s in [-1.0f32, 1.0] {
        let cx = mx + s * ax;
        bl.ellipse(cx, ay, arx, ary, [0xF2, 0xF2, 0xF5]);
        bl.ellipse(cx, ay, arx * 0.52, ary * 0.72, auge);
        bl.ellipse(cx, ay, arx * 0.24, ary * 0.34, [0x11, 0x11, 0x14]);
        bl.ellipse(
            cx - arx * 0.18,
            ay - ary * 0.28,
            arx * 0.12,
            ary * 0.16,
            [0xFF, 0xFF, 0xFF],
        );
        // Eyelid: comes down from above when blinking.
        if blink > 0.01 {
            let lid = ary * 2.0 * blink.clamp(0.0, 1.0);
            bl.rect(cx - arx - 1.0, ay - ary - 1.0, cx + arx + 1.0, ay - ary + lid, haut);
        }
        // Braue
        bl.rect(
            cx - arx,
            ay - ary * 2.4,
            cx + arx,
            ay - ary * 2.0,
            tonen(hair, 0.85),
        );
    }

    // Brille
    if a.glasses {
        let rand = tonen([0x20, 0x22, 0x28], 1.0);
        for s in [-1.0f32, 1.0] {
            let cx = mx + s * ax;
            bl.ellipse(cx, ay, arx * 1.5, ary * 1.7, rand);
            bl.ellipse(cx, ay, arx * 1.32, ary * 1.48, haut);
            // Hint at glass lightly - otherwise it looks like a hole.
            bl.ellipse(cx, ay, arx, ary, [0xF2, 0xF2, 0xF5]);
            bl.ellipse(cx, ay, arx * 0.52, ary * 0.72, auge);
            bl.ellipse(cx, ay, arx * 0.24, ary * 0.34, [0x11, 0x11, 0x14]);
        }
        bl.rect(mx - ax * 0.4, ay - e * 0.004, mx + ax * 0.4, ay + e * 0.004, rand);
    }

    // Nase
    bl.rect(
        mx - e * 0.010,
        my + e * 0.005,
        mx + e * 0.010,
        my + e * 0.055,
        tonen(haut, 0.90),
    );

    // Bart
    if a.bart {
        bl.ellipse(mx, my + e * 0.135, kr_x * 0.80, kr_y * 0.42, tonen(hair, 0.9));
        bl.ellipse(mx, my + e * 0.085, kr_x * 0.52, kr_y * 0.20, haut);
    }

    // Mouth - that is the moving part.
    let auf = mouth.clamp(0.0, 1.0);
    let mrx = e * (0.052 + 0.020 * auf);
    let mry = e * (0.008 + 0.052 * auf);
    bl.ellipse(mx, my + e * 0.125, mrx, mry, [0x6E, 0x2C, 0x33]);
    if auf > 0.25 {
        // With a wide open mouth, teeth become visible at the top.
        bl.ellipse(
            mx,
            my + e * 0.125 - mry * 0.55,
            mrx * 0.82,
            mry * 0.30,
            [0xEE, 0xEE, 0xEE],
        );
    }
}

/// The level fluctuates far too fast for a mouth. Here it is
/// smoothed and spread: quiet background noise moves nothing, normal
/// speech moves clearly.
pub fn mouth_shape(level: f32, before: f32) -> f32 {
    let roh = ((level - 0.03) / 0.25).clamp(0.0, 1.0);
    // Opening faster than closing - that way it looks like a mouth and
    // not like a turn signal.
    if roh > before {
        before + (roh - before) * 0.55
    } else {
        before + (roh - before) * 0.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(px: &[u8], b: u32, x: u32, y: u32) -> Color {
        let i = ((y * b + x) * 3) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    #[test]
    fn image_has_the_right_size() {
        let mut px = Vec::new();
        draw(&Look::default(), 320, 240, 0.0, 0.0, 0.0, &mut px);
        assert_eq!(px.len(), 320 * 240 * 3);
    }

    #[test]
    fn face_is_in_the_middle_not_the_background() {
        let a = Look::default();
        let mut px = Vec::new();
        draw(&a, 320, 320, 0.0, 0.0, 0.0, &mut px);
        // Ecke = Background, Mitte = Haut. Waeren beide gleich, waere
        // gar nichts gezeichnet worden.
        let corner = point(&px, 320, 3, 3);
        // Deliberately the CHEEK: depending on the hairstyle, hair sits in the
        // middle of the image, which would be a shaky check point.
        let mitte = point(&px, 320, 120, 185);
        assert_ne!(corner, mitte, "apparently nothing was drawn");
        let haut = SKIN_TONES[a.haut as usize];
        let nah = (mitte[0] as i32 - haut[0] as i32).abs()
            + (mitte[1] as i32 - haut[1] as i32).abs()
            + (mitte[2] as i32 - haut[2] as i32).abs();
        assert!(nah < 120, "no skin in the middle: {:?}", mitte);
    }

    /// The actual point of the exercise: does the mouth REALLY move?
    #[test]
    fn open_mouth_is_measurably_larger_than_closed() {
        let a = Look::default();
        let dark = |px: &[u8]| {
            // Count pixels in the mouth area that are dark red.
            let mut n = 0;
            for y in 200..260u32 {
                for x in 120..200u32 {
                    let c = point(px, 320, x, y);
                    if c[0] > 60 && c[0] < 140 && c[1] < 70 && c[2] < 80 {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut zu = Vec::new();
        draw(&a, 320, 320, 0.0, 0.0, 0.0, &mut zu);
        let mut auf = Vec::new();
        draw(&a, 320, 320, 1.0, 0.0, 0.0, &mut auf);
        let (nz, na) = (dark(&zu), dark(&auf));
        assert!(
            na > nz * 2,
            "the mouth barely moves: closed={} open={}",
            nz,
            na
        );
    }

    #[test]
    fn closed_eyes_show_no_white() {
        let a = Look::default();
        let weiss = |px: &[u8]| {
            let mut n = 0;
            for y in 130..175u32 {
                for x in 110..210u32 {
                    let c = point(px, 320, x, y);
                    if c[0] > 225 && c[1] > 225 && c[2] > 225 {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut is_open = Vec::new();
        draw(&a, 320, 320, 0.0, 0.0, 0.0, &mut is_open);
        let mut zu = Vec::new();
        draw(&a, 320, 320, 0.0, 1.0, 0.0, &mut zu);
        assert!(weiss(&is_open) > 40, "offene Augen zeigen kein Weiss");
        assert!(
            weiss(&zu) * 4 < weiss(&is_open),
            "the lid does not close: open={} closed={}",
            weiss(&is_open),
            weiss(&zu)
        );
    }

    #[test]
    fn every_setting_really_changes_the_image() {
        // A choice that changes nothing would be worse than none.
        let reason = Look::default();
        let mut a_px = Vec::new();
        draw(&reason, 160, 160, 0.0, 0.0, 0.0, &mut a_px);
        for (name, x) in [
            ("Haut", Look { haut: 4, ..reason }),
            ("Haar", Look { hair: 4, ..reason }),
            ("Form", Look { form: 1, ..reason }),
            ("Augen", Look { eyes: 2, ..reason }),
            ("Grund", Look { reason: 3, ..reason }),
            ("Brille", Look { glasses: true, ..reason }),
            ("Bart", Look { bart: true, ..reason }),
        ] {
            let mut b_px = Vec::new();
            draw(&x, 160, 160, 0.0, 0.0, 0.0, &mut b_px);
            assert!(a_px != b_px, "{} aendert nichts am Bild", name);
        }
    }

    #[test]
    fn quiet_noise_does_not_move_the_mouth() {
        let mut m = 0.0;
        for _ in 0..30 {
            m = mouth_shape(0.02, m);
        }
        assert!(m < 0.01, "Grundrauschen bewegt den Mund: {m}");
        for _ in 0..30 {
            m = mouth_shape(0.25, m);
        }
        assert!(m > 0.7, "speaking moves the mouth too little: {m}");
    }
}
