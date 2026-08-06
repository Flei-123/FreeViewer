//! Das Meetingfenster - so wie der FreeMeet-Browser-Client, nur nativ.
//!
//! Vorbild ist meet.fleitec.com (web/index.html + web/style.css): oben eine
//! Reihe Marken ("Tags"), in der Mitte die Buehne mit 16:9-Kacheln, rechts
//! eine ausklappbare Seitenleiste mit Chat/Leute/Info, unten die Steuerleiste
//! mit runden Symbolknoepfen. Vor dem Beitritt steht dieselbe mittige Karte
//! wie im Browser: Selbstvorschau, Pegel, zwei runde Knoepfe, Geraetewahl.
//!
//! WARUM ein eigenes Modul mit eigenem Datensatz (`Sicht`) statt direkt auf
//! NativMeet zu zeichnen: so laesst sich JEDE Ansicht ohne Server, Kamera und
//! Soundkarte zeichnen. `--uitest` baut sich eine Sicht aus der Luft und
//! rendert Beitritt, leeren Raum, volle Buehne und Bildschirmfreigabe durch -
//! genau die Faelle, die man sonst nie zu Gesicht bekommt, bevor der Kunde
//! sie meldet. Nebenbei loest es das Borrow-Problem: gezeichnet wird aus
//! einer Kopie, geklickte Wuensche kommen als `Aktion` zurueck.

use crate::icons;
use crate::theme::Palette;
use egui::{pos2, vec2, Color32, Rect, Vec2};
use std::collections::HashMap;

// ----------------------------------------------------------------- Daten

/// Ein Teilnehmer, so wie ihn die Oberflaeche braucht.
#[derive(Clone, Default)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub stumm: bool,
    pub kamera_aus: bool,
    pub hand: bool,
    pub gastgeber: bool,
    /// Nicht leer = dieser Teilnehmer laesst sich mit FreeViewer fernsteuern.
    pub fvid: String,
    pub ich: bool,
    /// Spricht gerade (Server-Hinweis oder eigener Pegel).
    pub spricht: bool,
}

/// Farbe einer Marke in der Kopfzeile.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Ton {
    #[default]
    Neutral,
    Gut,
    Warn,
    Schlecht,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reiter {
    Chat,
    Leute,
    Info,
}

/// Wohin mit den Kameras, waehrend ein Bildschirm geteilt wird?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kameraplatz {
    Seite,
    Unten,
    Aus,
}

impl Kameraplatz {
    fn weiter(self) -> Kameraplatz {
        match self {
            Kameraplatz::Seite => Kameraplatz::Unten,
            Kameraplatz::Unten => Kameraplatz::Aus,
            Kameraplatz::Aus => Kameraplatz::Seite,
        }
    }
    fn wort(self) -> &'static str {
        match self {
            Kameraplatz::Seite => "Kameras rechts",
            Kameraplatz::Unten => "Kameras unten",
            Kameraplatz::Aus => "Kameras aus",
        }
    }
}

/// Eine Zeile im Chat. `von` = 0 bedeutet Systemmeldung.
#[derive(Clone, Default)]
pub struct Chatzeile {
    pub von: u64,
    pub name: String,
    pub text: String,
    pub eigen: bool,
}

/// Alles, was das Meetingfenster zum Zeichnen braucht - einmal je Bild aus
/// dem laufenden Meeting abgeschrieben.
#[derive(Clone, Default)]
pub struct Sicht {
    pub raum: String,
    pub titel: String,
    pub gastgeber: bool,
    /// Text der Verbindungsmarke ("direkt verbunden", "verbinde ...").
    pub verbindung: String,
    pub verbindung_ton: Ton,
    pub e2e: bool,
    /// Bandbreite als fertiger Text ("340 kbit/s"); leer = nicht anzeigen.
    pub bandbreite: String,
    /// Alle im Raum, ich zuerst.
    pub leute: Vec<Person>,
    pub wartende: Vec<(u64, String)>,
    pub warteraum_an: bool,
    pub chat: Vec<Chatzeile>,
    /// Was im Reiter "Info" steht (Protokoll, Meldungen).
    pub protokoll: Vec<String>,
    pub stumm: bool,
    pub kamera_an: bool,
    pub schirm_an: bool,
    pub hand: bool,
    pub steuer_frei: bool,
    /// Wer teilt gerade einen Bildschirm: (Teilnehmer, Name).
    pub schirme: Vec<(u64, String)>,
    /// Ungelesene Chatnachrichten (Zaehler am Chat-Knopf).
    pub ungelesen: u32,
    /// Wer tippt gerade (Namen).
    pub tippen: Vec<String>,
    /// Wir stehen noch vor der Tuer - der Gastgeber muss uns hereinlassen.
    pub im_warteraum: bool,
    /// Text des Servers dazu ("Der Gastgeber wurde benachrichtigt.").
    pub warte_text: String,
    /// Geraete, die sich im LAUFENDEN Meeting umstellen lassen. Der Browser
    /// kann das auch - ohne den Raum zu verlassen.
    pub cams: Vec<String>,
    pub mics: Vec<String>,
    pub spks: Vec<String>,
    /// 0 = Standardgeraet, sonst Index+1 in der jeweiligen Liste.
    pub cam_sel: usize,
    pub mic_sel: usize,
    pub spk_sel: usize,
    /// Was gerade WIRKLICH laeuft (Name aus dem Treiber) - zur Kontrolle.
    pub kamera_name: String,
    pub ton_ein: String,
    pub ton_aus: String,
}

/// Zustand, der nur die Oberflaeche etwas angeht (nicht das Meeting).
#[derive(Clone)]
pub struct Fensterzustand {
    pub seite_offen: bool,
    pub reiter: Reiter,
    pub eingabe: String,
    pub kameraplatz: Kameraplatz,
    pub vollbild: bool,
    pub pip: bool,
    /// Was zuletzt als "schreibt gerade" gemeldet wurde. Ohne das ginge bei
    /// JEDEM Tastendruck eine Nachricht raus - der Server bekaeme Dutzende
    /// Meldungen je Wort.
    pub tippt_gemeldet: bool,
    /// Einstellungen (Geraetewahl) sind aufgeklappt.
    pub einstellungen_offen: bool,
    /// Im Bild-im-Bild auch die EIGENE Kamera zeigen.
    pub pip_selbst: bool,
}

impl Default for Fensterzustand {
    fn default() -> Self {
        Self {
            seite_offen: false,
            reiter: Reiter::Chat,
            eingabe: String::new(),
            kameraplatz: Kameraplatz::Seite,
            vollbild: false,
            pip: false,
            tippt_gemeldet: false,
            einstellungen_offen: false,
            // Sich selbst im kleinen Fenster sehen ist der Normalfall - beim
            // Bildschirmteilen will man ja pruefen, ob die Kamera laeuft.
            pip_selbst: true,
        }
    }
}

/// Die fertigen Texturen. Getrennt von `Sicht`, weil sie nicht kopierbar
/// sind und pro Bild ohnehin aus dem Dekodierer kommen.
#[derive(Default)]
pub struct Bilder {
    pub eigen: Option<egui::TextureHandle>,
    pub kameras: HashMap<u64, egui::TextureHandle>,
    pub schirme: HashMap<u64, egui::TextureHandle>,
}

/// Was der Nutzer angeklickt hat. Ausgefuehrt wird es vom Aufrufer - hier
/// wird nur gezeichnet.
#[derive(Clone, PartialEq)]
pub enum Aktion {
    Stumm(bool),
    Kamera(bool),
    Schirm(bool),
    Hand(bool),
    Steuerung(bool),
    Verlassen,
    Beenden,
    AlleStumm,
    Senden(String),
    Tippt(bool),
    Einlassen(u64),
    Abweisen(u64),
    AlleEinlassen,
    Warteraum(bool),
    Stummschalten(u64),
    Rauswerfen(u64),
    /// Eine echte FreeViewer-Sitzung zu diesem Teilnehmer aufbauen.
    Steuern(String, String),
    EinladungKopieren,
    /// Geraet im laufenden Meeting wechseln (0 = Standardgeraet).
    KameraGeraet(usize),
    MikroGeraet(usize),
    LautsprecherGeraet(usize),
    /// Geraeteliste neu einlesen (nach Ein-/Ausstecken).
    GeraeteNeuLesen,
}

