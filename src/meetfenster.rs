//! The meeting window - just like the FreeMeet browser client, only native.
//!
//! The model is meet.fleitec.com (web/index.html + web/style.css): at the top a
//! row of badges ("tags"), in the middle the stage with 16:9 tiles, on the right
//! a collapsible sidebar with chat/people/info, at the bottom the control bar
//! with round icon buttons. Before joining, the same centred card as in the
//! browser is shown: self-preview, level, two round buttons, device choice.
//!
//! WHY a separate module with its own data record (`Sicht`) instead of drawing
//! straight from NativMeet: this way EVERY view can be drawn without server,
//! camera or sound card. `--uitest` builds a view out of thin air and
//! renders join, empty room, full stage and screen share -
//! exactly the cases you otherwise never get to see before the customer
//! reports them. Incidentally it solves the borrow problem: drawing is done from
//! a copy, clicked wishes come back as `Aktion`.

use crate::icons;
use crate::theme::Palette;
use egui::{pos2, vec2, Color32, Rect, Vec2};
use std::collections::HashMap;

// ----------------------------------------------------------------- Daten

/// A participant, as the UI needs it.
#[derive(Clone, Default)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub is_muted: bool,
    pub kamera_aus: bool,
    pub hand: bool,
    pub gastgeber: bool,
    /// Not empty = this participant can be remote controlled with FreeViewer.
    pub fvid: String,
    pub ich: bool,
    /// Spricht gerade (Server-Hinweis oder eigener Pegel).
    pub spricht: bool,
}

/// Colour of a badge in the header.
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

/// What is the section attached to when zooming
/// into a shared screen?
///
/// WHY three kinds: when watching you sometimes want to move around yourself
/// ("Own": the window is a magnifier that follows your own pointer) and
/// sometimes see exactly what the other is pointing at ("Sharer"). Whoever wants
/// to pin a spot drags the image - then it stays fixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zoomanker {
    /// Magnifier: the spot under the own mouse pointer.
    Eigene,
    /// Follow the mouse pointer of whoever shares the screen.
    Teiler,
    /// Fester Ausschnitt (selbst hingezogen).
    Fest,
}

impl Zoomanker {
    pub fn wort(self) -> &'static str {
        match self {
            Zoomanker::Eigene => "eigene Maus",
            Zoomanker::Teiler => "Maus des Teilers",
            Zoomanker::Fest => "fest",
        }
    }
    pub fn weiter(self) -> Zoomanker {
        match self {
            Zoomanker::Eigene => Zoomanker::Teiler,
            Zoomanker::Teiler => Zoomanker::Fest,
            Zoomanker::Fest => Zoomanker::Eigene,
        }
    }
}

/// Where to put the cameras while a screen is shared?
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

/// A line in the chat. `von` = 0 means system message.
#[derive(Clone, Default)]
pub struct Chatzeile {
    pub von: u64,
    pub name: String,
    pub text: String,
    pub eigen: bool,
}

/// Everything the meeting window needs for drawing - copied once per frame from
/// the running meeting.
#[derive(Clone, Default)]
pub struct Sicht {
    pub raum: String,
    pub titel: String,
    pub gastgeber: bool,
    /// Text of the connection badge ("directly connected", "connecting ...").
    pub connection: String,
    pub connect_tone: Ton,
    pub e2e: bool,
    /// Bandwidth as finished text ("340 kbit/s"); empty = do not show.
    pub bandbreite: String,
    /// Everyone in the room, me first.
    pub leute: Vec<Person>,
    pub wartende: Vec<(u64, String)>,
    pub warteraum_an: bool,
    pub chat: Vec<Chatzeile>,
    /// Was im Reiter "Info" steht (Protokoll, Meldungen).
    pub protokoll: Vec<String>,
    pub is_muted: bool,
    pub kamera_an: bool,
    pub screen_on: bool,
    pub hand: bool,
    pub steuer_frei: bool,
    /// Offene Steuerungsanfragen AN MICH: (Participant, Name).
    pub steuer_anfragen: Vec<(u64, String)>,
    /// Whom I have allowed to control: (participant, name).
    pub steuer_erlaubt: Vec<(u64, String)>,
    /// My own request is still pending.
    pub steuer_gefragt: bool,
    /// Zusage bekommen: (Name des Teilenden, seine FreeViewer-Nummer).
    pub steuer_zusage: Option<(String, String)>,
    /// Is someone ELSE sharing? Only then is the request button worth it.
    pub foreign_screen: bool,
    /// Wer teilt gerade einen Display: (Participant, Name).
    pub schirme: Vec<(u64, String)>,
    /// Ungelesene Chatnachrichten (Zaehler am Chat-Knopf).
    pub ungelesen: u32,
    /// Wer tippt gerade (Namen).
    pub tippen: Vec<String>,
    /// We are still standing at the door - the host has to let us in.
    pub im_warteraum: bool,
    /// Text from the server for it ("The host has been notified.").
    pub warte_text: String,
    /// Devices that can be switched in the RUNNING meeting. The browser
    /// can do that too - without leaving the room.
    pub cams: Vec<String>,
    pub mics: Vec<String>,
    pub spks: Vec<String>,
    /// 0 = default device, otherwise index+1 in the respective list.
    pub cam_sel: usize,
    pub mic_sel: usize,
    pub spk_sel: usize,
    /// What is REALLY running right now (name from the driver) - for checking.
    pub kamera_name: String,
    pub ton_ein: String,
    pub ton_aus: String,
    /// Where is the mouse pointer of whoever is currently sharing the screen?
    /// (0..1 on THEIR screen). None = the sender does not report it.
    pub teiler_zeiger: Option<(f32, f32)>,
    /// Which part of their screen are they currently sending? (fractions 0..1)
    /// None or (0,0,1,1) = the whole thing. If it is smaller, the image is
    /// SHARP - then the magnifier must not zoom in once more.
    pub screen_region: Option<(f32, f32, f32, f32)>,
    /// The sender can deliver sections (new client). Only then is it worth
    /// offering "sharp" - otherwise the button would be a lie.
    pub scharf_moeglich: bool,
    /// The screens of this machine (name, width, height). With more than
    /// one the user is asked which one to share - the browser gets
    /// the choice from Windows for that, natively we have to ask ourselves.
    pub monitore: Vec<(String, u32, u32)>,
    /// Open program windows that can be shared individually:
    /// (id, title, program).
    pub fenster: Vec<(isize, String, String)>,
    /// Red frame around what is shared: on?
    pub rahmen_an: bool,
    /// Noise gate: on, strictness 0..100, currently letting sound through,
    /// estimated noise floor (0..1).
    pub sperre_an: bool,
    pub sperre_staerke: u8,
    pub sperre_offen: bool,
    pub sperre_grund: f32,
    /// Level of the own microphone (0..1) - for the level bar.
    pub eigener_pegel: f32,
    /// Avatar instead of camera image - on and what it looks like.
    pub avatar_an: bool,
    pub avatar: crate::avatar::Aussehen,
}

/// State that only concerns the UI (not the meeting).
#[derive(Clone)]
pub struct Fensterzustand {
    pub seite_offen: bool,
    pub reiter: Reiter,
    pub input_text: String,
    pub kameraplatz: Kameraplatz,
    pub vollbild: bool,
    pub pip: bool,
    /// What was last reported as "is typing". Without it a message would go out on
    /// EVERY keystroke - the server would get dozens of
    /// messages per word.
    pub tippt_gemeldet: bool,
    /// Einstellungen (Geraetewahl) sind aufgeklappt.
    pub settings_open: bool,
    /// The question "which screen?" is open.
    pub schirmwahl_offen: bool,
    /// Also show the OWN camera in picture-in-picture.
    pub pip_selbst: bool,
    /// The small window was opened AUTOMATICALLY when sharing - then
    /// it may also close by itself when finishing. A manually
    /// opened window stays instead.
    pub pip_auto: bool,
    /// The big window was minimised automatically when sharing.
    pub minimiert_auto: bool,
    /// Vergroesserung im geteilten Display (1.0 = alles).
    pub zoom: f32,
    pub zoomanker: Zoomanker,
    /// Centre of the section (0..1) - only by hand with Zoomanker::Fest.
    pub zoommitte: Vec2,
    /// The section drawn last (x, y, w, h) in screen
    /// coordinates of the sender. Reported upwards so that it can send
    /// exactly this part in full sharpness.
    pub zoomfenster: (f32, f32, f32, f32),
    /// Should the sender really send only this part (real pixels
    /// instead of extrapolated ones)? Costs it some CPU time, hence a
    /// switch - but on by default, because sharp is the point of it.
    pub scharf: bool,
}

impl Default for Fensterzustand {
    fn default() -> Self {
        Self {
            seite_offen: false,
            reiter: Reiter::Chat,
            input_text: String::new(),
            kameraplatz: Kameraplatz::Seite,
            vollbild: false,
            pip: false,
            tippt_gemeldet: false,
            settings_open: false,
            schirmwahl_offen: false,
            // Seeing yourself in the small window is the normal case - when
            // sharing the screen you want to check whether the camera is running.
            pip_selbst: true,
            pip_auto: false,
            minimiert_auto: false,
            zoom: 1.0,
            zoomanker: Zoomanker::Eigene,
            zoommitte: vec2(0.5, 0.5),
            zoomfenster: (0.0, 0.0, 1.0, 1.0),
            scharf: true,
        }
    }
}

/// The finished textures. Separate from `Sicht` because they cannot be copied
/// and come from the decoder for every frame anyway.
#[derive(Default)]
pub struct Bilder {
    pub eigen: Option<egui::TextureHandle>,
    pub kameras: HashMap<u64, egui::TextureHandle>,
    pub schirme: HashMap<u64, egui::TextureHandle>,
}

