//! Ein selbst gezeichneter Avatar statt des Kamerabilds.
//!
//! Warum ueberhaupt: nicht jeder will jederzeit sein Gesicht (und sein
//! Wohnzimmer) zeigen, aber eine schwarze Kachel ist das andere Extrem -
//! man wirkt abwesend. Der Avatar ist der Mittelweg: er bewegt den Mund
//! zum eigenen Mikrofonpegel, blinzelt und atmet leicht. Damit sieht die
//! Runde, dass da jemand ist und wer gerade spricht.
//!
//! Bewusst KOMPLETT selbst gezeichnet - ein paar Ellipsen und Rechtecke,
//! keine Fremdkiste, keine Bilddateien, kein Modell. Das kostet nichts,
//! laeuft ueberall gleich und laesst sich pruefen: die Tests messen
//! wirklich Bildpunkte.
//!
//! Der Mundausschlag kommt aus dem Pegel, den das Meeting ohnehin misst
//! (`NativMeet::pegel`). Es wird NICHT so getan, als wuerde die Lippen-
//! bewegung zum Gesprochenen passen - das koennte nur ein Modell, und ein
//! erfundenes "Lippenlesen" waere eine Luege.

/// Wie der Avatar aussieht. Alles ist einstellbar, sonst haetten alle
/// dasselbe Gesicht.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aussehen {
    /// Hautton 0..5
    pub haut: u8,
    /// Haarfarbe 0..5
    pub haar: u8,
    /// Haarform: 0 kurz, 1 lang, 2 ohne, 3 Pony
    pub form: u8,
    /// Augenfarbe 0..3
    pub augen: u8,
    /// Hintergrund 0..5
    pub grund: u8,
    pub brille: bool,
    pub bart: bool,
}

impl Default for Aussehen {
    fn default() -> Self {
        Aussehen {
            haut: 1,
            haar: 2,
            form: 0,
            augen: 0,
            grund: 0,
            brille: false,
            bart: false,
        }
    }
}

type Farbe = [u8; 3];

pub const HAUTTOENE: [Farbe; 6] = [
    [0xF7, 0xD9, 0xC4],
    [0xEA, 0xC0, 0x9C],
    [0xD2, 0xA0, 0x7A],
    [0xA9, 0x74, 0x4F],
    [0x7B, 0x4F, 0x33],
    [0x4E, 0x31, 0x1F],
];
pub const HAARFARBEN: [Farbe; 6] = [
    [0x1B, 0x1B, 0x1F],
    [0x4A, 0x33, 0x24],
    [0x8B, 0x5A, 0x2B],
    [0xD8, 0xB4, 0x6A],
    [0xB0, 0x3A, 0x2E],
    [0x9A, 0x9A, 0xA6],
];
pub const AUGENFARBEN: [Farbe; 4] = [
    [0x3A, 0x2A, 0x1E],
    [0x2F, 0x6B, 0xA8],
    [0x35, 0x7A, 0x4E],
    [0x6B, 0x5A, 0x8C],
];
pub const GRUNDFARBEN: [Farbe; 6] = [
    [0x11, 0x18, 0x28],
    [0x0E, 0x2A, 0x2E],
    [0x24, 0x16, 0x2E],
    [0x2A, 0x21, 0x12],
    [0x1B, 0x1B, 0x1B],
    [0x10, 0x2E, 0x1C],
];

pub const HAARFORMEN: [&str; 4] = ["kurz", "lang", "ohne", "Pony"];

fn hole(t: &[Farbe], i: u8) -> Farbe {
    t[(i as usize).min(t.len() - 1)]
}

/// Farbe aufhellen (>1) oder abdunkeln (<1).
fn tonen(c: Farbe, f: f32) -> Farbe {
    [
        (c[0] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[1] as f32 * f).clamp(0.0, 255.0) as u8,
        (c[2] as f32 * f).clamp(0.0, 255.0) as u8,
    ]
}

struct Blatt<'a> {
    px: &'a mut [u8],
    b: i32,
    h: i32,
}