/// Der Beitritts-Schirm braucht Schreibzugriff (Name, Geraetewahl).
#[derive(Clone, Default)]
pub struct Beitritt {
    pub raum: String,
    pub titel: String,
    pub name: String,
    pub mics: Vec<String>,
    pub cams: Vec<String>,
    pub mic_sel: usize,
    pub cam_sel: usize,
    pub mikro_an: bool,
    pub kamera_an: bool,
    pub geraete_da: bool,
    pub pegel: f32,
    pub hinweis: String,
    /// Beitritt laeuft schon - dann Knopf sperren.
    pub laeuft: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Beitrittsaktion {
    Beitreten,
    Zurueck,
    MikroUm,
    KameraUm,
}

// ----------------------------------------------------------------- Farben

/// Die Palette plus die drei Signalfarben, die FreeMeet benutzt. In hellen
/// Themen werden sie dunkler genommen, sonst waeren sie auf Weiss nicht
/// lesbar - Farbe ist hier eine Aussage, keine Dekoration.
pub struct Farben {
    pub p: Palette,
    pub warn: Color32,
    pub bad: Color32,
    /// Der sehr dunkle Grund der Kacheln (#05070c im Browser).
    pub tief: Color32,
}

fn dunkler(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

pub fn farben() -> Farben {
    let p = crate::theme::palette();
    Farben {
        warn: if p.dark {
            Color32::from_rgb(0xfb, 0xbf, 0x24)
        } else {
            Color32::from_rgb(0xb4, 0x53, 0x09)
        },
        bad: if p.dark {
            Color32::from_rgb(0xf8, 0x71, 0x71)
        } else {
            Color32::from_rgb(0xdc, 0x26, 0x26)
        },
        tief: if p.dark {
            dunkler(p.bg, 0.62)
        } else {
            Color32::from_rgb(0xe8, 0xec, 0xf4)
        },
        p,
    }
}

impl Farben {
    fn ton_farbe(&self, t: Ton) -> Color32 {
        match t {
            Ton::Neutral => self.p.muted,
            Ton::Gut => self.p.green,
            Ton::Warn => self.warn,
            Ton::Schlecht => self.bad,
        }
    }
}

/// Anfangsbuchstaben fuer den Kreis, wenn keine Kamera laeuft.
pub fn kuerzel(name: &str) -> String {
    let k: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    if k.is_empty() {
        "?".to_string()
    } else {
        k.to_uppercase()
    }
}

// ------------------------------------------------------- kleine Bausteine

/// Eine Marke ("Tag") in der Kopfzeile: rundes Pillenfeld, Symbol + Wort.
fn tag(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: Option<&str>,
    text: &str,
    ton: Ton,
    klickbar: bool,
) -> egui::Response {
    let farbe = f.ton_farbe(ton);
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, farbe);
    let sym_b = if symbol.is_some() { 14.0 + 6.0 } else { 0.0 };
    let groesse = vec2(galley.size().x + sym_b + 22.0, 25.0);
    let sinn = if klickbar {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(groesse, sinn);
    let hover = klickbar && resp.hovered();
    let grund = match ton {
        Ton::Neutral => {
            if hover {
                f.p.card_hi
            } else {
                f.tief
            }
        }
        _ => farbe.gamma_multiply(0.10),
    };
    let rand = match ton {
        Ton::Neutral => {
            if hover {
                f.p.line.gamma_multiply(2.0)
            } else {
                f.p.line
            }
        }
        _ => farbe.gamma_multiply(0.45),
    };
    let mal = ui.painter();
    mal.rect_filled(rect, 12.5, grund);
    mal.rect_stroke(
        rect,
        12.5,
        egui::Stroke::new(1.0, rand),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + 11.0;
    if let Some(s) = symbol {
        let r = Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0));
        icons::image(s, 14.0, farbe).paint_at(ui, r);
        x += 20.0;
    }
    ui.painter().galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        farbe,
    );
    if klickbar {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Zustand eines Knopfs in der Steuerleiste.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ctl {
    /// unauffaellig an (Standard)
    Normal,
    /// bewusst eingeschaltet (Hand, Teilen, Chat) - Akzentfarbe
    An,
    /// Mikro/Kamera abgeschaltet - das muss rot sein, nicht bunt
    Aus,
    /// Verlassen/Beenden
    Gefahr,
}

/// Ein Knopf der Fussleiste: Zeichen oben, Wort darunter - wie bei Meet
/// und Teams. Genau die Form, die der Browser-Client zeigt.
fn ctl(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: &str,
    wort: &str,
    zustand: Ctl,
    zaehler: Option<u32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 58.0), egui::Sense::click());
    let (grund, rand, vorn, wortfarbe) = match zustand {
        Ctl::Normal => (
            f.p.card_hi,
            f.p.line,
            if resp.hovered() { f.p.text } else { f.p.muted },
            if resp.hovered() { f.p.text } else { f.p.muted },
        ),
        Ctl::An => (
            f.p.accent.gamma_multiply(0.16),
            f.p.accent.gamma_multiply(0.45),
            f.p.accent,
            f.p.accent,
        ),
        Ctl::Aus => (
            f.bad.gamma_multiply(0.16),
            f.bad.gamma_multiply(0.45),
            f.bad,
            f.bad,
        ),
        Ctl::Gefahr => (
            f.bad.gamma_multiply(0.14),
            f.bad.gamma_multiply(0.32),
            f.bad,
            f.bad,
        ),
    };
    let grund = if resp.hovered() {
        grund.gamma_multiply(1.35)
    } else {
        grund
    };
    let mal = ui.painter();
    mal.rect_filled(rect, 14.0, grund);
    mal.rect_stroke(
        rect,
        14.0,
        egui::Stroke::new(1.0, rand),
        egui::StrokeKind::Inside,
    );
    let sym = Rect::from_center_size(pos2(rect.center().x, rect.top() + 20.0), vec2(22.0, 22.0));
    icons::image(symbol, 22.0, vorn).paint_at(ui, sym);
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_BOTTOM,
        wort,
        egui::FontId::proportional(10.5),
        wortfarbe,
    );
    if let Some(n) = zaehler {
        if n > 0 {
            let text = if n > 99 { "99+".to_string() } else { n.to_string() };
            let mitte = pos2(rect.right() - 13.0, rect.top() + 11.0);
            ui.painter().circle_filled(mitte, 9.0, f.bad);
            ui.painter().text(
                mitte,
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(10.0),
                Color32::from_rgb(0x2b, 0x05, 0x05),
            );
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Runder Knopf (Vorschau, Senden) - 42 px wie im Browser.
fn rund(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: &str,
    groesse: f32,
    an: bool,
    betont: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(groesse), egui::Sense::click());
    let (grund, vorn) = if betont {
        (f.p.accent, f.p.on_accent)
    } else if an {
        (f.bad, Color32::from_rgb(0x2b, 0x05, 0x05))
    } else {
        (f.tief.gamma_multiply(if resp.hovered() { 1.5 } else { 1.0 }), f.p.text)
    };
    ui.painter()
        .circle_filled(rect.center(), groesse / 2.0, grund);
    if !betont && !an {
        ui.painter().circle_stroke(
            rect.center(),
            groesse / 2.0 - 0.5,
            egui::Stroke::new(1.0, f.p.line.gamma_multiply(2.0)),
        );
    }
    let s = groesse * 0.46;
    icons::image(symbol, s, vorn).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(s)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Kleiner Knopf in Listen ("Einlassen", "Steuern").
fn mini(ui: &mut egui::Ui, f: &Farben, text: &str, betont: bool) -> egui::Response {
    let knopf = egui::Button::new(
        egui::RichText::new(text)
            .size(11.5)
            .color(if betont { f.p.on_accent } else { f.p.text }),
    )
    .fill(if betont { f.p.accent } else { f.p.card_hi })
    .stroke(egui::Stroke::new(
        1.0,
        if betont {
            Color32::TRANSPARENT
        } else {
            f.p.line
        },
    ))
    .corner_radius(9);
    ui.add(knopf).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Ein Bild in ein Rechteck malen. `ganz` = vollstaendig zeigen (nichts
/// abschneiden), sonst fuellend zuschneiden. `spiegeln` nur beim eigenen
/// Kamerabild - so kennt man sich aus dem Spiegel.
fn bild_malen(
    mal: &egui::Painter,
    rect: Rect,
    tex: &egui::TextureHandle,
    spiegeln: bool,
    ganz: bool,
) {
    let ar_bild = tex.aspect_ratio().max(0.05);
    let ar_rect = (rect.width() / rect.height().max(1.0)).max(0.05);
    let (ziel, mut uv) = if ganz {
        // Alles zeigen: hineinpassen, Rand bleibt Hintergrund.
        let mut b = rect.width();
        let mut h = b / ar_bild;
        if h > rect.height() {
            h = rect.height();
            b = h * ar_bild;
        }
        (
            Rect::from_center_size(rect.center(), vec2(b, h)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        )
    } else {
        // Fuellen: das Ueberstehende wird ueber die Bildkoordinaten
        // weggeschnitten, damit keine schwarzen Balken entstehen.
        let uv = if ar_bild > ar_rect {
            let u = (ar_rect / ar_bild).clamp(0.0, 1.0);
            Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0))
        } else {
            let v = (ar_bild / ar_rect).clamp(0.0, 1.0);
            Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0))
        };
        (rect, uv)
    };
    if spiegeln {
        uv = Rect::from_min_max(
            pos2(uv.max.x, uv.min.y),
            pos2(uv.min.x, uv.max.y),
        );
    }
    mal.image(tex.id(), ziel, uv, Color32::WHITE);
}

/// Wird das Bild abgeschnitten, wenn wir es fuellend zeichnen? Weicht das
/// Seitenverhaeltnis um mehr als ein Fuenftel ab (Hochformat vom Handy,
/// 4:3-Webcam, ultrabreiter Monitor), zeigen wir lieber alles.
fn passt_nicht(tex: &egui::TextureHandle, rect: Rect) -> bool {
    let b = tex.aspect_ratio().max(0.05);
    let k = (rect.width() / rect.height().max(1.0)).max(0.05);
    (b - k).abs() / b.max(k) > 0.20
}

/// Eine Bildschirmkachel bekommt genau die Form ihres Bildes. Sonst stuende
/// das Namensschild unter breiten schwarzen Balken statt am Bild - und die
/// Flaeche waere verschenkt.
fn schirm_rect(rect: Rect, tex: Option<&egui::TextureHandle>) -> Rect {
    let t = match tex {
        Some(t) => t,
        None => return rect,
    };
    let ar = t.aspect_ratio().max(0.05);
    let mut b = rect.width();
    let mut h = b / ar;
    if h > rect.height() {
        h = rect.height();
        b = h * ar;
    }
    Rect::from_center_size(rect.center(), vec2(b, h))
}

/// Eine Kachel wie im Browser.
pub struct Kachel {
    pub id: u64,
    pub name: String,
    pub stumm: bool,
    pub hand: bool,
    pub spricht: bool,
    pub ich: bool,
    pub schirm: bool,
    /// Kamera ausgeschaltet - dann NIE ein altes Standbild zeigen, sondern
    /// den Kreis mit den Anfangsbuchstaben. Ein eingefrorenes Gesicht waere
    /// eine Luege ueber den Zustand des anderen.
    pub kamera_aus: bool,
}

fn kachel_malen(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Kachel,
    tex: Option<&egui::TextureHandle>,
    f: &Farben,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        ui.id().with(("kachel", k.id, k.schirm)),
        egui::Sense::click(),
    );
    let mal = ui.painter_at(rect);
    mal.rect_filled(rect, 14.0, f.tief);
    match tex {
        Some(t) => {
            // Ein geteilter Bildschirm wird NIE zugeschnitten - abgeschnittener
            // Text ist wertlos.
            let ganz = k.schirm || passt_nicht(t, rect);
            bild_malen(&mal, rect, t, k.ich && !k.schirm, ganz);
        }
        None => {
            // Kein Bild: weicher Kreis mit den Anfangsbuchstaben, wie im
            // Browser. Der Verlauf entsteht aus wenigen Kreisen.
            let r = rect.height().min(rect.width()) * 0.5;
            for i in 0..6 {
                let t = i as f32 / 6.0;
                mal.circle_filled(
                    rect.center(),
                    r * (1.0 - t) * 1.4,
                    f.p.card.gamma_multiply(0.16),
                );
            }
            let kreis = (rect.height() * 0.30).clamp(16.0, 34.0);
            mal.circle_filled(rect.center(), kreis, f.p.card_hi);
            mal.circle_stroke(rect.center(), kreis, egui::Stroke::new(1.0, f.p.line));
            mal.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                kuerzel(&k.name),
                egui::FontId::proportional((kreis * 0.72).clamp(11.0, 24.0)),
                f.p.text,
            );
            if k.kamera_aus && rect.height() > 120.0 {
                mal.text(
                    rect.center() + vec2(0.0, kreis + 16.0),
                    egui::Align2::CENTER_CENTER,
                    "Kamera aus",
                    egui::FontId::proportional(12.5),
                    f.p.muted,
                );
            }
        }
    }
    // Namensschild unten links: Name, dazu Stumm- und Handzeichen.
    let font = egui::FontId::proportional(12.0);
    let schild_text = if k.schirm {
        format!("{} · Bildschirm", k.name)
    } else {
        k.name.clone()
    };
    let galley = mal.layout_no_wrap(schild_text, font, f.p.text);
    let zeichen = (if k.stumm { 1 } else { 0 } + if k.hand { 1 } else { 0 }) as f32;
    let breite = (galley.size().x + 18.0 + zeichen * 19.0).min(rect.width() - 16.0);
    let schild = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.bottom() - 8.0 - 24.0),
        vec2(breite, 24.0),
    );
    // Halbdurchsichtig wie im Browser (rgba(5,7,12,.68)) - das Bild bleibt
    // darunter zu ahnen, der Text ist trotzdem lesbar.
    mal.rect_filled(
        schild,
        9.0,
        if f.p.dark {
            Color32::from_black_alpha(180)
        } else {
            Color32::from_white_alpha(210)
        },
    );
    mal.rect_stroke(
        schild,
        9.0,
        egui::Stroke::new(1.0, f.p.line),
        egui::StrokeKind::Inside,
    );
    let mut x = schild.left() + 9.0;
    mal.galley(
        pos2(x, schild.center().y - galley.size().y / 2.0),
        galley.clone(),
        f.p.text,
    );
    x += galley.size().x + 5.0;
    if k.stumm {
        icons::image("mic-off", 13.0, f.bad).paint_at(
            ui,
            Rect::from_min_size(pos2(x, schild.center().y - 6.5), Vec2::splat(13.0)),
        );
        x += 19.0;
    }
    if k.hand {
        icons::image("hand", 13.0, f.warn).paint_at(
            ui,
            Rect::from_min_size(pos2(x, schild.center().y - 6.5), Vec2::splat(13.0)),
        );
    }
    // Rahmen zuletzt, damit er ueber dem Bild liegt.
    let (randfarbe, dicke) = if k.spricht {
        (f.p.green, 2.0)
    } else if k.hand {
        (f.warn, 2.0)
    } else {
        (f.p.line, 1.0)
    };
    ui.painter().rect_stroke(
        rect,
        14.0,
        egui::Stroke::new(dicke, randfarbe),
        egui::StrokeKind::Inside,
    );
    resp
}