/// What the user clicked. It is executed by the caller - here
/// we only draw.
#[derive(Clone, PartialEq)]
pub enum Aktion {
    Muted(bool),
    Kamera(bool),
    Screen(bool),
    Hand(bool),
    Control(bool),
    Verlassen,
    Leave,
    MuteAll,
    Send(String),
    Tippt(bool),
    Einlassen(u64),
    Abweisen(u64),
    AlleEinlassen,
    Warteraum(bool),
    Stummschalten(u64),
    Rauswerfen(u64),
    /// Set up a real FreeViewer session to this participant.
    Steuern(String, String),
    EinladungKopieren,
    /// Geraet im laufenden Meeting wechseln (0 = Standardgeraet).
    KameraGeraet(usize),
    MikroGeraet(usize),
    LautsprecherGeraet(usize),
    /// Re-read the device list (after plugging/unplugging).
    ReloadDevices,
    /// Diesen Display teilen (Index aus `Sicht::monitore`).
    PickScreen(usize),
    /// Genau dieses Programmfenster teilen (Kennung aus `Sicht::fenster`).
    PickWindow(isize),
    /// Switch the red frame around what is shared on/off.
    Rahmen(bool),
    /// Noise gate: on/off and strictness (0..100).
    Rauschsperre(bool, u8),
    /// Avatar statt Kamera an/aus.
    Avatar(bool),
    /// Aussehen des Avatars aendern.
    AvatarAussehen(crate::avatar::Aussehen),
    /// As viewer, ask the sharer for control.
    RequestControl,
    /// As sharer, decide on a request.
    ControlReply(u64, bool),
    /// Erteilte Freigabe zuruecknehmen (None = alle).
    ControlGiveBack(Option<u64>),
}

/// The join screen needs write access (name, device choice).
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
    /// Join is already running - then lock the button.
    pub laeuft: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Beitrittsaktion {
    Beitreten,
    Back,
    MikroUm,
    KameraUm,
}

// ----------------------------------------------------------------- Farben

/// The palette plus the three signal colours FreeMeet uses. In light
/// themes they are taken darker, otherwise they would not be
/// readable on white - colour is a statement here, not decoration.
pub struct Farben {
    pub p: Palette,
    pub warn: Color32,
    pub bad: Color32,
    /// The very dark background of the tiles (#05070c in the browser).
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
    fn tone_color(&self, t: Ton) -> Color32 {
        match t {
            Ton::Neutral => self.p.muted,
            Ton::Gut => self.p.green,
            Ton::Warn => self.warn,
            Ton::Schlecht => self.bad,
        }
    }
}

/// Initials for the circle when no camera is running.
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

/// A badge ("tag") in the header: round pill field, icon + word.
fn tag(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: Option<&str>,
    text: &str,
    ton: Ton,
    klickbar: bool,
) -> egui::Response {
    let tint = f.tone_color(ton);
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, tint);
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
        _ => tint.gamma_multiply(0.10),
    };
    let rand = match ton {
        Ton::Neutral => {
            if hover {
                f.p.line.gamma_multiply(2.0)
            } else {
                f.p.line
            }
        }
        _ => tint.gamma_multiply(0.45),
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
        icons::image(s, 14.0, tint).paint_at(ui, r);
        x += 20.0;
    }
    ui.painter().galley(
        pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        tint,
    );
    if klickbar {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// State of a button in the control bar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ctl {
    /// unauffaellig an (Standard)
    Normal,
    /// bewusst eingeschaltet (Hand, Teilen, Chat) - Akzentfarbe
    An,
    /// Mic/camera switched off - that has to be red, not colourful
    Aus,
    /// Verlassen/Leave
    Gefahr,
}

/// A SMALL button: only the glyph, label as a hint on
/// hover. For the bar at the top right - every pixel counts there,
/// and the words underneath cost half the window height.
fn ctl_klein(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: &str,
    wort: &str,
    state: Ctl,
    ctr: Option<u32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::click());
    let (grund, rand, vorn) = match state {
        Ctl::Normal => (
            f.p.card_hi,
            f.p.line,
            if resp.hovered() { f.p.text } else { f.p.muted },
        ),
        Ctl::An => (
            f.p.accent.gamma_multiply(0.16),
            f.p.accent.gamma_multiply(0.45),
            f.p.accent,
        ),
        Ctl::Aus => (f.bad.gamma_multiply(0.16), f.bad.gamma_multiply(0.45), f.bad),
        Ctl::Gefahr => (f.bad.gamma_multiply(0.14), f.bad.gamma_multiply(0.32), f.bad),
    };
    let grund = if resp.hovered() {
        grund.gamma_multiply(1.35)
    } else {
        grund
    };
    let mal = ui.painter();
    mal.rect_filled(rect, 9.0, grund);
    mal.rect_stroke(rect, 9.0, egui::Stroke::new(1.0, rand), egui::StrokeKind::Inside);
    icons::image(symbol, 17.0, vorn)
        .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(17.0)));
    if let Some(n) = ctr {
        if n > 0 {
            let text = if n > 9 { "9+".to_string() } else { n.to_string() };
            let mitte = pos2(rect.right() - 5.0, rect.top() + 5.0);
            ui.painter().circle_filled(mitte, 7.0, f.bad);
            ui.painter().text(
                mitte,
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(9.0),
                Color32::from_rgb(0x2b, 0x05, 0x05),
            );
        }
    }
    resp.on_hover_text(wort)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A footer-bar button: glyph on top, word below - like Meet
/// and Teams. Exactly the shape the browser client shows.
fn ctl(
    ui: &mut egui::Ui,
    f: &Farben,
    symbol: &str,
    wort: &str,
    state: Ctl,
    ctr: Option<u32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 58.0), egui::Sense::click());
    let (grund, rand, vorn, wortfarbe) = match state {
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
    if let Some(n) = ctr {
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

/// Runder Knopf (Vorschau, Send) - 42 px wie im Browser.
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

/// Small button in lists ("Let in", "Control").
/// A small colour dot to click. The chosen one gets a ring -
/// without it nobody knows which one is currently active.
fn farbtupfer(ui: &mut egui::Ui, tint: [u8; 3], gewaehlt: bool, f: &Farben) -> bool {
    let (r, antwort) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
    let c = Color32::from_rgb(tint[0], tint[1], tint[2]);
    ui.painter().circle_filled(r.center(), 8.0, c);
    if gewaehlt {
        ui.painter()
            .circle_stroke(r.center(), 9.5, egui::Stroke::new(2.0, f.p.accent));
    } else if antwort.hovered() {
        ui.painter()
            .circle_stroke(r.center(), 9.5, egui::Stroke::new(1.0, f.p.line));
    }
    if antwort.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    antwort.clicked()
}

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

/// Paint an image into a rectangle. `ganz` = show completely (cut off
/// nothing), otherwise crop to fill. `spiegeln` only for the own
/// camera image - that is how you know yourself from the mirror.
fn bild_malen(
    mal: &egui::Painter,
    rect: Rect,
    tex: &egui::TextureHandle,
    spiegeln: bool,
    ganz: bool,
) {
    bild_malen_zoom(mal, rect, tex, spiegeln, ganz, None)
}

/// Like `bild_malen`, but with a section (0..1) - there to zoom into a
/// shared screen without recomputing the image.
fn bild_malen_zoom(
    mal: &egui::Painter,
    rect: Rect,
    tex: &egui::TextureHandle,
    spiegeln: bool,
    ganz: bool,
    ausschnitt: Option<Rect>,
) {
    let ar_bild = tex.aspect_ratio().max(0.05);
    let ar_rect = (rect.width() / rect.height().max(1.0)).max(0.05);
    let (ziel, mut uv) = if ganz {
        // Alles zeigen: hineinpassen, Rand bleibt Background.
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
        // Fill: the overhang is cut away via the image
        // coordinates so that no black bars appear.
        let uv = if ar_bild > ar_rect {
            let u = (ar_rect / ar_bild).clamp(0.0, 1.0);
            Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0))
        } else {
            let v = (ar_bild / ar_rect).clamp(0.0, 1.0);
            Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0))
        };
        (rect, uv)
    };
    // The zoom section is placed INTO the already computed area -
    // so the cropping for odd aspect ratios is preserved.
    if let Some(a) = ausschnitt {
        let (b, h) = (uv.width(), uv.height());
        uv = Rect::from_min_max(
            pos2(uv.min.x + a.min.x * b, uv.min.y + a.min.y * h),
            pos2(uv.min.x + a.max.x * b, uv.min.y + a.max.y * h),
        );
    }
    if spiegeln {
        uv = Rect::from_min_max(
            pos2(uv.max.x, uv.min.y),
            pos2(uv.min.x, uv.max.y),
        );
    }
    mal.image(tex.id(), ziel, uv, Color32::WHITE);
}

/// Which section of the image is visible at this magnification?
///
/// `mitte` is the point (0..1) being zoomed to. The result ALWAYS lies
/// completely within the image - otherwise you would see black stripes at the edge
/// or stretched pixels. That is exactly why the centre moves
/// inwards at the edge instead of letting the section stick out.
pub fn zoom_ausschnitt(zoom: f32, mitte: Vec2) -> Rect {
    let z = zoom.clamp(1.0, 8.0);
    let halb = 0.5 / z;
    let x = mitte.x.clamp(halb, 1.0 - halb);
    let y = mitte.y.clamp(halb, 1.0 - halb);
    Rect::from_min_max(pos2(x - halb, y - halb), pos2(x + halb, y + halb))
}