impl Blatt<'_> {
    fn punkt(&mut self, x: i32, y: i32, c: Farbe, deckung: f32) {
        if x < 0 || y < 0 || x >= self.b || y >= self.h || deckung <= 0.0 {
            return;
        }
        let i = (y as usize * self.b as usize + x as usize) * 3;
        let d = deckung.min(1.0);
        for k in 0..3 {
            let alt = self.px[i + k] as f32;
            self.px[i + k] = (alt + (c[k] as f32 - alt) * d) as u8;
        }
    }

    /// Gefuellte Ellipse mit weichem Rand (ein Bildpunkt Uebergang) -
    /// ohne den sieht ein Gesicht aus wie aus Bauklotzen.
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Farbe) {
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
                // 1.0 = Rand. Der Uebergang haengt an der kleineren Achse.
                let weich = 1.6 / rx.min(ry).max(1.0);
                let deckung = ((1.0 - d) / weich + 0.5).clamp(0.0, 1.0);
                self.punkt(x, y, c, deckung);
            }
        }
    }

    /// Nur die obere Haelfte einer Ellipse (fuer Haare und Lider).
    fn halbellipse_oben(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Farbe) {
        let x0 = (cx - rx - 1.0).floor() as i32;
        let x1 = (cx + rx + 1.0).ceil() as i32;
        let y0 = (cy - ry - 1.0).floor() as i32;
        for y in y0..=(cy.ceil() as i32) {
            for x in x0..=x1 {
                let dx = (x as f32 + 0.5 - cx) / rx.max(0.001);
                let dy = (y as f32 + 0.5 - cy) / ry.max(0.001);
                let d = dx * dx + dy * dy;
                let weich = 1.6 / rx.min(ry).max(1.0);
                let deckung = ((1.0 - d) / weich + 0.5).clamp(0.0, 1.0);
                self.punkt(x, y, c, deckung);
            }
        }
    }

    fn rechteck(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Farbe) {
        for y in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for x in (x0.floor() as i32)..=(x1.ceil() as i32) {
                self.punkt(x, y, c, 1.0);
            }
        }
    }
}