// ------------------------------------------------------------- Beitritt

/// Der Schirm vor dem Beitritt - mittige Karte wie `#vorbereiten`.
pub fn beitritt_ui(
    ctx: &egui::Context,
    b: &mut Beitritt,
    vorschau: Option<&egui::TextureHandle>,
) -> Option<Beitrittsaktion> {
    let f = farben();
    let mut aktion = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg))
        .show(ctx, |ui| {
            let breite = ui.available_width().min(600.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(((ui.available_height() - 560.0) * 0.5).max(10.0));
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(breite);
                        egui::Frame::NONE
                            .fill(f.p.card)
                            .stroke(egui::Stroke::new(1.0, f.p.line))
                            .corner_radius(16)
                            .inner_margin(egui::Margin::symmetric(20, 18))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                // Die KARTE steht mittig, ihr INHALT nicht -
                                // sonst schwebten Beschriftungen zentriert im
                                // Nichts statt ueber ihrem Feld zu stehen.
                                ui.with_layout(
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        aktion = beitritt_karte(ui, &f, b, vorschau);
                                    },
                                );
                            });
                    });
                    ui.add_space(16.0);
                });
        });
    aktion
}

fn beitritt_karte(
    ui: &mut egui::Ui,
    f: &Farben,
    b: &mut Beitritt,
    vorschau: Option<&egui::TextureHandle>,
) -> Option<Beitrittsaktion> {
    let mut aktion = None;
    // Marke: Zeichen im Quadrat + Titel, wie ".marke klein" im Browser.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 9.0, f.p.accent.gamma_multiply(0.16));
        icons::image("settings", 16.0, f.p.accent)
            .paint_at(ui, Rect::from_center_size(r.center(), Vec2::splat(16.0)));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Bereit zum Beitreten?")
                .size(18.0)
                .strong()
                .color(f.p.text),
        );
    });
    let unter = if b.titel.trim().is_empty() {
        format!("Meeting {}", b.raum)
    } else {
        format!("{} · Meeting {}", b.titel.trim(), b.raum)
    };
    ui.label(egui::RichText::new(unter).size(13.0).color(f.p.muted));
    ui.add_space(10.0);

    // ---- Selbstvorschau 16:9 mit Pegelbalken und zwei runden Knoepfen
    let breite = ui.available_width();
    let hoehe = (breite * 9.0 / 16.0).clamp(120.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(vec2(breite, hoehe), egui::Sense::hover());
    {
        let mal = ui.painter_at(rect);
        mal.rect_filled(rect, 14.0, f.tief);
        match (b.kamera_an, vorschau) {
            (true, Some(t)) => bild_malen(&mal, rect, t, true, passt_nicht(t, rect)),
            _ => {
                mal.text(
                    rect.center() + vec2(0.0, 22.0),
                    egui::Align2::CENTER_CENTER,
                    if b.kamera_an {
                        "Kamera startet …"
                    } else {
                        "Kamera ist aus"
                    },
                    egui::FontId::proportional(13.0),
                    f.p.muted,
                );
            }
        }
        mal.rect_stroke(
            rect,
            14.0,
            egui::Stroke::new(1.0, f.p.line),
            egui::StrokeKind::Inside,
        );
    }
    if !b.kamera_an || vorschau.is_none() {
        icons::image("cam-off", 34.0, f.p.muted.gamma_multiply(0.6)).paint_at(
            ui,
            Rect::from_center_size(rect.center() - vec2(0.0, 8.0), Vec2::splat(34.0)),
        );
    }
    // Pegelbalken oben - man sieht sofort, ob das Mikrofon etwas hoert.
    let balken = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.top() + 12.0),
        vec2(rect.width() - 24.0, 5.0),
    );
    ui.painter()
        .rect_filled(balken, 3.0, Color32::from_white_alpha(30));
    let voll = balken.width() * b.pegel.clamp(0.0, 1.0);
    if voll > 1.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(balken.min, vec2(voll, balken.height())),
            3.0,
            if b.mikro_an { f.p.green } else { f.p.muted },
        );
    }
    // Die zwei runden Knoepfe sitzen IM Bild, unten mittig.
    let knopf_y = rect.bottom() - 32.0;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(
                pos2(rect.center().x - 47.0, knopf_y - 10.0),
                vec2(104.0, 46.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
        if rund(
            ui,
            f,
            if b.mikro_an { "mic" } else { "mic-off" },
            42.0,
            !b.mikro_an,
            false,
        )
        .on_hover_text("Mikrofon")
        .clicked()
        {
            aktion = Some(Beitrittsaktion::MikroUm);
        }
        ui.add_space(8.0);
        if rund(
            ui,
            f,
            if b.kamera_an { "cam" } else { "cam-off" },
            42.0,
            !b.kamera_an,
            false,
        )
        .on_hover_text("Kamera")
        .clicked()
        {
                aktion = Some(Beitrittsaktion::KameraUm);
            }
        },
    );

    ui.add_space(12.0);
    feld_beschriftung(ui, f, None, "Dein Name");
    ui.add(
        egui::TextEdit::singleline(&mut b.name)
            .desired_width(f32::INFINITY)
            .hint_text("Name")
            .margin(egui::Margin::symmetric(10, 7)),
    );
    ui.add_space(10.0);

    // Geraetewahl nebeneinander wie ".gitter2".
    if !b.geraete_da {
        ui.label(
            egui::RichText::new("Geräte werden eingelesen …")
                .size(11.5)
                .color(f.p.muted),
        );
        ui.add_space(4.0);
    }
    let halb = (ui.available_width() - 12.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(halb);
            feld_beschriftung(ui, f, Some("cam"), "Kamera");
            geraete_wahl(ui, "meet_cam_pick", halb, &b.cams, &mut b.cam_sel);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(halb);
            feld_beschriftung(ui, f, Some("mic"), "Mikrofon");
            geraete_wahl(ui, "meet_mic_pick", halb, &b.mics, &mut b.mic_sel);
        });
    });

    if !b.hinweis.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(&b.hinweis)
                .size(11.5)
                .color(f.p.muted),
        );
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let b_breite = (ui.available_width() - 10.0) * 0.62;
        let gross = egui::Button::image_and_text(
            icons::image("enter", 18.0, f.p.on_accent),
            egui::RichText::new(if b.laeuft {
                "Trete bei …"
            } else {
                "Jetzt beitreten"
            })
            .size(15.0)
            .strong()
            .color(f.p.on_accent),
        )
        .fill(f.p.accent)
        .stroke(egui::Stroke::NONE)
        .corner_radius(11)
        .min_size(vec2(b_breite, 44.0));
        if ui.add_enabled(!b.laeuft, gross).clicked() {
            aktion = Some(Beitrittsaktion::Beitreten);
        }
        ui.add_space(10.0);
        let zurueck = egui::Button::image_and_text(
            icons::image("back", 16.0, f.p.text),
            egui::RichText::new("Zurück").size(14.0).color(f.p.text),
        )
        .fill(f.p.card_hi)
        .stroke(egui::Stroke::new(1.0, f.p.line))
        .corner_radius(11)
        .min_size(vec2(ui.available_width(), 44.0));
        if ui.add(zurueck).clicked() {
            aktion = Some(Beitrittsaktion::Zurueck);
        }
    });
    aktion
}