/// Which part of the RECEIVED image shows the wanted window?
///
/// `fenster` and `bereich` are both fractions of the sender's WHOLE screen.
/// If it sends only a section (`bereich` smaller than everything),
/// the incoming image contains exactly this section - the magnifier must then
/// not zoom in as far again, otherwise it would be doubled.
///
/// This very calculation makes the sharp zoom possible AND correct: as soon as
/// the sender delivers the wanted area, the result is the whole
/// image (0..1) - so real pixels instead of extrapolated ones.
pub fn zoom_uv(fenster: Rect, bereich: Rect) -> Rect {
    let bw = bereich.width().max(0.0001);
    let bh = bereich.height().max(0.0001);
    let x0 = ((fenster.min.x - bereich.min.x) / bw).clamp(0.0, 1.0);
    let y0 = ((fenster.min.y - bereich.min.y) / bh).clamp(0.0, 1.0);
    let x1 = ((fenster.max.x - bereich.min.x) / bw).clamp(0.0, 1.0);
    let y1 = ((fenster.max.y - bereich.min.y) / bh).clamp(0.0, 1.0);
    // Never collapse to zero - an empty area would be a black
    // image instead of a section.
    let x1 = x1.max(x0 + 0.001).min(1.0);
    let y1 = y1.max(y0 + 0.001).min(1.0);
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// Is the image cropped when we draw it to fill? If the
/// aspect ratio deviates by more than a fifth (portrait from a phone,
/// 4:3 webcam, ultrawide monitor), we prefer to show everything.
fn passt_nicht(tex: &egui::TextureHandle, rect: Rect) -> bool {
    let b = tex.aspect_ratio().max(0.05);
    let k = (rect.width() / rect.height().max(1.0)).max(0.05);
    (b - k).abs() / b.max(k) > 0.20
}

/// A screen tile gets exactly the shape of its image. Otherwise the
/// name tag would sit below wide black bars instead of at the image - and the
/// area would be wasted.
fn screen_box(rect: Rect, tex: Option<&egui::TextureHandle>) -> Rect {
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

/// A tile like in the browser.
pub struct Kachel {
    pub id: u64,
    pub name: String,
    pub is_muted: bool,
    pub hand: bool,
    pub spricht: bool,
    pub ich: bool,
    pub screen: bool,
    /// Camera switched off - then NEVER show an old still image, but
    /// the circle with the initials. A frozen face would be
    /// a lie about the state of the other person.
    pub kamera_aus: bool,
}

fn kachel_malen(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Kachel,
    tex: Option<&egui::TextureHandle>,
    f: &Farben,
) -> egui::Response {
    kachel_malen_zoom(ui, rect, k, tex, f, None)
}

fn kachel_malen_zoom(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Kachel,
    tex: Option<&egui::TextureHandle>,
    f: &Farben,
    ausschnitt: Option<Rect>,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        ui.id().with(("kachel", k.id, k.screen)),
        egui::Sense::click_and_drag(),
    );
    let mal = ui.painter_at(rect);
    mal.rect_filled(rect, 14.0, f.tief);
    match tex {
        Some(t) => {
            // A shared screen is NEVER cropped - cropped
            // text is worthless.
            let ganz = k.screen || passt_nicht(t, rect);
            bild_malen_zoom(&mal, rect, t, k.ich && !k.screen, ganz, ausschnitt);
        }
        None => {
            // No image: soft circle with the initials, as in the
            // browser. The gradient is built from a few circles.
            let r = rect.height().min(rect.width()) * 0.5;
            for i in 0..6 {
                let t = i as f32 / 6.0;
                mal.circle_filled(
                    rect.center(),
                    r * (1.0 - t) * 1.4,
                    f.p.card.gamma_multiply(0.16),
                );
            }
            let circle = (rect.height() * 0.30).clamp(16.0, 34.0);
            mal.circle_filled(rect.center(), circle, f.p.card_hi);
            mal.circle_stroke(rect.center(), circle, egui::Stroke::new(1.0, f.p.line));
            mal.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                kuerzel(&k.name),
                egui::FontId::proportional((circle * 0.72).clamp(11.0, 24.0)),
                f.p.text,
            );
            if k.kamera_aus && rect.height() > 120.0 {
                mal.text(
                    rect.center() + vec2(0.0, circle + 16.0),
                    egui::Align2::CENTER_CENTER,
                    "Kamera aus",
                    egui::FontId::proportional(12.5),
                    f.p.muted,
                );
            }
        }
    }
    // Name tag at the bottom left: name, plus mute and raised-hand icons.
    let font = egui::FontId::proportional(12.0);
    let schild_text = if k.screen {
        format!("{} · Bildschirm", k.name)
    } else {
        k.name.clone()
    };
    let galley = mal.layout_no_wrap(schild_text, font, f.p.text);
    let zeichen = (if k.is_muted { 1 } else { 0 } + if k.hand { 1 } else { 0 }) as f32;
    let width_px = (galley.size().x + 18.0 + zeichen * 19.0).min(rect.width() - 16.0);
    let schild = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.bottom() - 8.0 - 24.0),
        vec2(width_px, 24.0),
    );
    // Semi-transparent like in the browser (rgba(5,7,12,.68)) - the image can still
    // be guessed underneath, the text is readable anyway.
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
    if k.is_muted {
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
    // Border last, so that it lies above the image.
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

/// The screen before joining - centred card like `#vorbereiten`.
pub fn beitritt_ui(
    ctx: &egui::Context,
    b: &mut Beitritt,
    preview: Option<&egui::TextureHandle>,
) -> Option<Beitrittsaktion> {
    let f = farben();
    let mut aktion = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg))
        .show(ctx, |ui| {
            let width_px = ui.available_width().min(600.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(((ui.available_height() - 560.0) * 0.5).max(10.0));
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(width_px);
                        egui::Frame::NONE
                            .fill(f.p.card)
                            .stroke(egui::Stroke::new(1.0, f.p.line))
                            .corner_radius(16)
                            .inner_margin(egui::Margin::symmetric(20, 18))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                // The CARD is centred, its CONTENT is not -
                                // otherwise labels would float centred in
                                // nothing instead of sitting above their field.
                                ui.with_layout(
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        aktion = beitritt_karte(ui, &f, b, preview);
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
    preview: Option<&egui::TextureHandle>,
) -> Option<Beitrittsaktion> {
    let mut aktion = None;
    // Brand: glyph in a square + title, like ".marke klein" in the browser.
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

    // ---- Self-preview 16:9 with level bar and two round buttons
    let width_px = ui.available_width();
    let height_px = (width_px * 9.0 / 16.0).clamp(120.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width_px, height_px), egui::Sense::hover());
    {
        let mal = ui.painter_at(rect);
        mal.rect_filled(rect, 14.0, f.tief);
        match (b.kamera_an, preview) {
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
    if !b.kamera_an || preview.is_none() {
        icons::image("cam-off", 34.0, f.p.muted.gamma_multiply(0.6)).paint_at(
            ui,
            Rect::from_center_size(rect.center() - vec2(0.0, 8.0), Vec2::splat(34.0)),
        );
    }
    // Level bar at the top - you see at once whether the microphone hears anything.
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
    // The two round buttons sit IN the image, bottom centre.
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
        let b_width = (ui.available_width() - 10.0) * 0.62;
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
        .min_size(vec2(b_width, 44.0));
        if ui.add_enabled(!b.laeuft, gross).clicked() {
            aktion = Some(Beitrittsaktion::Beitreten);
        }
        ui.add_space(10.0);
        let go_back = egui::Button::image_and_text(
            icons::image("back", 16.0, f.p.text),
            egui::RichText::new("Zurück").size(14.0).color(f.p.text),
        )
        .fill(f.p.card_hi)
        .stroke(egui::Stroke::new(1.0, f.p.line))
        .corner_radius(11)
        .min_size(vec2(ui.available_width(), 44.0));
        if ui.add(go_back).clicked() {
            aktion = Some(Beitrittsaktion::Back);
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

fn geraete_wahl(ui: &mut egui::Ui, id: &str, width_px: f32, namen: &[String], wahl: &mut usize) {
    let jetzt = if *wahl == 0 {
        "Standardgerät".to_string()
    } else {
        namen
            .get(*wahl - 1)
            .cloned()
            .unwrap_or_else(|| "Standardgerät".to_string())
    };
    egui::ComboBox::from_id_salt(id)
        .width(width_px - 8.0)
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

/// We are in the waiting room. The browser shows the same centred card here
/// with three breathing dots - without them you would sit in front of an empty room and
/// not know whether something is broken or you simply have to wait.
pub fn warteschirm_ui(ctx: &egui::Context, titel: &str, text: &str) -> bool {
    let f = farben();
    let mut cancel = false;
    // The dots breathe - that needs a repaint.
    // 20 ms instead of 60. In this tick EVERYTHING in the meeting hangs: picking up the
    // microphone, encoding the camera image, audio to the speaker. With 60 ms
    // at most 16 frames/s were possible, and the microphone delivered
    // its 20 ms frames in bundles of three instead of singly.
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
    let t = ctx.input(|i| i.time) as f32;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg))
        .show(ctx, |ui| {
            let width_px = ui.available_width().min(420.0);
            ui.add_space((ui.available_height() * 0.5 - 120.0).max(12.0));
            ui.vertical_centered(|ui| {
                ui.set_max_width(width_px);
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
                                cancel = true;
                            }
                        });
                    });
            });
        });
    cancel
}