/// Den Avatar zeichnen. `out` wird auf `breite*hoehe*3` gebracht (RGB).
///
/// * `mund` 0..1  - wie weit der Mund offen ist (aus dem Mikrofonpegel)
/// * `blinzeln` 0..1 - 0 offen, 1 geschlossen
/// * `atmen` -1..1 - leichte Auf-/Abbewegung des Kopfes
pub fn zeichnen(
    a: &Aussehen,
    breite: u32,
    hoehe: u32,
    mund: f32,
    blinzeln: f32,
    atmen: f32,
    out: &mut Vec<u8>,
) {
    let (b, h) = (breite.max(2) as i32, hoehe.max(2) as i32);
    out.clear();
    out.resize((b * h * 3) as usize, 0);
    let grund = hole(&GRUNDFARBEN, a.grund);
    // Leichter Verlauf nach unten - eine voellig flache Flaeche wirkt tot.
    for y in 0..h {
        let f = 1.0 + 0.25 * (y as f32 / h as f32);
        let c = tonen(grund, f);
        for x in 0..b {
            let i = ((y * b + x) * 3) as usize;
            out[i] = c[0];
            out[i + 1] = c[1];
            out[i + 2] = c[2];
        }
    }
    let mut bl = Blatt {
        px: out,
        b,
        h,
    };

    // Alles in Anteilen der kleineren Kante, damit es in jedem
    // Seitenverhaeltnis stimmt.
    let e = b.min(h) as f32;
    let mx = b as f32 * 0.5;
    let my = h as f32 * 0.5 + atmen * e * 0.008;

    let haut = hole(&HAUTTOENE, a.haut);
    let haar = hole(&HAARFARBEN, a.haar);
    let auge = hole(&AUGENFARBEN, a.augen);

    // Schultern
    let hemd = tonen(haar, 0.55);
    bl.ellipse(mx, my + e * 0.60, e * 0.42, e * 0.26, hemd);

    // Hals
    bl.rechteck(
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
            bl.rechteck(
                mx - kr_x - e * 0.02,
                my - kr_y * 0.2,
                mx - kr_x + e * 0.06,
                my + e * 0.30,
                haar,
            );
            bl.rechteck(
                mx + kr_x - e * 0.06,
                my - kr_y * 0.2,
                mx + kr_x + e * 0.02,
                my + e * 0.30,
                haar,
            );
            bl.halbellipse_oben(mx, my - kr_y * 0.12, kr_x * 1.06, kr_y * 0.95, haar);
        }
        2 => {}
        3 => {
            // Pony: Kappe plus gerade Stirnfranse
            bl.halbellipse_oben(mx, my - kr_y * 0.10, kr_x * 1.04, kr_y * 0.92, haar);
            bl.rechteck(
                mx - kr_x * 0.95,
                my - kr_y * 0.42,
                mx + kr_x * 0.95,
                my - kr_y * 0.22,
                haar,
            );
        }
        _ => {
            bl.halbellipse_oben(mx, my - kr_y * 0.16, kr_x * 1.02, kr_y * 0.86, haar);
        }
    }

    // Augen: Weiss, Iris, Pupille - und das Lid als Hautstreifen darueber.
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
        // Lid: faehrt beim Blinzeln von oben herunter.
        if blinzeln > 0.01 {
            let lid = ary * 2.0 * blinzeln.clamp(0.0, 1.0);
            bl.rechteck(cx - arx - 1.0, ay - ary - 1.0, cx + arx + 1.0, ay - ary + lid, haut);
        }
        // Braue
        bl.rechteck(
            cx - arx,
            ay - ary * 2.4,
            cx + arx,
            ay - ary * 2.0,
            tonen(haar, 0.85),
        );
    }

    // Brille
    if a.brille {
        let rand = tonen([0x20, 0x22, 0x28], 1.0);
        for s in [-1.0f32, 1.0] {
            let cx = mx + s * ax;
            bl.ellipse(cx, ay, arx * 1.5, ary * 1.7, rand);
            bl.ellipse(cx, ay, arx * 1.32, ary * 1.48, haut);
            // Glas leicht andeuten - sonst sieht es aus wie ein Loch.
            bl.ellipse(cx, ay, arx, ary, [0xF2, 0xF2, 0xF5]);
            bl.ellipse(cx, ay, arx * 0.52, ary * 0.72, auge);
            bl.ellipse(cx, ay, arx * 0.24, ary * 0.34, [0x11, 0x11, 0x14]);
        }
        bl.rechteck(mx - ax * 0.4, ay - e * 0.004, mx + ax * 0.4, ay + e * 0.004, rand);
    }

    // Nase
    bl.rechteck(
        mx - e * 0.010,
        my + e * 0.005,
        mx + e * 0.010,
        my + e * 0.055,
        tonen(haut, 0.90),
    );

    // Bart
    if a.bart {
        bl.ellipse(mx, my + e * 0.135, kr_x * 0.80, kr_y * 0.42, tonen(haar, 0.9));
        bl.ellipse(mx, my + e * 0.085, kr_x * 0.52, kr_y * 0.20, haut);
    }

    // Mund - das ist der bewegte Teil.
    let auf = mund.clamp(0.0, 1.0);
    let mrx = e * (0.052 + 0.020 * auf);
    let mry = e * (0.008 + 0.052 * auf);
    bl.ellipse(mx, my + e * 0.125, mrx, mry, [0x6E, 0x2C, 0x33]);
    if auf > 0.25 {
        // Bei weit offenem Mund werden Zaehne oben sichtbar.
        bl.ellipse(
            mx,
            my + e * 0.125 - mry * 0.55,
            mrx * 0.82,
            mry * 0.30,
            [0xEE, 0xEE, 0xEE],
        );
    }
}