fn feld_beschriftung(ui: &mut egui::Ui, f: &Farben, symbol: Option<&str>, text: &str) {
    ui.horizontal(|ui| {
        if let Some(s) = symbol {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(15.0), egui::Sense::hover());
            icons::image(s, 15.0, f.p.muted).paint_at(ui, r);
            ui.add_space(2.0);
        }
        ui.label(egui::RichText::new(text).size(11.5).color(f.p.muted));
    });
    ui.add_space(2.0);
}

fn geraete_wahl(ui: &mut egui::Ui, id: &str, breite: f32, namen: &[String], wahl: &mut usize) {
    let jetzt = if *wahl == 0 {
        "Standardgerät".to_string()
    } else {
        namen
            .get(*wahl - 1)
            .cloned()
            .unwrap_or_else(|| "Standardgerät".to_string())
    };
    egui::ComboBox::from_id_salt(id)
        .width(breite - 8.0)
        .selected_text(egui::RichText::new(kurz(&jetzt, 30)).size(12.5))
        .show_ui(ui, |ui| {
            if ui.selectable_label(*wahl == 0, "Standardgerät").clicked() {
                *wahl = 0;
            }
            for (i, n) in namen.iter().enumerate() {
                if ui.selectable_label(*wahl == i + 1, n).clicked() {
                    *wahl = i + 1;
                }
            }
        });
}

fn kurz(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

// -------------------------------------------------------------- Meeting

/// Wir stehen im Warteraum. Der Browser zeigt hier dieselbe mittige Karte
/// mit drei atmenden Punkten - ohne sie saesse man vor einem leeren Raum und
/// wuesste nicht, ob etwas kaputt ist oder ob man einfach warten muss.
pub fn warteschirm_ui(ctx: &egui::Context, titel: &str, text: &str) -> bool {
    let f = farben();
    let mut abbrechen = false;
    // Die Punkte atmen - dafuer muss neu gezeichnet werden.
    ctx.request_repaint_after(std::time::Duration::from_millis(60));
    let t = ctx.input(|i| i.time) as f32;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg))
        .show(ctx, |ui| {
            let breite = ui.available_width().min(420.0);
            ui.add_space((ui.available_height() * 0.5 - 120.0).max(12.0));
            ui.vertical_centered(|ui| {
                ui.set_max_width(breite);
                egui::Frame::NONE
                    .fill(f.p.card)
                    .stroke(egui::Stroke::new(1.0, f.p.line))
                    .corner_radius(16)
                    .inner_margin(egui::Margin::symmetric(20, 18))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                                ui.painter()
                                    .rect_filled(r, 9.0, f.p.accent.gamma_multiply(0.16));
                                icons::image("user", 16.0, f.p.accent).paint_at(
                                    ui,
                                    Rect::from_center_size(r.center(), Vec2::splat(16.0)),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(if titel.trim().is_empty() {
                                        "Bitte kurz warten"
                                    } else {
                                        titel
                                    })
                                    .size(17.0)
                                    .strong()
                                    .color(f.p.text),
                                );
                            });
                            ui.add_space(14.0);
                            let (rp, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 20.0),
                                egui::Sense::hover(),
                            );
                            for i in 0..3 {
                                let phase = t * 3.0 - i as f32 * 0.55;
                                let hub = (phase.sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                                ui.painter().circle_filled(
                                    pos2(
                                        rp.left() + 8.0 + i as f32 * 14.0,
                                        rp.center().y - hub * 3.0,
                                    ),
                                    3.5,
                                    f.p.accent.gamma_multiply(0.35 + hub * 0.65),
                                );
                            }
                            ui.add_space(10.0);
                            ui.label(
                                egui::RichText::new(if text.trim().is_empty() {
                                    "Der Gastgeber lässt dich gleich herein."
                                } else {
                                    text
                                })
                                .size(13.0)
                                .color(f.p.muted),
                            );
                            ui.add_space(14.0);
                            let knopf = egui::Button::image_and_text(
                                icons::image("back", 16.0, f.p.text),
                                egui::RichText::new("Abbrechen").size(14.0).color(f.p.text),
                            )
                            .fill(f.p.card_hi)
                            .stroke(egui::Stroke::new(1.0, f.p.line))
                            .corner_radius(11)
                            .min_size(vec2(ui.available_width(), 40.0));
                            if ui.add(knopf).clicked() {
                                abbrechen = true;
                            }
                        });
                    });
            });
        });
    abbrechen
}

/// Das laufende Meeting: Kopfzeile, Buehne, Seitenleiste, Steuerleiste.
pub fn meeting_ui(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    bilder: &Bilder,
) -> Vec<Aktion> {
    let f = farben();
    let mut aktionen: Vec<Aktion> = Vec::new();

    kopf(ctx, s, &f, &mut aktionen);
    fuss(ctx, s, z, &f, &mut aktionen);
    if z.seite_offen {
        seite(ctx, s, z, &f, &mut aktionen);
    }
    if !s.wartende.is_empty() {
        lobby_leiste(ctx, s, z, &f, &mut aktionen);
    }
    if z.einstellungen_offen {
        einstellungen(ctx, s, z, &f, &mut aktionen);
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            buehne(ui, s, z, bilder, &f);
        });
    aktionen
}

fn kopf(ctx: &egui::Context, s: &Sicht, f: &Farben, aktionen: &mut Vec<Aktion>) {
    egui::TopBottomPanel::top("meet_kopf")
        .frame(
            egui::Frame::NONE
                .fill(f.p.card)
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Marke links, wie ".kopfmarke".
                let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(r, 7.0, f.p.accent.gamma_multiply(0.16));
                icons::image("cam", 14.0, f.p.accent)
                    .paint_at(ui, Rect::from_center_size(r.center(), Vec2::splat(14.0)));
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(crate::brand::NAME)
                        .size(14.0)
                        .strong()
                        .color(f.p.text),
                );
                ui.add_space(6.0);

                let raumtext = if s.titel.trim().is_empty() {
                    s.raum.clone()
                } else {
                    format!("{} · {}", s.titel.trim(), s.raum)
                };
                tag(ui, f, None, &raumtext, Ton::Neutral, false);
                if tag(ui, f, Some("link"), "Einladung", Ton::Neutral, true)
                    .on_hover_text("Einladung in die Zwischenablage")
                    .clicked()
                {
                    aktionen.push(Aktion::EinladungKopieren);
                }
                if s.gastgeber {
                    tag(ui, f, Some("crown"), "Gastgeber", Ton::Gut, false);
                }
                tag(
                    ui,
                    f,
                    Some("signal"),
                    &s.verbindung,
                    s.verbindung_ton,
                    false,
                )
                .on_hover_text("Welcher Weg gerade benutzt wird");
                tag(
                    ui,
                    f,
                    Some("shield"),
                    if s.e2e {
                        "E2E: an"
                    } else {
                        "E2E: aus (Server sieht Medien)"
                    },
                    if s.e2e { Ton::Gut } else { Ton::Warn },
                    false,
                );
                if !s.bandbreite.is_empty() {
                    tag(ui, f, None, &s.bandbreite, Ton::Neutral, false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let wort = if s.leute.len() == 1 {
                        "1 Teilnehmer".to_string()
                    } else {
                        format!("{} Teilnehmer", s.leute.len())
                    };
                    tag(ui, f, Some("people"), &wort, Ton::Neutral, false)
                        .on_hover_text("Wer gerade im Raum ist");
                });
            });
        });
}