/// The running meeting: header, stage, sidebar, control bar.
pub fn meeting_ui(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    bilder: &Bilder,
) -> Vec<Aktion> {
    let f = farben();
    let mut aktionen: Vec<Aktion> = Vec::new();

    // The control buttons now sit small at the top right instead of as a wide
    // bar at the bottom - that cost almost a tenth of the window height, and in the
    // meeting every pixel counts for the image.
    kopf(ctx, s, z, &f, &mut aktionen);
    if z.seite_offen {
        seite(ctx, s, z, &f, &mut aktionen);
    }
    if !s.wartende.is_empty() {
        lobby_leiste(ctx, s, z, &f, &mut aktionen);
    }
    if !s.steuer_anfragen.is_empty() {
        steuer_leiste(ctx, s, &f, &mut aktionen);
    }
    if z.settings_open {
        settings(ctx, s, z, &f, &mut aktionen);
    }
    if z.schirmwahl_offen {
        schirmwahl(ctx, s, z, &f, &mut aktionen);
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            buehne(ui, s, z, bilder, &f);
        });
    aktionen
}

fn kopf(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
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
                    &s.connection,
                    s.connect_tone,
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
                    ui.spacing_mut().item_spacing.x = 5.0;
                    // Built from RIGHT to left: hang-up at the very
                    // outside, where you look for it.
                    if ctl_klein(ui, f, "leave", "Meeting verlassen", Ctl::Gefahr, None).clicked() {
                        aktionen.push(Aktion::Verlassen);
                    }
                    if s.gastgeber {
                        if ctl_klein(ui, f, "end", "Meeting für ALLE beenden", Ctl::Gefahr, None)
                            .clicked()
                        {
                            aktionen.push(Aktion::Leave);
                        }
                        if ctl_klein(ui, f, "muteall", "Alle stummschalten", Ctl::Normal, None)
                            .clicked()
                        {
                            aktionen.push(Aktion::MuteAll);
                        }
                    }
                    if ctl_klein(
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
                    if ctl_klein(
                        ui,
                        f,
                        "settings",
                        "Kamera, Mikrofon und Lautsprecher umstellen",
                        if z.settings_open { Ctl::An } else { Ctl::Normal },
                        None,
                    )
                    .clicked()
                    {
                        z.settings_open = !z.settings_open;
                    }
                    if ctl_klein(
                        ui,
                        f,
                        "hand",
                        "Hand heben",
                        if s.hand { Ctl::An } else { Ctl::Normal },
                        None,
                    )
                    .clicked()
                    {
                        aktionen.push(Aktion::Hand(!s.hand));
                    }
                    if !s.schirme.is_empty() {
                        if ctl_klein(
                            ui,
                            f,
                            if z.vollbild { "shrink" } else { "full" },
                            "Geteilten Bildschirm groß auf die ganze Fläche",
                            if z.vollbild { Ctl::An } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        {
                            z.vollbild = !z.vollbild;
                        }
                        if ctl_klein(
                            ui,
                            f,
                            "layout",
                            z.kameraplatz.wort(),
                            if z.kameraplatz == Kameraplatz::Aus { Ctl::Aus } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        {
                            z.kameraplatz = z.kameraplatz.weiter();
                        }
                    }
                    if ctl_klein(
                        ui,
                        f,
                        if z.pip { "pip" } else { "pip-off" },
                        "Kleines Fenster, das oben bleibt",
                        if z.pip { Ctl::An } else { Ctl::Normal },
                        None,
                    )
                    .clicked()
                    {
                        z.pip = !z.pip;
                    }
                    // Control exists ONLY on request. The old button
                    // "release for everyone" is gone: it unlocked the whole room,
                    // although almost always exactly one person was
                    // meant. Now whoever watches asks - and whoever shares
                    // decides for exactly this one person.
                    if let Some((wer, fvid)) = s.steuer_zusage.clone() {
                        if ctl_klein(
                            ui,
                            f,
                            "keyboard",
                            &format!("{} jetzt steuern (FreeViewer {})", wer, fvid),
                            Ctl::An,
                            None,
                        )
                        .clicked()
                        {
                            aktionen.push(Aktion::Steuern(fvid, wer));
                        }
                    } else if s.foreign_screen
                        && ctl_klein(
                            ui,
                            f,
                            "gamepad",
                            if s.steuer_gefragt {
                                "Anfrage läuft – warte auf Antwort"
                            } else {
                                "Steuerung des geteilten Bildschirms anfragen"
                            },
                            if s.steuer_gefragt { Ctl::An } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        && !s.steuer_gefragt
                    {
                        aktionen.push(Aktion::RequestControl);
                    }
                    if !s.steuer_erlaubt.is_empty() {
                        let namen: Vec<&str> =
                            s.steuer_erlaubt.iter().map(|(_, n)| n.as_str()).collect();
                        if ctl_klein(
                            ui,
                            f,
                            "shield",
                            &format!("{} darf steuern – Freigabe zurücknehmen", namen.join(", ")),
                            Ctl::Gefahr,
                            Some(s.steuer_erlaubt.len() as u32),
                        )
                        .clicked()
                        {
                            aktionen.push(Aktion::ControlGiveBack(None));
                        }
                    }
                    if ctl_klein(
                        ui,
                        f,
                        if s.screen_on { "screen-off" } else { "screen" },
                        if s.screen_on { "Teilen beenden" } else { "Bildschirm teilen" },
                        if s.screen_on { Ctl::An } else { Ctl::Normal },
                        None,
                    )
                    .clicked()
                    {
                        if s.screen_on {
                            aktionen.push(Aktion::Screen(false));
                        } else if s.monitore.len() > 1 || !s.fenster.is_empty() {
                            // As soon as there is something to choose, we ask.
                            // Earlier, with a single screen, EVERYTHING went out
                            // unasked - exactly what Justin did not want
                            // anymore.
                            z.schirmwahl_offen = true;
                        } else {
                            aktionen.push(Aktion::PickScreen(0));
                        }
                    }
                    if ctl_klein(
                        ui,
                        f,
                        if s.kamera_an { "cam" } else { "cam-off" },
                        if s.kamera_an { "Kamera aus" } else { "Kamera an" },
                        if s.kamera_an { Ctl::Normal } else { Ctl::Aus },
                        None,
                    )
                    .clicked()
                    {
                        aktionen.push(Aktion::Kamera(!s.kamera_an));
                    }
                    if ctl_klein(
                        ui,
                        f,
                        if s.is_muted { "mic-off" } else { "mic" },
                        if s.is_muted { "Stummschaltung aufheben" } else { "Stummschalten" },
                        if s.is_muted { Ctl::Aus } else { Ctl::Normal },
                        None,
                    )
                    .clicked()
                    {
                        aktionen.push(Aktion::Muted(!s.is_muted));
                    }
                    ui.add_space(6.0);
                    // Only icon and number - the word "participants" tells
                    // nobody anything new and eats space in the bar.
                    tag(ui, f, Some("people"), &s.leute.len().to_string(), Ton::Neutral, false)
                        .on_hover_text("Wer gerade im Raum ist");
                });
            });
        });
}

/// Which screen to share? Only needed when there are several.
fn schirmwahl(
    ctx: &egui::Context,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    let mut offen = true;
    egui::Window::new("Was möchtest du teilen?")
        .open(&mut offen)
        .collapsible(false)
        .resizable(false)
        .default_width(380.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -20.0))
        .frame(
            egui::Frame::NONE
                .fill(f.p.card)
                .stroke(egui::Stroke::new(1.0, f.p.line))
                .corner_radius(12.0)
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
            ui.label(
                egui::RichText::new("GANZER BILDSCHIRM")
                    .size(10.5)
                    .color(f.p.muted),
            );
            for (i, (name, b, h)) in s.monitore.iter().enumerate() {
                let text = format!("{}  ·  {}×{}", kurz(name, 26), b, h);
                if mini(ui, f, &text, i == 0).clicked() {
                    aktionen.push(Aktion::PickScreen(i));
                    z.schirmwahl_offen = false;
                }
            }
            if !s.fenster.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("NUR EIN PROGRAMM")
                        .size(10.5)
                        .color(f.p.muted),
                );
                ui.label(
                    egui::RichText::new(
                        "Es geht wirklich nur dieses Fenster raus – alles andere bleibt privat.",
                    )
                    .size(10.5)
                    .color(f.p.muted),
                );
                // Long lists would otherwise burst the screen: with many
                // open windows we scroll instead of cutting off.
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (k, titel, prog) in s.fenster.iter() {
                            let text = if prog.is_empty() {
                                kurz(titel, 40)
                            } else {
                                format!("{}  ·  {}", kurz(titel, 32), kurz(prog, 14))
                            };
                            if mini(ui, f, &text, false).clicked() {
                                aktionen.push(Aktion::PickWindow(*k));
                                z.schirmwahl_offen = false;
                            }
                        }
                    });
            }
            ui.add_space(8.0);
            let mut rahmen = s.rahmen_an;
            if ui
                .checkbox(&mut rahmen, "Rot umranden, was geteilt wird")
                .changed()
            {
                aktionen.push(Aktion::Rahmen(rahmen));
            }
            ui.add_space(4.0);
            if mini(ui, f, "Abbrechen", false).clicked() {
                z.schirmwahl_offen = false;
            }
        });
    if !offen {
        z.schirmwahl_offen = false;
    }
}