/// Der Pegel schwankt viel zu schnell fuer einen Mund. Hier wird er
/// geglaettet und gespreizt: leises Grundrauschen bewegt nichts, normales
/// Sprechen bewegt deutlich.
pub fn mundstellung(pegel: f32, vorher: f32) -> f32 {
    let roh = ((pegel - 0.03) / 0.25).clamp(0.0, 1.0);
    // Aufmachen schneller als Zumachen - so wirkt es wie ein Mund und
    // nicht wie ein Blinker.
    if roh > vorher {
        vorher + (roh - vorher) * 0.55
    } else {
        vorher + (roh - vorher) * 0.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn punkt(px: &[u8], b: u32, x: u32, y: u32) -> Farbe {
        let i = ((y * b + x) * 3) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    #[test]
    fn bild_hat_die_richtige_groesse() {
        let mut px = Vec::new();
        zeichnen(&Aussehen::default(), 320, 240, 0.0, 0.0, 0.0, &mut px);
        assert_eq!(px.len(), 320 * 240 * 3);
    }

    #[test]
    fn in_der_mitte_steht_ein_gesicht_und_nicht_der_hintergrund() {
        let a = Aussehen::default();
        let mut px = Vec::new();
        zeichnen(&a, 320, 320, 0.0, 0.0, 0.0, &mut px);
        // Ecke = Hintergrund, Mitte = Haut. Waeren beide gleich, waere
        // gar nichts gezeichnet worden.
        let ecke = punkt(&px, 320, 3, 3);
        // Bewusst die WANGE: in der Bildmitte sitzen je nach Frisur die
        // Haare, das waere ein wackliger Pruefpunkt.
        let mitte = punkt(&px, 320, 120, 185);
        assert_ne!(ecke, mitte, "es wurde offenbar nichts gezeichnet");
        let haut = HAUTTOENE[a.haut as usize];
        let nah = (mitte[0] as i32 - haut[0] as i32).abs()
            + (mitte[1] as i32 - haut[1] as i32).abs()
            + (mitte[2] as i32 - haut[2] as i32).abs();
        assert!(nah < 120, "in der Mitte steht keine Haut: {:?}", mitte);
    }

    /// Der eigentliche Punkt der Uebung: bewegt sich der Mund WIRKLICH?
    #[test]
    fn offener_mund_ist_messbar_groesser_als_geschlossener() {
        let a = Aussehen::default();
        let dunkel = |px: &[u8]| {
            // Bildpunkte im Mundbereich zaehlen, die rot-dunkel sind.
            let mut n = 0;
            for y in 200..260u32 {
                for x in 120..200u32 {
                    let c = punkt(px, 320, x, y);
                    if c[0] > 60 && c[0] < 140 && c[1] < 70 && c[2] < 80 {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut zu = Vec::new();
        zeichnen(&a, 320, 320, 0.0, 0.0, 0.0, &mut zu);
        let mut auf = Vec::new();
        zeichnen(&a, 320, 320, 1.0, 0.0, 0.0, &mut auf);
        let (nz, na) = (dunkel(&zu), dunkel(&auf));
        assert!(
            na > nz * 2,
            "der Mund bewegt sich kaum: zu={} auf={}",
            nz,
            na
        );
    }

    #[test]
    fn geschlossene_augen_zeigen_kein_weiss_mehr() {
        let a = Aussehen::default();
        let weiss = |px: &[u8]| {
            let mut n = 0;
            for y in 130..175u32 {
                for x in 110..210u32 {
                    let c = punkt(px, 320, x, y);
                    if c[0] > 225 && c[1] > 225 && c[2] > 225 {
                        n += 1;
                    }
                }
            }
            n
        };
        let mut offen = Vec::new();
        zeichnen(&a, 320, 320, 0.0, 0.0, 0.0, &mut offen);
        let mut zu = Vec::new();
        zeichnen(&a, 320, 320, 0.0, 1.0, 0.0, &mut zu);
        assert!(weiss(&offen) > 40, "offene Augen zeigen kein Weiss");
        assert!(
            weiss(&zu) * 4 < weiss(&offen),
            "das Lid schliesst nicht: offen={} zu={}",
            weiss(&offen),
            weiss(&zu)
        );
    }

    #[test]
    fn jede_einstellung_aendert_das_bild_wirklich() {
        // Eine Auswahl, die nichts bewirkt, waere schlimmer als keine.
        let grund = Aussehen::default();
        let mut a_px = Vec::new();
        zeichnen(&grund, 160, 160, 0.0, 0.0, 0.0, &mut a_px);
        for (name, x) in [
            ("Haut", Aussehen { haut: 4, ..grund }),
            ("Haar", Aussehen { haar: 4, ..grund }),
            ("Form", Aussehen { form: 1, ..grund }),
            ("Augen", Aussehen { augen: 2, ..grund }),
            ("Grund", Aussehen { grund: 3, ..grund }),
            ("Brille", Aussehen { brille: true, ..grund }),
            ("Bart", Aussehen { bart: true, ..grund }),
        ] {
            let mut b_px = Vec::new();
            zeichnen(&x, 160, 160, 0.0, 0.0, 0.0, &mut b_px);
            assert!(a_px != b_px, "{} aendert nichts am Bild", name);
        }
    }

    #[test]
    fn leises_rauschen_bewegt_den_mund_nicht() {
        let mut m = 0.0;
        for _ in 0..30 {
            m = mundstellung(0.02, m);
        }
        assert!(m < 0.01, "Grundrauschen bewegt den Mund: {m}");
        for _ in 0..30 {
            m = mundstellung(0.25, m);
        }
        assert!(m > 0.7, "Sprechen bewegt den Mund zu wenig: {m}");
    }
}