fn fuss(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    egui::TopBottomPanel::bottom("meet_fuss")
        .frame(
            egui::Frame::NONE
                .fill(f.p.card)
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                // Zentrieren: der Rest links und rechts als Luecke.
                let anzahl = 9
                    + if !s.schirme.is_empty() { 2 } else { 0 }
                    + if s.gastgeber { 2 } else { 0 };
                let gebraucht = anzahl as f32 * 82.0;
                let luft = ((ui.available_width() - gebraucht) * 0.5).max(0.0);
                ui.add_space(luft);

                if ctl(
                    ui,
                    f,
                    if s.stumm { "mic-off" } else { "mic" },
                    "Mikro",
                    if s.stumm { Ctl::Aus } else { Ctl::Normal },
                    None,
                )
                .on_hover_text(if s.stumm {
                    "Stummschaltung aufheben"
                } else {
                    "Stummschalten"
                })
                .clicked()
                {
                    aktionen.push(Aktion::Stumm(!s.stumm));
                }
                if ctl(
                    ui,
                    f,
                    if s.kamera_an { "cam" } else { "cam-off" },
                    "Kamera",
                    if s.kamera_an { Ctl::Normal } else { Ctl::Aus },
                    None,
                )
                .clicked()
                {
                    aktionen.push(Aktion::Kamera(!s.kamera_an));
                }
                if ctl(
                    ui,
                    f,
                    if s.schirm_an { "screen-off" } else { "screen" },
                    "Bildschirm",
                    if s.schirm_an { Ctl::An } else { Ctl::Normal },
                    None,
                )
                .clicked()
                {
                    aktionen.push(Aktion::Schirm(!s.schirm_an));
                }
                if ctl(
                    ui,
                    f,
                    "keyboard",
                    "Steuerung",
                    if s.steuer_frei { Ctl::An } else { Ctl::Normal },
                    None,
                )
                .on_hover_text("Andere dürfen diesen PC mit FreeViewer steuern")
                .clicked()
                {
                    aktionen.push(Aktion::Steuerung(!s.steuer_frei));
                }
                if ctl(
                    ui,
                    f,
                    if z.pip { "pip" } else { "pip-off" },
                    "Bild im Bild",
                    if z.pip { Ctl::An } else { Ctl::Normal },
                    None,
                )
                .on_hover_text("Kleines Fenster mit den anderen - bleibt sichtbar, wenn du deinen Bildschirm teilst")
                .clicked()
                {
                    z.pip = !z.pip;
                }
                if !s.schirme.is_empty() {
                    if ctl(
                        ui,
                        f,
                        "layout",
                        z.kameraplatz.wort(),
                        if z.kameraplatz == Kameraplatz::Aus {
                            Ctl::Aus
                        } else {
                            Ctl::Normal
                        },
                        None,
                    )
                    .on_hover_text("Wohin mit den Kameras, während ein Bildschirm geteilt wird?")
                    .clicked()
                    {
                        z.kameraplatz = z.kameraplatz.weiter();
                    }
                    if ctl(
                        ui,
                        f,
                        if z.vollbild { "shrink" } else { "full" },
                        "Vollbild",
                        if z.vollbild { Ctl::An } else { Ctl::Normal },
                        None,
                    )
                    .on_hover_text("Geteilten Bildschirm groß auf die ganze Fläche")
                    .clicked()
                    {
                        z.vollbild = !z.vollbild;
                    }
                }
                if ctl(
                    ui,
                    f,
                    "hand",
                    "Hand",
                    if s.hand { Ctl::An } else { Ctl::Normal },
                    None,
                )
                .clicked()
                {
                    aktionen.push(Aktion::Hand(!s.hand));
                }
                if ctl(
                    ui,
                    f,
                    "settings",
                    "Einstellungen",
                    if z.einstellungen_offen { Ctl::An } else { Ctl::Normal },
                    None,
                )
                .on_hover_text("Kamera, Mikrofon und Lautsprecher umstellen")
                .clicked()
                {
                    z.einstellungen_offen = !z.einstellungen_offen;
                }
                if ctl(
                    ui,
                    f,
                    "chat",
                    "Chat",
                    if z.seite_offen { Ctl::An } else { Ctl::Normal },
                    Some(s.ungelesen),
                )
                .clicked()
                {
                    z.seite_offen = !z.seite_offen;
                    z.reiter = Reiter::Chat;
                }
                if s.gastgeber {
                    if ctl(ui, f, "muteall", "Alle stumm", Ctl::Normal, None)
                        .on_hover_text("Alle Teilnehmer stummschalten")
                        .clicked()
                    {
                        aktionen.push(Aktion::AlleStumm);
                    }
                    if ctl(ui, f, "end", "Beenden", Ctl::Gefahr, None)
                        .on_hover_text("Meeting für ALLE beenden")
                        .clicked()
                    {
                        aktionen.push(Aktion::Beenden);
                    }
                }
                if ctl(ui, f, "leave", "Verlassen", Ctl::Gefahr, None).clicked() {
                    aktionen.push(Aktion::Verlassen);
                }
            });
        });
}

/// Einstellungen MITTEN im Meeting: Kamera, Mikrofon, Lautsprecher.
///
/// WARUM als eigenes Fenster und nicht in der Seitenleiste: die Wahl ist
/// eine kurze Unterbrechung, danach will man sie wieder weghaben. Der
/// Browser-Client macht es genauso (Overlay ueber der Buehne).
fn einstellungen(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    let mut offen = true;
    egui::Window::new("Einstellungen")
        .open(&mut offen)
        .collapsible(false)
        .resizable(false)
        .default_width(340.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -20.0))
        .frame(
            egui::Frame::NONE
                .fill(f.p.card)
                .stroke(egui::Stroke::new(1.0, f.p.line))
                .corner_radius(12.0)
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let breite = 300.0;

            feld_beschriftung(ui, f, Some("cam"), "Kamera");
            let mut cam = s.cam_sel;
            geraete_wahl(ui, "set_cam", breite, &s.cams, &mut cam);
            if cam != s.cam_sel {
                aktionen.push(Aktion::KameraGeraet(cam));
            }
            if !s.kamera_name.is_empty() {
                ui.label(
                    egui::RichText::new(format!("läuft: {}", kurz(&s.kamera_name, 42)))
                        .size(10.5)
                        .color(f.p.muted),
                );
            }
            ui.add_space(6.0);

            feld_beschriftung(ui, f, Some("mic"), "Mikrofon");
            let mut mic = s.mic_sel;
            geraete_wahl(ui, "set_mic", breite, &s.mics, &mut mic);
            if mic != s.mic_sel {
                aktionen.push(Aktion::MikroGeraet(mic));
            }
            ui.add_space(6.0);

            feld_beschriftung(ui, f, Some("sound"), "Lautsprecher");
            let mut spk = s.spk_sel;
            geraete_wahl(ui, "set_spk", breite, &s.spks, &mut spk);
            if spk != s.spk_sel {
                aktionen.push(Aktion::LautsprecherGeraet(spk));
            }
            if !s.ton_ein.is_empty() || !s.ton_aus.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "läuft: {} → {}",
                        kurz(&s.ton_ein, 20),
                        kurz(&s.ton_aus, 20)
                    ))
                    .size(10.5)
                    .color(f.p.muted),
                );
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if mini(ui, f, "Geräte neu suchen", false).clicked() {
                    aktionen.push(Aktion::GeraeteNeuLesen);
                }
                if mini(ui, f, "Fertig", true).clicked() {
                    z.einstellungen_offen = false;
                }
            });
            // Ehrlich bleiben: ein Wechsel der Tongeraete setzt die
            // Echoausloeschung zurueck, das hoert man kurz.
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Beim Wechsel setzt die Echounterdrückung kurz aus - das ist normal.",
                )
                .size(10.0)
                .color(f.p.muted),
            );
        });
    if !offen {
        z.einstellungen_offen = false;
    }
}

// ------------------------------------------------------------- Seitenleiste

fn seite(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    let breite = (ctx.screen_rect().width() * 0.45).min(340.0).max(220.0);
    egui::SidePanel::right("meet_seite")
        .resizable(false)
        .exact_width(breite)
        .frame(
            egui::Frame::NONE
                .fill(f.p.card)
                .inner_margin(egui::Margin::same(0)),
        )
        .show(ctx, |ui| {
            // Reiterleiste
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                let voll = ui.available_width() - 40.0;
                for (r, sym, wort) in [
                    (Reiter::Chat, "chat", "Chat"),
                    (Reiter::Leute, "people", "Leute"),
                    (Reiter::Info, "info", "Info"),
                ] {
                    if reiter_knopf(ui, f, sym, wort, z.reiter == r, voll / 3.0 - 4.0).clicked() {
                        z.reiter = r;
                    }
                }
                let (rx, rr) = ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::click());
                if rr.hovered() {
                    ui.painter().rect_filled(rx, 9.0, f.p.card_hi);
                }
                icons::image("x", 16.0, if rr.hovered() { f.p.text } else { f.p.muted })
                    .paint_at(ui, Rect::from_center_size(rx.center(), Vec2::splat(16.0)));
                if rr
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Seitenleiste schließen")
                    .clicked()
                {
                    z.seite_offen = false;
                }
            });
            ui.painter().hline(
                ui.max_rect().left()..=ui.max_rect().right(),
                ui.cursor().top(),
                egui::Stroke::new(1.0, f.p.line),
            );
            ui.add_space(1.0);
            match z.reiter {
                Reiter::Chat => reiter_chat(ui, s, z, f, aktionen),
                Reiter::Leute => reiter_leute(ui, s, f, aktionen),
                Reiter::Info => reiter_info(ui, s, f),
            }
        });
}

fn reiter_knopf(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: &str,
    wort: &str,
    aktiv: bool,
    breite: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(breite.max(40.0), 44.0), egui::Sense::click());
    let farbe = if aktiv {
        f.p.accent
    } else if resp.hovered() {
        f.p.text
    } else {
        f.p.muted
    };
    if aktiv {
        ui.painter()
            .rect_filled(rect, 10.0, f.p.accent.gamma_multiply(0.10));
        ui.painter().hline(
            (rect.left() + 8.0)..=(rect.right() - 8.0),
            rect.bottom() - 1.0,
            egui::Stroke::new(2.0, f.p.accent),
        );
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 10.0, f.p.card_hi.gamma_multiply(0.6));
    }
    icons::image(symbol, 18.0, farbe).paint_at(
        ui,
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 14.0), Vec2::splat(18.0)),
    );
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_BOTTOM,
        wort,
        egui::FontId::proportional(11.0),
        farbe,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn reiter_chat(
    ui: &mut egui::Ui,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    let hoehe = ui.available_height();
    let eingabe_hoehe = 52.0;
    egui::ScrollArea::vertical()
        .id_salt("meet_chat_log")
        .max_height(hoehe - eingabe_hoehe)
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(8.0);
            if s.chat.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(
                        egui::RichText::new("Noch keine Nachrichten.")
                            .size(12.5)
                            .color(f.p.muted),
                    );
                });
            }
            for zeile in s.chat.iter() {
                chatzeile(ui, zeile, f);
                ui.add_space(8.0);
            }
            if !s.tippen.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("{} schreibt …", s.tippen.join(", ")))
                            .size(11.5)
                            .color(f.p.muted),
                    );
                });
            }
            ui.add_space(6.0);
        });
    // Eingabezeile unten - rundes Feld plus runder Senden-Knopf.
    ui.painter().hline(
        ui.max_rect().left()..=ui.max_rect().right(),
        ui.cursor().top(),
        egui::Stroke::new(1.0, f.p.line),
    );
    ui.add_space(7.0);
    ui.horizontal(|ui| {
        ui.add_space(9.0);
        let feld = ui.add(
            egui::TextEdit::singleline(&mut z.eingabe)
                .desired_width((ui.available_width() - 58.0).max(60.0))
                .hint_text("Nachricht …")
                .margin(egui::Margin::symmetric(11, 7)),
        );
        let enter = feld.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let tippt = !z.eingabe.trim().is_empty();
        if tippt != z.tippt_gemeldet {
            z.tippt_gemeldet = tippt;
            aktionen.push(Aktion::Tippt(tippt));
        }
        let senden = rund(ui, f, "send", 34.0, false, true)
            .on_hover_text("Senden")
            .clicked();
        if (enter || senden) && !z.eingabe.trim().is_empty() {
            aktionen.push(Aktion::Senden(z.eingabe.trim().to_string()));
            z.eingabe.clear();
            z.tippt_gemeldet = false;
            if enter {
                feld.request_focus();
            }
        }
    });
    ui.add_space(7.0);
}