/// Settings IN THE MIDDLE of the meeting: camera, microphone, speaker.
///
/// WHY as a separate window and not in the sidebar: the choice is
/// a short interruption, afterwards you want it gone again. The
/// browser client does it the same way (overlay above the stage).
fn settings(
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
            let width_px = 300.0;

            feld_beschriftung(ui, f, Some("cam"), "Kamera");
            let mut cam = s.cam_sel;
            geraete_wahl(ui, "set_cam", width_px, &s.cams, &mut cam);
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
            geraete_wahl(ui, "set_mic", width_px, &s.mics, &mut mic);
            if mic != s.mic_sel {
                aktionen.push(Aktion::MikroGeraet(mic));
            }
            ui.add_space(6.0);

            feld_beschriftung(ui, f, Some("sound"), "Lautsprecher");
            let mut spk = s.spk_sel;
            geraete_wahl(ui, "set_spk", width_px, &s.spks, &mut spk);
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

            // ---------------------------------------------------- Avatar
            feld_beschriftung(ui, f, Some("user"), "Avatar statt Kamera");
            let mut av = s.avatar_an;
            if ui
                .checkbox(&mut av, "Gezeichnetes Gesicht senden (Mund folgt der Stimme)")
                .changed()
            {
                aktionen.push(Aktion::Avatar(av));
            }
            if s.avatar_an {
                let mut a = s.avatar;
                let mut geaendert = false;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Haut").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::HAUTTOENE.len() as u8 {
                        if farbtupfer(ui, crate::avatar::HAUTTOENE[i as usize], a.haut == i, f) {
                            a.haut = i;
                            geaendert = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Haare").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::HAARFARBEN.len() as u8 {
                        if farbtupfer(ui, crate::avatar::HAARFARBEN[i as usize], a.haar == i, f) {
                            a.haar = i;
                            geaendert = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Frisur").size(11.0).color(f.p.muted));
                    for (i, name) in crate::avatar::HAARFORMEN.iter().enumerate() {
                        if mini(ui, f, name, a.form == i as u8).clicked() {
                            a.form = i as u8;
                            geaendert = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Augen").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::AUGENFARBEN.len() as u8 {
                        if farbtupfer(ui, crate::avatar::AUGENFARBEN[i as usize], a.augen == i, f) {
                            a.augen = i;
                            geaendert = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Hintergrund").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::GRUNDFARBEN.len() as u8 {
                        if farbtupfer(ui, crate::avatar::GRUNDFARBEN[i as usize], a.grund == i, f) {
                            a.grund = i;
                            geaendert = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let mut brille = a.brille;
                    if ui.checkbox(&mut brille, "Brille").changed() {
                        a.brille = brille;
                        geaendert = true;
                    }
                    let mut bart = a.bart;
                    if ui.checkbox(&mut bart, "Bart").changed() {
                        a.bart = bart;
                        geaendert = true;
                    }
                });
                if geaendert {
                    aktionen.push(Aktion::AvatarAussehen(a));
                }
                ui.label(
                    egui::RichText::new(
                        "Die Lippen folgen der Lautstärke, nicht dem Wortlaut - mehr kann ohne Modell niemand.",
                    )
                    .size(10.0)
                    .color(f.p.muted),
                );
            }
            ui.add_space(10.0);

            // ---------------------------------------------- Rauschsperre
            feld_beschriftung(ui, f, Some("shield"), "Rauschsperre");
            let mut sperre = s.sperre_an;
            if ui
                .checkbox(&mut sperre, "Leises Rauschen nicht übertragen")
                .changed()
            {
                aktionen.push(Aktion::Rauschsperre(sperre, s.sperre_staerke));
            }
            if s.sperre_an {
                let mut st = s.sperre_staerke as f32;
                let regler = ui.add(
                    egui::Slider::new(&mut st, 0.0..=100.0)
                        .show_value(false)
                        .text("weniger streng  ↔  strenger"),
                );
                if regler.changed() {
                    aktionen.push(Aktion::Rauschsperre(true, st.round() as u8));
                }
                // Without feedback the slider would be flying blind: here
                // you see whether sound is currently let through or blocked
                // and what the gate has calibrated to.
                let (r, _) = ui.allocate_exact_size(vec2(width_px, 10.0), egui::Sense::hover());
                let mal = ui.painter();
                mal.rect_filled(r, 5.0, f.p.card_hi);
                let pegel = s.eigener_pegel.clamp(0.0, 1.0).sqrt();
                let voll = Rect::from_min_size(r.min, vec2(r.width() * pegel, r.height()));
                mal.rect_filled(
                    voll,
                    5.0,
                    if s.sperre_offen { f.p.accent } else { f.p.muted },
                );
                // Where the threshold roughly lies (noise floor times
                // factor), as a thin line.
                let s_anteil = (s.sperre_grund
                    * (2.0 + s.sperre_staerke as f32 / 100.0 * 6.0))
                    .clamp(0.0, 1.0)
                    .sqrt();
                let x = r.left() + r.width() * s_anteil;
                mal.line_segment(
                    [pos2(x, r.top()), pos2(x, r.bottom())],
                    egui::Stroke::new(1.5, f.p.accent),
                );
                ui.label(
                    egui::RichText::new(if s.sperre_offen {
                        "geht raus"
                    } else {
                        "gesperrt – es geht gerade nichts raus"
                    })
                    .size(10.0)
                    .color(if s.sperre_offen { f.p.accent } else { f.p.muted }),
                );
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if mini(ui, f, "Geräte neu suchen", false).clicked() {
                    aktionen.push(Aktion::ReloadDevices);
                }
                if mini(ui, f, "Fertig", true).clicked() {
                    z.settings_open = false;
                }
            });
            // Stay honest: switching the audio devices resets the
            // echo cancellation, you hear that briefly.
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
        z.settings_open = false;
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
    let width_px = (ctx.screen_rect().width() * 0.45).min(340.0).max(220.0);
    egui::SidePanel::right("meet_seite")
        .resizable(false)
        .exact_width(width_px)
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
    width_px: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width_px.max(40.0), 44.0), egui::Sense::click());
    let tint = if aktiv {
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
    icons::image(symbol, 18.0, tint).paint_at(
        ui,
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 14.0), Vec2::splat(18.0)),
    );
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_BOTTOM,
        wort,
        egui::FontId::proportional(11.0),
        tint,
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
    let height_px = ui.available_height();
    let input_height = 52.0;
    egui::ScrollArea::vertical()
        .id_salt("meet_chat_log")
        .max_height(height_px - input_height)
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
    // Eingabezeile unten - rundes Feld plus runder Send-Knopf.
    ui.painter().hline(
        ui.max_rect().left()..=ui.max_rect().right(),
        ui.cursor().top(),
        egui::Stroke::new(1.0, f.p.line),
    );
    ui.add_space(7.0);
    ui.horizontal(|ui| {
        ui.add_space(9.0);
        let feld = ui.add(
            egui::TextEdit::singleline(&mut z.input_text)
                .desired_width((ui.available_width() - 58.0).max(60.0))
                .hint_text("Nachricht …")
                .margin(egui::Margin::symmetric(11, 7)),
        );
        let enter = feld.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let tippt = !z.input_text.trim().is_empty();
        if tippt != z.tippt_gemeldet {
            z.tippt_gemeldet = tippt;
            aktionen.push(Aktion::Tippt(tippt));
        }
        let send_out = rund(ui, f, "send", 34.0, false, true)
            .on_hover_text("Senden")
            .clicked();
        if (enter || send_out) && !z.input_text.trim().is_empty() {
            aktionen.push(Aktion::Send(z.input_text.trim().to_string()));
            z.input_text.clear();
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
        // For own messages pull everything to the right - otherwise the
        // name would stick to the left edge and slide out of the bar.
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
                    // Right-to-left: whatever comes first sits at the far right.
                    // Buttons first, then the icons - so the mute
                    // and raised-hand icons stick to the name and not to the window edge.
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
                    if p.is_muted {
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

fn symbol_klein(ui: &mut egui::Ui, name: &str, tint: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(15.0), egui::Sense::hover());
    icons::image(name, 15.0, tint).paint_at(ui, r);
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
                    // FIRST place the buttons on the right, THEN put the name in the
                    // remainder and cut it off there. The other way round
                    // a long name would push the buttons out of the bar.
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

/// The bar that reports that someone is waiting at the door.
/// The question to the SHARER: "X wants to control your screen".
///
/// Why a separate bar and not a window: it must not cover the image,
/// but must catch the eye at once - just like #ctrlLeiste in the
/// browser. It stays until a decision is made; a request that
/// disappears by itself would be worse than none.
fn steuer_leiste(
    ctx: &egui::Context,
    s: &Sicht,
    f: &Farben,
    aktionen: &mut Vec<Aktion>,
) {
    // Below the waiting-room bar, otherwise the two overlap.
    let oben = if s.wartende.is_empty() { 56.0 } else { 100.0 };
    egui::Area::new(egui::Id::new("meet_steuerfrage"))
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, oben))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (id, name) in s.steuer_anfragen.iter() {
                egui::Frame::NONE
                    .fill(f.p.card_hi)
                    .stroke(egui::Stroke::new(1.0, f.p.accent))
                    .corner_radius(20)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            symbol_klein(ui, "gamepad", f.p.accent);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} möchte deinen Bildschirm steuern",
                                    name
                                ))
                                .size(12.5)
                                .color(f.p.text),
                            );
                            if mini(ui, f, "Erlauben", true).clicked() {
                                aktionen.push(Aktion::ControlReply(*id, true));
                            }
                            if mini(ui, f, "Ablehnen", false).clicked() {
                                aktionen.push(Aktion::ControlReply(*id, false));
                            }
                        });
                    });
                ui.add_space(6.0);
            }
        });
}

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
            is_muted: false,
            hand: false,
            spricht: false,
            ich: s.leute.iter().any(|p| p.id == *id && p.ich),
            screen: true,
            kamera_aus: false,
        })
        .collect();
    let kameras = s
        .leute
        .iter()
        .map(|p| Kachel {
            id: p.id,
            // The own tile is called "Du" - just like in the browser. In the
            // participant list the real name is shown, where it is useful.
            name: if p.ich { "Du".to_string() } else { p.name.clone() },
            is_muted: p.is_muted,
            hand: p.hand,
            spricht: p.spricht && !p.is_muted,
            ich: p.ich,
            screen: false,
            kamera_aus: p.kamera_aus,
        })
        .collect();
    (schirme, kameras)
}

fn tex_fuer<'a>(k: &Kachel, b: &'a Bilder) -> Option<&'a egui::TextureHandle> {
    if k.kamera_aus {
        return None;
    }
    if k.screen {
        b.schirme.get(&k.id)
    } else if k.ich {
        b.eigen.as_ref()
    } else {
        b.kameras.get(&k.id)
    }
}

/// Draw the shared screen - with magnifier.
///
/// Wheel = larger/smaller, dragging = move the section (and thereby set it to
/// "fixed"), double click = back to whole. At the top right sits a
/// small bar with the same in button form so that you also find it.
fn screen_tile(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Kachel,
    tex: Option<&egui::TextureHandle>,
    s: &Sicht,
    z: &mut Fensterzustand,
    f: &Farben,
) {
    // --- Wheel: zoom in and out ---
    let zeiger = ui.ctx().pointer_latest_pos();
    let ueber = zeiger.map(|p| rect.contains(p)).unwrap_or(false);
    if ueber {
        let rad = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if rad.abs() > 0.5 {
            // Before the first wheel click the anchor is on "own mouse" -
            // then the wheel acts like a magnifier under the pointer.
            z.zoom = (z.zoom * (1.0 + rad * 0.0025)).clamp(1.0, 8.0);
            if z.zoom <= 1.001 {
                z.zoom = 1.0;
            }
        }
    }

    // Which part of their screen is the other currently sending? All
    // following calculations run in THEIR screen coordinates - only this
    // way the magnifier is right even when they already deliver a section.
    let ber = s
        .screen_region
        .map(|(x, y, w, h)| Rect::from_min_size(pos2(x, y), vec2(w, h)))
        .unwrap_or(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));

    // --- Where does the magnifier look? ---
    let mut mitte = z.zoommitte;
    if z.zoom > 1.0 {
        match z.zoomanker {
            Zoomanker::Eigene => {
                // The pointer IN the tile area is mapped onto the screen of the
                // other: pointing at top left means seeing top left.
                // No trailing, no drifting - a real magnifier.
                if let Some(p) = zeiger {
                    if rect.contains(p) {
                        let fx = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
                        let fy = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                        mitte = vec2(
                            ber.min.x + fx * ber.width(),
                            ber.min.y + fy * ber.height(),
                        );
                        z.zoommitte = mitte;
                    }
                }
            }
            Zoomanker::Teiler => {
                if let Some((x, y)) = s.teiler_zeiger {
                    mitte = vec2(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
                    z.zoommitte = mitte;
                }
            }
            Zoomanker::Fest => {}
        }
    }
    // The wanted window - ALWAYS in screen coordinates of the sender.
    // It is also reported upwards so that the sender can send exactly this part
    // sharp.
    let fenster = zoom_ausschnitt(z.zoom, mitte);
    z.zoomfenster = (fenster.min.x, fenster.min.y, fenster.width(), fenster.height());
    let ausschnitt = if z.zoom > 1.0 || s.screen_region.is_some() {
        Some(zoom_uv(fenster, ber))
    } else {
        None
    };

    let antwort = kachel_malen_zoom(ui, rect, k, tex, f, ausschnitt);

    // --- Dragging moves the section (and holds it) ---
    if z.zoom > 1.0 && antwort.dragged() {
        let d = antwort.drag_delta();
        if d.length_sq() > 0.0 {
            z.zoomanker = Zoomanker::Fest;
            z.zoommitte = vec2(
                (z.zoommitte.x - d.x / rect.width().max(1.0) * fenster.width()).clamp(0.0, 1.0),
                (z.zoommitte.y - d.y / rect.height().max(1.0) * fenster.height()).clamp(0.0, 1.0),
            );
        }
    }
    if antwort.double_clicked() {
        z.zoom = 1.0;
        z.zoomanker = Zoomanker::Eigene;
        z.zoommitte = vec2(0.5, 0.5);
    }
    if z.zoom > 1.0 && ueber {
        ui.ctx().set_cursor_icon(if antwort.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }

    zoomleiste(ui, rect, s, z, f);
}

/// Kleine Leiste oben rechts im geteilten Bild: -, Stufe, +, Anker, go_back.
fn zoomleiste(ui: &mut egui::Ui, rect: Rect, s: &Sicht, z: &mut Fensterzustand, f: &Farben) {
    if rect.width() < 260.0 || rect.height() < 120.0 {
        return; // in einem Briefmarkenbild waere die Leiste nur im Weg
    }
    let anker_text = match z.zoomanker {
        Zoomanker::Teiler if s.teiler_zeiger.is_none() => "Maus des Teilers (kommt nicht an)",
        a => a.wort(),
    };
    let stufe = format!("{:.0} %", z.zoom * 100.0);
    let h = 28.0;
    let mal = ui.painter_at(rect);
    // Roughly estimate the width from the texts - accurate enough for a bar.
    let width_px = 30.0 + 54.0 + 30.0 + 14.0 + anker_text.chars().count() as f32 * 6.4 + 16.0
        + if z.zoom > 1.0 { 62.0 + 110.0 } else { 0.0 };
    let leiste = Rect::from_min_size(
        pos2(rect.right() - 10.0 - width_px.min(rect.width() - 20.0), rect.top() + 10.0),
        vec2(width_px.min(rect.width() - 20.0), h),
    );
    mal.rect_filled(
        leiste,
        9.0,
        if f.p.dark {
            Color32::from_black_alpha(185)
        } else {
            Color32::from_white_alpha(215)
        },
    );
    mal.rect_stroke(leiste, 9.0, egui::Stroke::new(1.0, f.p.line), egui::StrokeKind::Inside);

    let mut x = leiste.left() + 5.0;
    let mut knopf = |ui: &mut egui::Ui, x: &mut f32, text: &str, w: f32, tip: &str| -> bool {
        let r = Rect::from_min_size(pos2(*x, leiste.top() + 3.0), vec2(w, h - 6.0));
        let a = ui
            .interact(r, ui.id().with(("zoomknopf", text.to_string())), egui::Sense::click())
            .on_hover_text(tip);
        if a.hovered() {
            ui.painter_at(rect).rect_filled(r, 7.0, f.p.card_hi);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter_at(rect).text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(12.0),
            f.p.text,
        );
        *x += w;
        a.clicked()
    };
    if knopf(ui, &mut x, "−", 26.0, "Kleiner (Mausrad geht auch)") {
        z.zoom = (z.zoom / 1.25).max(1.0);
        if z.zoom <= 1.001 {
            z.zoom = 1.0;
        }
    }
    let _ = knopf(ui, &mut x, &stufe, 50.0, "Vergroesserung");
    if knopf(ui, &mut x, "+", 26.0, "Groesser (Mausrad geht auch)") {
        z.zoom = (z.zoom * 1.25).min(8.0);
    }
    x += 6.0;
    if knopf(
        ui,
        &mut x,
        anker_text,
        anker_text.chars().count() as f32 * 6.4 + 14.0,
        "Woran haengt der Ausschnitt? Klicken zum Umschalten.",
    ) {
        z.zoomanker = z.zoomanker.weiter();
    }
    if z.zoom > 1.0 {
        // Show honestly WHAT is arriving: real pixels or
        // extrapolated ones. That is the whole difference.
        let (text, tip) = if !s.scharf_moeglich {
            (
                "unscharf",
                "Der Sender ist ein aelterer Stand und kann keinen Ausschnitt liefern - vergroessert wird nur gerechnet.",
            )
        } else if !z.scharf {
            ("nur vergroessert", "Klicken: den Sender bitten, diesen Ausschnitt scharf zu schicken")
        } else if s.screen_region.map(|(_, _, w, _)| w < 0.95).unwrap_or(false) {
            ("scharf", "Der Sender schickt genau diesen Ausschnitt - echte Bildpunkte. Klicken zum Abschalten.")
        } else {
            ("scharf (kommt)", "Angefragt - der Sender stellt gerade um")
        };
        if knopf(ui, &mut x, text, text.chars().count() as f32 * 6.4 + 14.0, tip)
            && s.scharf_moeglich
        {
            z.scharf = !z.scharf;
        }
        if knopf(ui, &mut x, "ganz", 50.0, "Zurueck auf das ganze Bild") {
            z.zoom = 1.0;
            z.zoomanker = Zoomanker::Eigene;
            z.zoommitte = vec2(0.5, 0.5);
        }
    }
}

fn buehne(ui: &mut egui::Ui, s: &Sicht, z: &mut Fensterzustand, b: &Bilder, f: &Farben) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover());
    if area.width() < 40.0 || area.height() < 40.0 {
        return;
    }
    let (schirme, kameras) = kacheln_bauen(s);

    if schirme.is_empty() {
        z.vollbild = false;
        raster(ui, area, &kameras, b, f);
        return;
    }
    // A shared screen is THE content: large and complete.
    let haupt = schirme
        .iter()
        .position(|k| !k.ich)
        .unwrap_or(0);
    if z.vollbild {
        let t = tex_fuer(&schirme[haupt], b);
        let r = screen_box(area.shrink(6.0), t);
        screen_tile(ui, r, &schirme[haupt], t, s, z, f);
        return;
    }
    let lueck = 10.0;
    let rand = 10.0;
    let innen = area.shrink(rand);
    let (screen_area, streifen) = match z.kameraplatz {
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
    let h = (screen_area.height() - lueck * (n as f32 - 1.0)) / n as f32;
    for (i, k) in schirme.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(screen_area.left(), screen_area.top() + i as f32 * (h + lueck)),
            vec2(screen_area.width(), h),
        );
        if r.height() > 20.0 {
            let t = tex_fuer(k, b);
            // Only the MAIN share gets the magnifier. With several
            // shares at once a common zoom state would be
            // confusing - you would not know which image it means.
            if i == haupt {
                screen_tile(ui, screen_box(r, t), k, t, s, z, f);
            } else {
                kachel_malen(ui, screen_box(r, t), k, t, f);
            }
        }
    }
    // Camera strip. The height is chosen so that all fit in -
    // otherwise the last camera hangs below the footer bar.
    if let Some(st) = streifen {
        match z.kameraplatz {
            Kameraplatz::Seite => {
                let k = kameras.len().max(1) as f32;
                let voll = st.width() * 9.0 / 16.0;
                let height_px =
                    ((st.height() - lueck * (k - 1.0)) / k).min(voll).max(48.0);
                let width_px = (height_px * 16.0 / 9.0).min(st.width());
                for (i, kk) in kameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(
                            st.center().x - width_px / 2.0,
                            st.top() + i as f32 * (height_px + lueck),
                        ),
                        vec2(width_px, height_px),
                    );
                    if r.bottom() > st.bottom() + 2.0 {
                        break;
                    }
                    kachel_malen(ui, r, kk, tex_fuer(kk, b), f);
                }
            }
            Kameraplatz::Unten => {
                let height_px = st.height();
                let width_px = height_px * 16.0 / 9.0;
                for (i, kk) in kameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(st.left() + i as f32 * (width_px + lueck), st.top()),
                        vec2(width_px, height_px),
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

/// How many columns? Zoom and Meet try every column count and
/// take the one where the single tile becomes LARGEST - with a fixed
/// 16:9, so that no faces are cropped. Exactly that here.
pub fn beste_aufteilung(area: Vec2, item_count: usize) -> (usize, f32) {
    if item_count == 0 {
        return (1, 0.0);
    }
    let lueck = 10.0;
    let rand = 20.0;
    let platz_b = area.x - rand;
    let platz_h = area.y - rand;
    // Alone in the meeting the own tile should not fill the whole area.
    let grenze = if item_count == 1 {
        (area.x * 0.7).min(880.0)
    } else {
        f32::INFINITY
    };
    let (mut beste_spalten, mut best_width) = (1usize, 0.0f32);
    for s in 1..=item_count {
        let zeilen = item_count.div_ceil(s) as f32;
        let from_width = (platz_b - lueck * (s as f32 - 1.0)) / s as f32;
        let from_height = ((platz_h - lueck * (zeilen - 1.0)) / zeilen) * (16.0 / 9.0);
        let width_px = from_width.min(from_height).min(grenze);
        if width_px > best_width {
            best_width = width_px;
            beste_spalten = s;
        }
    }
    (beste_spalten, best_width.max(80.0))
}

fn raster(ui: &mut egui::Ui, area: Rect, kacheln: &[Kachel], b: &Bilder, f: &Farben) {
    if kacheln.is_empty() {
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            "Noch niemand im Raum.",
            egui::FontId::proportional(14.0),
            f.p.muted,
        );
        return;
    }
    let lueck = 10.0;
    let (spalten, width_px) = beste_aufteilung(area.size(), kacheln.len());
    let height_px = width_px * 9.0 / 16.0;
    let zeilen = kacheln.len().div_ceil(spalten);
    let gesamt_h = zeilen as f32 * height_px + (zeilen as f32 - 1.0) * lueck;
    let mut y = area.center().y - gesamt_h / 2.0;
    let mut i = 0usize;
    for _ in 0..zeilen {
        let in_zeile = spalten.min(kacheln.len() - i);
        let gesamt_b = in_zeile as f32 * width_px + (in_zeile as f32 - 1.0) * lueck;
        let mut x = area.center().x - gesamt_b / 2.0;
        for _ in 0..in_zeile {
            let r = Rect::from_min_size(pos2(x, y), vec2(width_px, height_px));
            kachel_malen(ui, r, &kacheln[i], tex_fuer(&kacheln[i], b), f);
            x += width_px + lueck;
            i += 1;
        }
        y += height_px + lueck;
    }
}