fn chatzeile(ui: &mut egui::Ui, zeile: &Chatzeile, f: &Farben) {
    if zeile.von == 0 {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(&zeile.text)
                    .size(11.5)
                    .color(f.p.muted),
            );
        });
        return;
    }
    let voll = ui.available_width() - 20.0;
    let layout = if zeile.eigen {
        egui::Layout::right_to_left(egui::Align::TOP)
    } else {
        egui::Layout::left_to_right(egui::Align::TOP)
    };
    ui.allocate_ui_with_layout(vec2(voll, 0.0), layout, |ui| {
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 14.0, f.p.accent);
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            kuerzel(&zeile.name),
            egui::FontId::proportional(12.0),
            f.p.on_accent,
        );
        ui.add_space(6.0);
        // Bei eigenen Nachrichten alles nach rechts ziehen - sonst klebte
        // der Name am linken Rand und rutschte aus der Leiste heraus.
        let innen = if zeile.eigen {
            egui::Layout::top_down(egui::Align::Max)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        ui.allocate_ui_with_layout(vec2((voll - 50.0).max(80.0), 0.0), innen, |ui| {
            ui.set_max_width((voll - 50.0).max(80.0));
            ui.label(
                egui::RichText::new(&zeile.name)
                    .size(10.5)
                    .color(f.p.muted),
            );
            egui::Frame::NONE
                .fill(if zeile.eigen {
                    f.p.accent.gamma_multiply(0.14)
                } else {
                    f.p.card_hi
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if zeile.eigen {
                        f.p.accent.gamma_multiply(0.35)
                    } else {
                        f.p.line
                    },
                ))
                .corner_radius(12)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&zeile.text)
                            .size(13.0)
                            .color(f.p.text),
                    );
                });
        });
    });
}

fn reiter_leute(ui: &mut egui::Ui, s: &Sicht, f: &Farben, aktionen: &mut Vec<Aktion>) {
    egui::ScrollArea::vertical()
        .id_salt("meet_leute")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_space(9.0);
            if s.gastgeber {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if mini(
                        ui,
                        f,
                        if s.warteraum_an {
                            "Warteraum: an"
                        } else {
                            "Warteraum: aus"
                        },
                        s.warteraum_an,
                    )
                    .clicked()
                    {
                        aktionen.push(Aktion::Warteraum(!s.warteraum_an));
                    }
                });
                ui.add_space(8.0);
            }
            if !s.wartende.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Wartet vor der Tür")
                            .size(11.0)
                            .color(f.warn),
                    );
                });
                ui.add_space(3.0);
                for (id, name) in s.wartende.iter() {
                    person_zeile(ui, f, name, false, |ui| {
                        if mini(ui, f, "Einlassen", true).clicked() {
                            aktionen.push(Aktion::Einlassen(*id));
                        }
                        if mini(ui, f, "Ablehnen", false).clicked() {
                            aktionen.push(Aktion::Abweisen(*id));
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if mini(ui, f, "Alle einlassen", true).clicked() {
                        aktionen.push(Aktion::AlleEinlassen);
                    }
                });
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("Im Raum ({})", s.leute.len()))
                        .size(11.0)
                        .color(f.p.muted),
                );
            });
            ui.add_space(3.0);
            for p in s.leute.iter() {
                let name = if p.ich {
                    format!("{} (du)", p.name)
                } else {
                    p.name.clone()
                };
                person_zeile(ui, f, &name, p.gastgeber, |ui| {
                    // Rechts-nach-links: was zuerst kommt, steht ganz rechts.
                    // Erst die Knoepfe, dann die Zeichen - so kleben Stumm-
                    // und Handzeichen am Namen und nicht am Fensterrand.
                    if !p.ich && !p.fvid.is_empty() {
                        if mini(ui, f, "Steuern", false)
                            .on_hover_text(format!("{} mit FreeViewer fernsteuern", p.name))
                            .clicked()
                        {
                            aktionen.push(Aktion::Steuern(p.fvid.clone(), p.name.clone()));
                        }
                    }
                    if s.gastgeber && !p.ich {
                        if mini(ui, f, "Stumm", false).clicked() {
                            aktionen.push(Aktion::Stummschalten(p.id));
                        }
                        if mini(ui, f, "Raus", false).clicked() {
                            aktionen.push(Aktion::Rauswerfen(p.id));
                        }
                    }
                    if p.kamera_aus {
                        symbol_klein(ui, "cam-off", f.p.muted);
                    }
                    if p.stumm {
                        symbol_klein(ui, "mic-off", f.bad);
                    }
                    if p.hand {
                        symbol_klein(ui, "hand", f.warn);
                    }
                });
            }
            ui.add_space(10.0);
        });
}

fn symbol_klein(ui: &mut egui::Ui, name: &str, farbe: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(15.0), egui::Sense::hover());
    icons::image(name, 15.0, farbe).paint_at(ui, r);
}

fn person_zeile(
    ui: &mut egui::Ui,
    f: &Farben,
    name: &str,
    gastgeber: bool,
    rechts: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(9.0);
        egui::Frame::NONE
            .fill(f.tief)
            .stroke(egui::Stroke::new(1.0, f.p.line))
            .corner_radius(11)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 9.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 13.0, f.p.accent);
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        kuerzel(name),
                        egui::FontId::proportional(11.0),
                        f.p.on_accent,
                    );
                    ui.add_space(3.0);
                    // ERST die Knoepfe rechts belegen, DANN den Namen in den
                    // Rest setzen und dort abschneiden. Andersherum schoebe
                    // ein langer Name die Knoepfe aus der Leiste.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        rechts(ui);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if gastgeber {
                                symbol_klein(ui, "crown", f.warn);
                            }
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name).size(13.0).color(f.p.text),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
            });
    });
    ui.add_space(4.0);
}

fn reiter_info(ui: &mut egui::Ui, s: &Sicht, f: &Farben) {
    egui::ScrollArea::vertical()
        .id_salt("meet_info")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(9.0);
            for zeile in s.protokoll.iter() {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(zeile)
                            .size(11.5)
                            .family(egui::FontFamily::Monospace)
                            .color(f.p.muted),
                    );
                });
                ui.add_space(2.0);
            }
            ui.add_space(10.0);
        });
}

/// Der Balken, der meldet, dass jemand vor der Tuer wartet.
fn lobby_leiste(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    let text = if s.wartende.len() == 1 {
        format!("{} wartet vor der Tür", s.wartende[0].1)
    } else {
        format!("{} warten vor der Tür", s.wartende.len())
    };
    egui::Area::new(egui::Id::new("meet_lobby"))
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 56.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(f.p.card_hi)
                .stroke(egui::Stroke::new(1.0, f.p.accent))
                .corner_radius(20)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        symbol_klein(ui, "user", f.p.accent);
                        ui.label(egui::RichText::new(text).size(12.5).color(f.p.text));
                        if mini(ui, f, "Alle einlassen", true).clicked() {
                            aktionen.push(Aktion::AlleEinlassen);
                        }
                        if mini(ui, f, "Ansehen", false).clicked() {
                            z.seite_offen = true;
                            z.reiter = Reiter::Leute;
                        }
                    });
                });
        });
}

// ------------------------------------------------------------------ Buehne

fn kacheln_bauen(s: &Sicht) -> (Vec<Kachel>, Vec<Kachel>) {
    let schirme = s
        .schirme
        .iter()
        .map(|(id, name)| Kachel {
            id: *id,
            name: name.clone(),
            stumm: false,
            hand: false,
            spricht: false,
            ich: s.leute.iter().any(|p| p.id == *id && p.ich),
            schirm: true,
            kamera_aus: false,
        })
        .collect();
    let kameras = s
        .leute
        .iter()
        .map(|p| Kachel {
            id: p.id,
            // Die eigene Kachel heisst "Du" - genau wie im Browser. In der
            // Teilnehmerliste steht der echte Name, dort ist er nuetzlich.
            name: if p.ich { "Du".to_string() } else { p.name.clone() },
            stumm: p.stumm,
            hand: p.hand,
            spricht: p.spricht && !p.stumm,
            ich: p.ich,
            schirm: false,
            kamera_aus: p.kamera_aus,
        })
        .collect();
    (schirme, kameras)
}

fn tex_fuer<'a>(k: &Kachel, b: &'a Bilder) -> Option<&'a egui::TextureHandle> {
    if k.kamera_aus {
        return None;
    }
    if k.schirm {
        b.schirme.get(&k.id)
    } else if k.ich {
        b.eigen.as_ref()
    } else {
        b.kameras.get(&k.id)
    }
}