/// The small picture-in-picture window: the cameras of the others while you
/// are sharing the screen yourself and the big window is covered.
/// The small window that stays on top. It shows the others AND - if
/// wanted - the own camera, plus a narrow bar with pure
/// icon buttons.
///
/// WHY buttons here: while you share your own screen, the big
/// meeting window lies behind the shared application. Without buttons in the
/// small window you could no longer reach mute and camera
/// without interrupting the share.
pub fn pip_inhalt(ui: &mut egui::Ui, s: &Sicht, b: &Bilder, selbst: &mut bool) -> Vec<Aktion> {
    let f = farben();
    let mut aktionen: Vec<Aktion> = Vec::new();
    let (_, kameras) = kacheln_bauen(s);
    // First the others, the own tile last (like in the browser).
    let mut zeigen: Vec<&Kachel> = kameras.iter().filter(|k| !k.ich).collect();
    if *selbst {
        if let Some(ich) = kameras.iter().find(|k| k.ich) {
            zeigen.push(ich);
        }
    }
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover());
    ui.painter().rect_filled(area, 0.0, f.p.bg);

    // --- Bar at the bottom: icons only, so that it fits in 260 px width ---
    let leiste_h = 34.0;
    let leiste = Rect::from_min_size(
        pos2(area.left(), area.bottom() - leiste_h),
        vec2(area.width(), leiste_h),
    );
    ui.painter().rect_filled(leiste, 0.0, f.p.card);
    let knoepfe: [(&str, bool, &str); 4] = [
        (
            if s.is_muted { "mic-off" } else { "mic" },
            s.is_muted,
            if s.is_muted { "Stummschaltung aufheben" } else { "Stummschalten" },
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
        // The last button is hang-up - always red, otherwise you hit the wrong one.
        let grund = if i == 3 {
            f.bad
        } else if *aus {
            dunkler(f.p.card_hi, 0.9)
        } else {
            f.p.card_hi
        };
        let grund = if antwort.hovered() { dunkler(grund, 1.25) } else { grund };
        ui.painter().circle_filled(r.center(), gr * 0.5, grund);
        let tint = if i == 3 {
            Color32::WHITE
        } else if *aus {
            f.bad
        } else {
            f.p.text
        };
        let ir = Rect::from_center_size(r.center(), Vec2::splat(gr * 0.58));
        icons::image(name, gr * 0.58, tint).paint_at(ui, ir);
        if antwort.clicked() {
            match i {
                0 => aktionen.push(Aktion::Muted(!s.is_muted)),
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

    // --- Notices at the bottom right, above the bar ---
    //
    // WHY here and not in the big window: whoever shares has minimised the big
    // window (otherwise they would share themselves). Without these notices they
    // would receive neither chat messages nor control or
    // join requests - the request would go nowhere.
    //
    // IMPORTANT: they sit in the MINI window, not above the shared image.
    // Otherwise half the room would read the private chat message.
    let mut hinweise: Vec<(String, Option<(Aktion, Aktion)>)> = Vec::new();
    for (id, name) in s.steuer_anfragen.iter() {
        hinweise.push((
            format!("{} möchte steuern", kurz(name, 18)),
            Some((
                Aktion::ControlReply(*id, true),
                Aktion::ControlReply(*id, false),
            )),
        ));
    }
    for (id, name) in s.wartende.iter() {
        hinweise.push((
            format!("{} wartet vor der Tür", kurz(name, 16)),
            Some((Aktion::Einlassen(*id), Aktion::Abweisen(*id))),
        ));
    }
    if s.ungelesen > 0 {
        if let Some(z) = s.chat.iter().rev().find(|z| !z.eigen) {
            hinweise.push((
                format!("{}: {}", kurz(&z.name, 12), kurz(&z.text, 26)),
                None,
            ));
        }
    }
    // At most three - more would wall in the small window.
    hinweise.truncate(3);
    let mut hint_height = 0.0;
    if !hinweise.is_empty() {
        let zeile = 46.0;
        hint_height = hinweise.len() as f32 * zeile + 6.0;
        let bereich = Rect::from_min_max(
            pos2(area.left() + 6.0, leiste.top() - hint_height),
            pos2(area.right() - 6.0, leiste.top() - 4.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(bereich)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                for (text, knoepfe) in hinweise.iter() {
                    egui::Frame::NONE
                        .fill(f.p.card_hi)
                        .stroke(egui::Stroke::new(1.0, f.p.accent))
                        .corner_radius(9)
                        .inner_margin(egui::Margin::symmetric(7, 4))
                        .show(ui, |ui| {
                            ui.set_width(bereich.width() - 14.0);
                            ui.label(
                                egui::RichText::new(text).size(10.5).color(f.p.text),
                            );
                            if let Some((ja, nein)) = knoepfe {
                                ui.horizontal(|ui| {
                                    if mini(ui, &f, "Erlauben", true).clicked() {
                                        aktionen.push(ja.clone());
                                    }
                                    if mini(ui, &f, "Ablehnen", false).clicked() {
                                        aktionen.push(nein.clone());
                                    }
                                });
                            }
                        });
                }
            },
        );
    }

    // --- Kacheln darueber ---
    let oben = Rect::from_min_max(
        area.min,
        pos2(area.right(), leiste.top() - hint_height),
    );
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
    let height_px = ((oben.height() - lueck * (n as f32 + 1.0)) / n as f32).max(40.0);
    for (i, k) in zeigen.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(oben.left() + lueck, oben.top() + lueck + i as f32 * (height_px + lueck)),
            vec2(oben.width() - 2.0 * lueck, height_px),
        );
        if r.bottom() > oben.bottom() {
            break;
        }
        kachel_malen(ui, r, k, tex_fuer(k, b), &f);
    }
    aktionen
}


// --------------------------------------------------- Beispiele zum Pruefen

/// How many example states there are.
pub const BEISPIELE: usize = 12;

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

/// An example state: (name, view, window state).
///
/// WHY this lives here and not in the test: `--uitest` renders all
/// views headless with it AND `--meetdemo` makes real images from it. Both
/// thereby see guaranteed the same thing - otherwise you end up checking a different
/// window than the one the customer gets.
pub fn beispiel(nr: usize) -> (&'static str, Sicht, Fensterzustand) {
    let namen = ["Justin", "Anna Berger", "Cem", "Dora", "Emil", "Frida"];
    let mach = |item_count: usize, with_screen: bool, warteraum: bool| {
        let mut leute = Vec::new();
        for i in 0..item_count {
            leute.push(Person {
                id: i as u64 + 1,
                name: namen[i % namen.len()].to_string(),
                is_muted: i % 3 == 1,
                kamera_aus: i % 4 == 2,
                hand: i % 5 == 3,
                gastgeber: i == 0,
                fvid: if i == 1 { "497628420".into() } else { String::new() },
                ich: i == 0,
                spricht: i == 1,
            });
        }
        Sicht {
            fenster: Vec::new(),
            rahmen_an: true,
            sperre_an: true,
            sperre_staerke: 45,
            sperre_offen: true,
            sperre_grund: 0.004,
            eigener_pegel: 0.2,
            avatar_an: false,
            avatar: crate::avatar::Aussehen::default(),
            steuer_anfragen: Vec::new(),
            steuer_erlaubt: Vec::new(),
            steuer_gefragt: false,
            steuer_zusage: None,
            foreign_screen: false,
            raum: "482-913-770".into(),
            titel: "Wochenbesprechung".into(),
            gastgeber: true,
            connection: "direkt verbunden".into(),
            connect_tone: Ton::Gut,
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
            is_muted: false,
            kamera_an: true,
            screen_on: with_screen,
            hand: false,
            steuer_frei: true,
            schirme: if with_screen {
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
            teiler_zeiger: if with_screen { Some((0.62, 0.41)) } else { None },
            screen_region: None,
            scharf_moeglich: with_screen,
            monitore: vec![
                ("Surface Panel".to_string(), 2496, 1664),
                ("Generic PnP Monitor".to_string(), 2160, 1440),
            ],
            // (Example 10 sets the area further below by hand.)
        }
    };
    let zu = |seite: bool, reiter: Reiter, platz: Kameraplatz, voll: bool| Fensterzustand {
        seite_offen: seite,
        reiter,
        kameraplatz: platz,
        vollbild: voll,
        ..Default::default()
    };
    // The settings screen needs its own example state, otherwise it
    // would never be drawn in --uitest and an error in it would only be
    // noticed by the user.
    let with_settings = || Fensterzustand {
        settings_open: true,
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
        8 => (
            "Einstellungen offen",
            mach(3, false, false),
            with_settings(),
        ),
        // The zoom in the shared screen - otherwise the magnifier together with
        // its bar would never be drawn and an error in it would only be
        // noticed by the viewer.
        9 => (
            "Bildschirm hineingezoomt",
            mach(3, true, false),
            Fensterzustand {
                zoom: 2.5,
                zoomanker: Zoomanker::Teiler,
                zoommitte: vec2(0.62, 0.41),
                ..Default::default()
            },
        ),
        11 => (
            "Bildschirmauswahl offen",
            mach(2, false, false),
            Fensterzustand {
                schirmwahl_offen: true,
                ..Default::default()
            },
        ),
        // The SHARP zoom: the sender already sends exactly this section.
        // Then the incoming image must be shown COMPLETELY - if zoomed
        // once more here, it would be doubled.
        10 => (
            "Bildschirm scharf gezoomt",
            {
                let mut v = mach(3, true, false);
                v.screen_region = Some((0.47, 0.26, 0.4, 0.4));
                v
            },
            Fensterzustand {
                zoom: 2.5,
                zoomanker: Zoomanker::Fest,
                zoommitte: vec2(0.67, 0.46),
                ..Default::default()
            },
        ),
        _ => (
            "Info-Reiter, schmales Fenster",
            mach(5, false, false),
            zu(true, Reiter::Info, Kameraplatz::Seite, false),
        ),
    }
}

/// Test images as camera and screen content. A chessboard shows
/// at once whether it is cropped, stretched or mirrored.
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
    let screen = ctx.load_texture("pruef_schirm", muster(1920, 1080, 40), Default::default());
    let mut b = Bilder {
        eigen: Some(kam.clone()),
        ..Default::default()
    };
    for i in 1..=6u64 {
        b.kameras.insert(i, kam.clone());
    }
    b.schirme.insert(1, screen);
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
        // 4 people on a wide area: 2x2 is better than 4x1.
        let (spalten, width_px) = beste_aufteilung(vec2(1200.0, 700.0), 4);
        assert_eq!(spalten, 2, "2x2 haette die groesseren Kacheln");
        assert!(width_px > 300.0, "Kachel zu klein: {}", width_px);
        // One alone gets at most 70 % of the width - otherwise it looks
        // like an error, not like a meeting.
        let (_, allein) = beste_aufteilung(vec2(1200.0, 700.0), 1);
        assert!(allein <= 1200.0 * 0.7 + 0.5, "allein zu breit: {}", allein);
    }

    #[test]
    fn aufteilung_stuerzt_bei_null_nicht_ab() {
        let (s, b) = beste_aufteilung(vec2(800.0, 600.0), 0);
        assert_eq!(s, 1);
        assert_eq!(b, 0.0);
    }

    /// The section must ALWAYS lie completely within the image - otherwise you see
    /// black stripes at the edge or stretched pixels.
    #[test]
    fn zoom_ausschnitt_bleibt_im_bild() {
        for zoom in [1.0f32, 1.5, 2.0, 3.7, 8.0] {
            for (mx, my) in [(0.0f32, 0.0f32), (1.0, 1.0), (0.5, 0.5), (-3.0, 9.0)] {
                let a = zoom_ausschnitt(zoom, vec2(mx, my));
                assert!(a.min.x >= -0.0001 && a.min.y >= -0.0001, "links/oben raus: {:?}", a);
                assert!(a.max.x <= 1.0001 && a.max.y <= 1.0001, "rechts/unten raus: {:?}", a);
                let erwartet = 1.0 / zoom.clamp(1.0, 8.0);
                assert!(
                    (a.width() - erwartet).abs() < 0.001,
                    "Breite {} statt {}",
                    a.width(),
                    erwartet
                );
            }
        }
    }

    /// If the other already sends exactly the wanted window, the
    /// incoming image is to be shown COMPLETELY - then they are real pixels.
    #[test]
    fn scharfer_zoom_zeigt_das_ganze_bild() {
        let fenster = Rect::from_min_size(pos2(0.25, 0.25), vec2(0.5, 0.5));
        let uv = zoom_uv(fenster, fenster);
        assert!((uv.min.x).abs() < 1e-5 && (uv.min.y).abs() < 1e-5);
        assert!((uv.max.x - 1.0).abs() < 1e-5 && (uv.max.y - 1.0).abs() < 1e-5);
    }

    /// If it sends everything, the magnifier behaves as before.
    #[test]
    fn ohne_ausschnitt_bleibt_die_lupe_wie_gehabt() {
        let ganz = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let fenster = Rect::from_min_size(pos2(0.25, 0.25), vec2(0.5, 0.5));
        assert_eq!(zoom_uv(fenster, ganz), fenster);
    }

    /// Halfway: it sends more than wanted - then the rest is
    /// cut away, but NO piece is zoomed twice.
    #[test]
    fn groesserer_ausschnitt_wird_richtig_verrechnet() {
        let bereich = Rect::from_min_size(pos2(0.2, 0.2), vec2(0.6, 0.6));
        let fenster = Rect::from_min_size(pos2(0.35, 0.35), vec2(0.3, 0.3));
        let uv = zoom_uv(fenster, bereich);
        assert!((uv.min.x - 0.25).abs() < 1e-5, "links falsch: {}", uv.min.x);
        assert!((uv.width() - 0.5).abs() < 1e-5, "Breite falsch: {}", uv.width());
    }

    /// If the viewer drags faster than the sender switches, nothing may
    /// run out of the image - better to stay at the edge.
    #[test]
    fn window_outside_stays_in_frame() {
        let bereich = Rect::from_min_size(pos2(0.5, 0.5), vec2(0.2, 0.2));
        let fenster = Rect::from_min_size(pos2(0.0, 0.0), vec2(0.2, 0.2));
        let uv = zoom_uv(fenster, bereich);
        assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0);
        assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
        assert!(uv.width() > 0.0 && uv.height() > 0.0, "leere Flaeche waere schwarz");
    }

    #[test]
    fn zoom_eins_zeigt_alles() {
        let a = zoom_ausschnitt(1.0, vec2(0.2, 0.9));
        assert_eq!(a, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    }

    /// Double magnification in the middle = exactly the middle quarter.
    #[test]
    fn zoom_zwei_nimmt_die_mitte() {
        let a = zoom_ausschnitt(2.0, vec2(0.5, 0.5));
        assert_eq!(a, Rect::from_min_max(pos2(0.25, 0.25), pos2(0.75, 0.75)));
    }

    #[test]
    fn zoom_anchor_rotates_in_circle() {
        let mut a = Zoomanker::Eigene;
        for _ in 0..3 {
            a = a.weiter();
        }
        assert_eq!(a, Zoomanker::Eigene);
        assert!(!Zoomanker::Teiler.wort().is_empty());
    }

    #[test]
    fn camera_slot_rotates_in_circle() {
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