fn buehne(ui: &mut egui::Ui, s: &Sicht, z: &mut Fensterzustand, b: &Bilder, f: &Farben) {
    let flaeche = ui.available_rect_before_wrap();
    ui.allocate_rect(flaeche, egui::Sense::hover());
    if flaeche.width() < 40.0 || flaeche.height() < 40.0 {
        return;
    }
    let (schirme, kameras) = kacheln_bauen(s);

    if schirme.is_empty() {
        z.vollbild = false;
        raster(ui, flaeche, &kameras, b, f);
        return;
    }
    // Ein geteilter Bildschirm ist DER Inhalt: gross und vollstaendig.
    let haupt = schirme
        .iter()
        .position(|k| !k.ich)
        .unwrap_or(0);
    if z.vollbild {
        let t = tex_fuer(&schirme[haupt], b);
        kachel_malen(ui, schirm_rect(flaeche.shrink(6.0), t), &schirme[haupt], t, f);
        return;
    }
    let lueck = 10.0;
    let rand = 10.0;
    let innen = flaeche.shrink(rand);
    let (schirm_flaeche, streifen) = match z.kameraplatz {
        Kameraplatz::Aus => (innen, None),
        Kameraplatz::Seite => {
            let sb = (innen.width() * 0.28).min(232.0).max(120.0);
            if kameras.is_empty() {
                (innen, None)
            } else {
                (
                    Rect::from_min_max(innen.min, pos2(innen.right() - sb - lueck, innen.bottom())),
                    Some(Rect::from_min_max(
                        pos2(innen.right() - sb, innen.top()),
                        innen.max,
                    )),
                )
            }
        }
        Kameraplatz::Unten => {
            let sh = (innen.height() * 0.26).min(210.0 * 9.0 / 16.0).max(70.0);
            if kameras.is_empty() {
                (innen, None)
            } else {
                (
                    Rect::from_min_max(innen.min, pos2(innen.right(), innen.bottom() - sh - lueck)),
                    Some(Rect::from_min_max(
                        pos2(innen.left(), innen.bottom() - sh),
                        innen.max,
                    )),
                )
            }
        }
    };
    // Mehrere Freigaben: untereinander, jede vollstaendig.
    let n = schirme.len().max(1);
    let h = (schirm_flaeche.height() - lueck * (n as f32 - 1.0)) / n as f32;
    for (i, k) in schirme.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(schirm_flaeche.left(), schirm_flaeche.top() + i as f32 * (h + lueck)),
            vec2(schirm_flaeche.width(), h),
        );
        if r.height() > 20.0 {
            let t = tex_fuer(k, b);
            kachel_malen(ui, schirm_rect(r, t), k, t, f);
        }
    }
    // Kamerastreifen. Die Hoehe wird so gewaehlt, dass alle hineinpassen -
    // sonst haengt die letzte Kamera unter der Fussleiste.
    if let Some(st) = streifen {
        match z.kameraplatz {
            Kameraplatz::Seite => {
                let k = kameras.len().max(1) as f32;
                let voll = st.width() * 9.0 / 16.0;
                let hoehe =
                    ((st.height() - lueck * (k - 1.0)) / k).min(voll).max(48.0);
                let breite = (hoehe * 16.0 / 9.0).min(st.width());
                for (i, kk) in kameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(
                            st.center().x - breite / 2.0,
                            st.top() + i as f32 * (hoehe + lueck),
                        ),
                        vec2(breite, hoehe),
                    );
                    if r.bottom() > st.bottom() + 2.0 {
                        break;
                    }
                    kachel_malen(ui, r, kk, tex_fuer(kk, b), f);
                }
            }
            Kameraplatz::Unten => {
                let hoehe = st.height();
                let breite = hoehe * 16.0 / 9.0;
                for (i, kk) in kameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(st.left() + i as f32 * (breite + lueck), st.top()),
                        vec2(breite, hoehe),
                    );
                    if r.right() > st.right() + 2.0 {
                        break;
                    }
                    kachel_malen(ui, r, kk, tex_fuer(kk, b), f);
                }
            }
            Kameraplatz::Aus => {}
        }
    }
}

/// Wie viele Spalten? Zoom und Meet probieren jede Spaltenzahl durch und
/// nehmen die, bei der die einzelne Kachel am GROESSTEN wird - bei festem
/// 16:9, damit keine Gesichter abgeschnitten werden. Genau das hier.
pub fn beste_aufteilung(flaeche: Vec2, anzahl: usize) -> (usize, f32) {
    if anzahl == 0 {
        return (1, 0.0);
    }
    let lueck = 10.0;
    let rand = 20.0;
    let platz_b = flaeche.x - rand;
    let platz_h = flaeche.y - rand;
    // Allein im Meeting soll die eigene Kachel nicht die ganze Flaeche fuellen.
    let grenze = if anzahl == 1 {
        (flaeche.x * 0.7).min(880.0)
    } else {
        f32::INFINITY
    };
    let (mut beste_spalten, mut beste_breite) = (1usize, 0.0f32);
    for s in 1..=anzahl {
        let zeilen = anzahl.div_ceil(s) as f32;
        let aus_breite = (platz_b - lueck * (s as f32 - 1.0)) / s as f32;
        let aus_hoehe = ((platz_h - lueck * (zeilen - 1.0)) / zeilen) * (16.0 / 9.0);
        let breite = aus_breite.min(aus_hoehe).min(grenze);
        if breite > beste_breite {
            beste_breite = breite;
            beste_spalten = s;
        }
    }
    (beste_spalten, beste_breite.max(80.0))
}

fn raster(ui: &mut egui::Ui, flaeche: Rect, kacheln: &[Kachel], b: &Bilder, f: &Farben) {
    if kacheln.is_empty() {
        ui.painter().text(
            flaeche.center(),
            egui::Align2::CENTER_CENTER,
            "Noch niemand im Raum.",
            egui::FontId::proportional(14.0),
            f.p.muted,
        );
        return;
    }
    let lueck = 10.0;
    let (spalten, breite) = beste_aufteilung(flaeche.size(), kacheln.len());
    let hoehe = breite * 9.0 / 16.0;
    let zeilen = kacheln.len().div_ceil(spalten);
    let gesamt_h = zeilen as f32 * hoehe + (zeilen as f32 - 1.0) * lueck;
    let mut y = flaeche.center().y - gesamt_h / 2.0;
    let mut i = 0usize;
    for _ in 0..zeilen {
        let in_zeile = spalten.min(kacheln.len() - i);
        let gesamt_b = in_zeile as f32 * breite + (in_zeile as f32 - 1.0) * lueck;
        let mut x = flaeche.center().x - gesamt_b / 2.0;
        for _ in 0..in_zeile {
            let r = Rect::from_min_size(pos2(x, y), vec2(breite, hoehe));
            kachel_malen(ui, r, &kacheln[i], tex_fuer(&kacheln[i], b), f);
            x += breite + lueck;
            i += 1;
        }
        y += hoehe + lueck;
    }
}

/// Das kleine Bild-im-Bild-Fenster: die Kameras der anderen, waehrend man
/// selbst den Bildschirm teilt und das grosse Fenster verdeckt ist.
/// Das kleine Fenster, das oben bleibt. Es zeigt die anderen UND - wenn
/// gewuenscht - die eigene Kamera, dazu eine schmale Leiste mit reinen
/// Symbolknoepfen.
///
/// WARUM Knoepfe hier: waehrend man den eigenen Bildschirm teilt, liegt das
/// grosse Meetingfenster hinter der geteilten Anwendung. Ohne Knoepfe im
/// kleinen Fenster kaeme man an Stummschaltung und Kamera nicht mehr heran,
/// ohne das Teilen zu unterbrechen.
pub fn pip_inhalt(ui: &mut egui::Ui, s: &Sicht, b: &Bilder, selbst: &mut bool) -> Vec<Aktion> {
    let f = farben();
    let mut aktionen: Vec<Aktion> = Vec::new();
    let (_, kameras) = kacheln_bauen(s);
    // Erst die anderen, die eigene Kachel zuletzt (wie im Browser).
    let mut zeigen: Vec<&Kachel> = kameras.iter().filter(|k| !k.ich).collect();
    if *selbst {
        if let Some(ich) = kameras.iter().find(|k| k.ich) {
            zeigen.push(ich);
        }
    }
    let flaeche = ui.available_rect_before_wrap();
    ui.allocate_rect(flaeche, egui::Sense::hover());
    ui.painter().rect_filled(flaeche, 0.0, f.p.bg);

    // --- Leiste unten: nur Symbole, damit sie in 260 px Breite passt ---
    let leiste_h = 34.0;
    let leiste = Rect::from_min_size(
        pos2(flaeche.left(), flaeche.bottom() - leiste_h),
        vec2(flaeche.width(), leiste_h),
    );
    ui.painter().rect_filled(leiste, 0.0, f.p.card);
    let knoepfe: [(&str, bool, &str); 4] = [
        (
            if s.stumm { "mic-off" } else { "mic" },
            s.stumm,
            if s.stumm { "Stummschaltung aufheben" } else { "Stummschalten" },
        ),
        (
            if s.kamera_an { "cam" } else { "cam-off" },
            !s.kamera_an,
            if s.kamera_an { "Kamera aus" } else { "Kamera an" },
        ),
        (
            if *selbst { "eye" } else { "eye-off" },
            !*selbst,
            if *selbst { "Dich hier ausblenden" } else { "Dich hier einblenden" },
        ),
        ("leave", true, "Meeting verlassen"),
    ];
    let gr = 26.0;
    let luecke = 8.0;
    let gesamt = knoepfe.len() as f32 * gr + (knoepfe.len() as f32 - 1.0) * luecke;
    let mut x = leiste.center().x - gesamt * 0.5;
    for (i, (name, aus, tip)) in knoepfe.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, leiste.center().y - gr * 0.5), Vec2::splat(gr));
        let antwort = ui
            .interact(r, ui.id().with(("pipknopf", i)), egui::Sense::click())
            .on_hover_text(*tip);
        // Der letzte Knopf ist das Auflegen - immer rot, sonst greift man daneben.
        let grund = if i == 3 {
            f.bad
        } else if *aus {
            dunkler(f.p.card_hi, 0.9)
        } else {
            f.p.card_hi
        };
        let grund = if antwort.hovered() { dunkler(grund, 1.25) } else { grund };
        ui.painter().circle_filled(r.center(), gr * 0.5, grund);
        let farbe = if i == 3 {
            Color32::WHITE
        } else if *aus {
            f.bad
        } else {
            f.p.text
        };
        let ir = Rect::from_center_size(r.center(), Vec2::splat(gr * 0.58));
        icons::image(name, gr * 0.58, farbe).paint_at(ui, ir);
        if antwort.clicked() {
            match i {
                0 => aktionen.push(Aktion::Stumm(!s.stumm)),
                1 => aktionen.push(Aktion::Kamera(!s.kamera_an)),
                2 => *selbst = !*selbst,
                _ => aktionen.push(Aktion::Verlassen),
            }
        }
        if antwort.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        x += gr + luecke;
    }

    // --- Kacheln darueber ---
    let oben = Rect::from_min_max(flaeche.min, pos2(flaeche.right(), leiste.top()));
    if zeigen.is_empty() {
        ui.painter().text(
            oben.center(),
            egui::Align2::CENTER_CENTER,
            "Niemand sonst da",
            egui::FontId::proportional(12.0),
            f.p.muted,
        );
        return aktionen;
    }
    let lueck = 6.0;
    let n = zeigen.len();
    let hoehe = ((oben.height() - lueck * (n as f32 + 1.0)) / n as f32).max(40.0);
    for (i, k) in zeigen.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(oben.left() + lueck, oben.top() + lueck + i as f32 * (hoehe + lueck)),
            vec2(oben.width() - 2.0 * lueck, hoehe),
        );
        if r.bottom() > oben.bottom() {
            break;
        }
        kachel_malen(ui, r, k, tex_fuer(k, b), &f);
    }
    aktionen
}


// --------------------------------------------------- Beispiele zum Pruefen

/// Wie viele Beispielzustaende es gibt.
pub const BEISPIELE: usize = 8;

/// Beispielzustand des Beitritts-Schirms (fuer --meetdemo).
pub fn beispiel_beitritt() -> Beitritt {
    Beitritt {
        raum: "482-913-770".into(),
        titel: "Wochenbesprechung".into(),
        name: "Justin".into(),
        mics: vec!["Mikrofon (Realtek Audio)".into(), "Headset (Jabra Evolve)".into()],
        cams: vec!["Integrated Webcam".into(), "Logitech C920".into()],
        mic_sel: 1,
        cam_sel: 1,
        mikro_an: true,
        kamera_an: true,
        geraete_da: true,
        pegel: 0.42,
        hinweis: "Kamera: Integrated Webcam".into(),
        laeuft: false,
    }
}

/// Ein Beispielzustand: (Name, Sicht, Fensterzustand).
///
/// WARUM das hier steht und nicht im Test: `--uitest` rendert damit alle
/// Ansichten headless durch UND `--meetdemo` macht davon echte Bilder. Beide
/// sehen dadurch garantiert dasselbe - sonst prueft man am Ende ein anderes
/// Fenster, als der Kunde bekommt.
pub fn beispiel(nr: usize) -> (&'static str, Sicht, Fensterzustand) {
    let namen = ["Justin", "Anna Berger", "Cem", "Dora", "Emil", "Frida"];
    let mach = |anzahl: usize, mit_schirm: bool, warteraum: bool| {
        let mut leute = Vec::new();
        for i in 0..anzahl {
            leute.push(Person {
                id: i as u64 + 1,
                name: namen[i % namen.len()].to_string(),
                stumm: i % 3 == 1,
                kamera_aus: i % 4 == 2,
                hand: i % 5 == 3,
                gastgeber: i == 0,
                fvid: if i == 1 { "497628420".into() } else { String::new() },
                ich: i == 0,
                spricht: i == 1,
            });
        }
        Sicht {
            raum: "482-913-770".into(),
            titel: "Wochenbesprechung".into(),
            gastgeber: true,
            verbindung: "direkt verbunden".into(),
            verbindung_ton: Ton::Gut,
            e2e: false,
            bandbreite: "742 kbit/s".into(),
            leute,
            wartende: if warteraum {
                vec![(90, "Gast am Handy".to_string())]
            } else {
                Vec::new()
            },
            warteraum_an: warteraum,
            chat: vec![
                Chatzeile {
                    von: 0,
                    name: String::new(),
                    text: "Anna Berger ist dazugekommen".into(),
                    eigen: false,
                },
                Chatzeile {
                    von: 2,
                    name: "Anna Berger".into(),
                    text: "Servus, ich sehe euch. Der Ton passt auch.".into(),
                    eigen: false,
                },
                Chatzeile {
                    von: 1,
                    name: "Justin".into(),
                    text: "Passt. Ich teile gleich den Bildschirm.".into(),
                    eigen: true,
                },
            ],
            protokoll: vec![
                "Im Raum 482-913-770 - Server 1.4.2".into(),
                "Ton verbunden".into(),
                "Kamera: Integrated Webcam".into(),
                "Ton: 1204 raus / 3611 rein · Bild: 890 raus / 2670 rein".into(),
            ],
            stumm: false,
            kamera_an: true,
            schirm_an: mit_schirm,
            hand: false,
            steuer_frei: true,
            schirme: if mit_schirm {
                vec![(1, "Justin".to_string())]
            } else {
                Vec::new()
            },
            ungelesen: 3,
            tippen: vec!["Cem".into()],
            im_warteraum: false,
            warte_text: String::new(),
            cams: vec!["Integrated Webcam".into(), "Logitech StreamCam".into()],
            mics: vec!["Mikrofonarray".into(), "Yeti Nano".into()],
            spks: vec!["Lautsprecher (Realtek)".into(), "Kopfhoerer".into()],
            cam_sel: 1,
            mic_sel: 0,
            spk_sel: 0,
            kamera_name: "Integrated Webcam (1280x720 -> 640x360)".into(),
            ton_ein: "Mikrofonarray".into(),
            ton_aus: "Lautsprecher (Realtek)".into(),
        }
    };
    let zu = |seite: bool, reiter: Reiter, platz: Kameraplatz, voll: bool| Fensterzustand {
        seite_offen: seite,
        reiter,
        kameraplatz: platz,
        vollbild: voll,
        ..Default::default()
    };
    match nr {
        0 => (
            "allein",
            mach(1, false, false),
            zu(false, Reiter::Chat, Kameraplatz::Seite, false),
        ),
        1 => (
            "zu zweit",
            mach(2, false, false),
            zu(false, Reiter::Chat, Kameraplatz::Seite, false),
        ),
        2 => (
            "zu viert mit Chat",
            mach(4, false, false),
            zu(true, Reiter::Chat, Kameraplatz::Seite, false),
        ),
        3 => (
            "sechs Leute, Liste offen",
            mach(6, false, true),
            zu(true, Reiter::Leute, Kameraplatz::Seite, false),
        ),
        4 => (
            "Bildschirm geteilt, Kameras rechts",
            mach(4, true, false),
            zu(false, Reiter::Chat, Kameraplatz::Seite, false),
        ),
        5 => (
            "Bildschirm geteilt, Kameras unten",
            mach(4, true, false),
            zu(true, Reiter::Chat, Kameraplatz::Unten, false),
        ),
        6 => (
            "Bildschirm allein gross",
            mach(4, true, false),
            zu(false, Reiter::Chat, Kameraplatz::Aus, false),
        ),
        _ => (
            "Info-Reiter, schmales Fenster",
            mach(5, false, false),
            zu(true, Reiter::Info, Kameraplatz::Seite, false),
        ),
    }
}

/// Prueffbilder als Kamera- und Bildschirminhalt. Ein Schachbrett zeigt
/// sofort, ob zugeschnitten, gestreckt oder gespiegelt wird.
pub fn pruefbilder(ctx: &egui::Context) -> Bilder {
    let muster = |b: usize, h: usize, ton: u8| {
        let mut px = vec![0u8; b * h * 4];
        for y in 0..h {
            for x in 0..b {
                let i = (y * b + x) * 4;
                let k = if ((x / 32) + (y / 32)) % 2 == 0 { ton } else { 255 - ton };
                px[i] = k;
                px[i + 1] = ton;
                px[i + 2] = 255 - k;
                px[i + 3] = 255;
            }
        }
        egui::ColorImage::from_rgba_unmultiplied([b, h], &px)
    };
    let kam = ctx.load_texture("pruef_kam", muster(640, 360, 90), Default::default());
    let schirm = ctx.load_texture("pruef_schirm", muster(1920, 1080, 40), Default::default());
    let mut b = Bilder {
        eigen: Some(kam.clone()),
        ..Default::default()
    };
    for i in 1..=6u64 {
        b.kameras.insert(i, kam.clone());
    }
    b.schirme.insert(1, schirm);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kuerzel_nimmt_zwei_anfangsbuchstaben() {
        assert_eq!(kuerzel("Justin Fleischmann"), "JF");
        assert_eq!(kuerzel("anna"), "A");
        assert_eq!(kuerzel("   "), "?");
    }

    #[test]
    fn aufteilung_macht_kacheln_so_gross_wie_moeglich() {
        // 4 Leute auf einer breiten Flaeche: 2x2 ist besser als 4x1.
        let (spalten, breite) = beste_aufteilung(vec2(1200.0, 700.0), 4);
        assert_eq!(spalten, 2, "2x2 haette die groesseren Kacheln");
        assert!(breite > 300.0, "Kachel zu klein: {}", breite);
        // Einer allein bekommt hoechstens 70 % der Breite - sonst wirkt es
        // wie ein Fehler, nicht wie ein Meeting.
        let (_, allein) = beste_aufteilung(vec2(1200.0, 700.0), 1);
        assert!(allein <= 1200.0 * 0.7 + 0.5, "allein zu breit: {}", allein);
    }

    #[test]
    fn aufteilung_stuerzt_bei_null_nicht_ab() {
        let (s, b) = beste_aufteilung(vec2(800.0, 600.0), 0);
        assert_eq!(s, 1);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn kameraplatz_dreht_sich_im_kreis() {
        let mut k = Kameraplatz::Seite;
        k = k.weiter();
        assert_eq!(k, Kameraplatz::Unten);
        k = k.weiter();
        assert_eq!(k, Kameraplatz::Aus);
        k = k.weiter();
        assert_eq!(k, Kameraplatz::Seite);
    }

    #[test]
    fn jedes_beispiel_hat_leute_und_einen_namen() {
        for i in 0..BEISPIELE {
            let (name, sicht, _z) = beispiel(i);
            assert!(!name.is_empty(), "Beispiel {} ohne Namen", i);
            assert!(!sicht.leute.is_empty(), "Beispiel {} ohne Teilnehmer", i);
            assert!(sicht.leute[0].ich, "Beispiel {}: ich stehe nicht vorn", i);
        }
    }

    #[test]
    fn kurz_schneidet_nur_wenn_noetig() {
        assert_eq!(kurz("kurz", 10), "kurz");
        assert_eq!(kurz("abcdefghijkl", 5), "abcd…");
    }
}
