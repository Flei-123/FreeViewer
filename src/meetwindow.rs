//! The meeting window - just like the FreeMeet browser client, only native.
//!
//! The model is meet.fleitec.com (web/index.html + web/style.css): at the top a
//! row of badges ("tags"), in the middle the stage with 16:9 tiles, on the right
//! a collapsible sidebar with chat/people/info, at the bottom the control bar
//! with round icon buttons. Before joining, the same centred card as in the
//! browser is shown: self-preview, level, two round buttons, device choice.
//!
//! WHY a separate module with its own data record (`MeetView`) instead of drawing
//! straight from NativMeet: this way EVERY view can be drawn without server,
//! camera or sound card. `--uitest` builds a view out of thin air and
//! renders join, empty room, full stage and screen share -
//! exactly the cases you otherwise never get to see before the customer
//! reports them. Incidentally it solves the borrow problem: drawing is done from
//! a copy, clicked wishes come back as `Action`.

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
    pub camera_from: bool,
    pub hand: bool,
    pub is_host: bool,
    /// Not empty = this participant can be remote controlled with FreeViewer.
    pub fvid: String,
    pub ich: bool,
    /// Spricht gerade (Server-Hinweis oder eigener Pegel).
    pub speaking: bool,
}

/// Colour of a badge in the header.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Ton {
    #[default]
    Neutral,
    Gut,
    Warn,
    Bad,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Chat,
    People,
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
pub enum ZoomAnchor {
    /// Magnifier: the spot under the own mouse pointer.
    Own,
    /// Follow the mouse pointer of whoever shares the screen.
    Divider,
    /// Fester Ausschnitt (selbst hingezogen).
    Fest,
}

impl ZoomAnchor {
    pub fn word(self) -> &'static str {
        match self {
            ZoomAnchor::Own => "eigene Maus",
            ZoomAnchor::Divider => "Maus des Teilers",
            ZoomAnchor::Fest => "fest",
        }
    }
    pub fn advance(self) -> ZoomAnchor {
        match self {
            ZoomAnchor::Own => ZoomAnchor::Divider,
            ZoomAnchor::Divider => ZoomAnchor::Fest,
            ZoomAnchor::Fest => ZoomAnchor::Own,
        }
    }
}

/// Where to put the cameras while a screen is shared?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraSlot {
    Page,
    Bottom,
    Aus,
}

impl CameraSlot {
    fn advance(self) -> CameraSlot {
        match self {
            CameraSlot::Page => CameraSlot::Bottom,
            CameraSlot::Bottom => CameraSlot::Aus,
            CameraSlot::Aus => CameraSlot::Page,
        }
    }
    fn word(self) -> &'static str {
        match self {
            CameraSlot::Page => "Kameras rechts",
            CameraSlot::Bottom => "Kameras unten",
            CameraSlot::Aus => "Kameras aus",
        }
    }
}

/// A line in the chat. `von` = 0 means system message.
#[derive(Clone, Default)]
pub struct ChatLine {
    pub von: u64,
    pub name: String,
    pub text: String,
    pub own: bool,
}

/// Everything the meeting window needs for drawing - copied once per frame from
/// the running meeting.
#[derive(Clone, Default)]
pub struct MeetView {
    pub room: String,
    pub heading: String,
    pub is_host: bool,
    /// Text of the connection badge ("directly connected", "connecting ...").
    pub connection: String,
    pub connect_tone: Ton,
    pub e2e: bool,
    /// Bandwidth as finished text ("340 kbit/s"); empty = do not show.
    pub bandwidth: String,
    /// Everyone in the room, me first.
    pub people: Vec<Person>,
    pub waiting: Vec<(u64, String)>,
    pub waiting_room_on: bool,
    pub chat: Vec<ChatLine>,
    /// Was im Reiter "Info" steht (Protokoll, Meldungen).
    pub protocol: Vec<String>,
    pub is_muted: bool,
    pub camera_on: bool,
    pub screen_on: bool,
    pub hand: bool,
    pub control_free: bool,
    /// Offene Steuerungsanfragen AN MICH: (Participant, Name).
    pub control_request: Vec<(u64, String)>,
    /// Whom I have allowed to control: (participant, name).
    pub control_allowed: Vec<(u64, String)>,
    /// My own request is still pending.
    pub control_asked: bool,
    /// Zusage bekommen: (Name des Teilenden, seine FreeViewer-Nummer).
    pub control_grant: Option<(String, String)>,
    /// Is someone ELSE sharing? Only then is the request button worth it.
    pub foreign_screen: bool,
    /// Wer teilt gerade einen Display: (Participant, Name).
    pub screens: Vec<(u64, String)>,
    /// Ungelesene Chatnachrichten (Zaehler am Chat-Knopf).
    pub unread: u32,
    /// Wer tippt gerade (Namen).
    pub type_text: Vec<String>,
    /// We are still standing at the door - the host has to let us in.
    pub in_waiting_room: bool,
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
    pub camera_name: String,
    pub ton_ein: String,
    pub ton_aus: String,
    /// Where is the mouse pointer of whoever is currently sharing the screen?
    /// (0..1 on THEIR screen). None = the sender does not report it.
    pub divider_pointer: Option<(f32, f32)>,
    /// Which part of their screen are they currently sending? (fractions 0..1)
    /// None or (0,0,1,1) = the whole thing. If it is smaller, the image is
    /// SHARP - then the magnifier must not zoom in once more.
    pub screen_region: Option<(f32, f32, f32, f32)>,
    /// The sender can deliver sections (new client). Only then is it worth
    /// offering "sharp" - otherwise the button would be a lie.
    pub sharp_possible: bool,
    /// The screens of this machine (name, width, height). With more than
    /// one the user is asked which one to share - the browser gets
    /// the choice from Windows for that, natively we have to ask ourselves.
    pub monitore: Vec<(String, u32, u32)>,
    /// Open program windows that can be shared individually:
    /// (id, title, program).
    pub window: Vec<(isize, String, String)>,
    /// Red frame around what is shared: on?
    pub frame_on: bool,
    /// Noise gate: on, strictness 0..100, currently letting sound through,
    /// estimated noise floor (0..1).
    pub lock_on: bool,
    pub lock_strength: u8,
    pub lock_open: bool,
    pub lock_reason: f32,
    /// Level of the own microphone (0..1) - for the level bar.
    pub own_level: f32,
    /// Avatar instead of camera image - on and what it looks like.
    pub avatar_an: bool,
    pub avatar: crate::avatar::Look,
}

/// State that only concerns the UI (not the meeting).
#[derive(Clone)]
pub struct MeetWindowState {
    pub page_open: bool,
    pub tab: Tab,
    pub input_text: String,
    pub camera_slot: CameraSlot,
    pub fullscreen: bool,
    pub pip: bool,
    /// What was last reported as "is typing". Without it a message would go out on
    /// EVERY keystroke - the server would get dozens of
    /// messages per word.
    pub types_reported: bool,
    /// Einstellungen (Geraetewahl) sind aufgeklappt.
    pub settings_open: bool,
    /// The question "which screen?" is open.
    pub screen_pick_open: bool,
    /// Also show the OWN camera in picture-in-picture.
    pub pip_self: bool,
    /// The small window was opened AUTOMATICALLY when sharing - then
    /// it may also close by itself when finishing. A manually
    /// opened window stays instead.
    pub pip_auto: bool,
    /// The big window was minimised automatically when sharing.
    pub minimized_auto: bool,
    /// Vergroesserung im geteilten Display (1.0 = alles).
    pub zoom: f32,
    pub zoom_anchor: ZoomAnchor,
    /// Centre of the section (0..1) - only by hand with Zoomanker::Fest.
    pub zoom_center: Vec2,
    /// The section drawn last (x, y, w, h) in screen
    /// coordinates of the sender. Reported upwards so that it can send
    /// exactly this part in full sharpness.
    pub zoom_window: (f32, f32, f32, f32),
    /// Should the sender really send only this part (real pixels
    /// instead of extrapolated ones)? Costs it some CPU time, hence a
    /// switch - but on by default, because sharp is the point of it.
    pub sharp: bool,
}

impl Default for MeetWindowState {
    fn default() -> Self {
        Self {
            page_open: false,
            tab: Tab::Chat,
            input_text: String::new(),
            camera_slot: CameraSlot::Page,
            fullscreen: false,
            pip: false,
            types_reported: false,
            settings_open: false,
            screen_pick_open: false,
            // Seeing yourself in the small window is the normal case - when
            // sharing the screen you want to check whether the camera is running.
            pip_self: true,
            pip_auto: false,
            minimized_auto: false,
            zoom: 1.0,
            zoom_anchor: ZoomAnchor::Own,
            zoom_center: vec2(0.5, 0.5),
            zoom_window: (0.0, 0.0, 1.0, 1.0),
            sharp: true,
        }
    }
}

/// The finished textures. Separate from `MeetView` because they cannot be copied
/// and come from the decoder for every frame anyway.
#[derive(Default)]
pub struct Images {
    pub own: Option<egui::TextureHandle>,
    pub cameras: HashMap<u64, egui::TextureHandle>,
    pub screens: HashMap<u64, egui::TextureHandle>,
}

/// What the user clicked. It is executed by the caller - here
/// we only draw.
#[derive(Clone, PartialEq)]
pub enum Action {
    Muted(bool),
    WebCam(bool),
    Screen(bool),
    Hand(bool),
    Control(bool),
    LeaveMeet,
    Leave,
    MuteAll,
    Send(String),
    Types(bool),
    Admit(u64),
    Refuse(u64),
    AllAdmit,
    WaitingRoom(bool),
    Mute(u64),
    Kick(u64),
    /// Set up a real FreeViewer session to this participant.
    AskControl(String, String),
    InviteCopy,
    /// Geraet im laufenden Meeting wechseln (0 = Standardgeraet).
    CameraDevice(usize),
    MicDevice(usize),
    SpeakerDevice(usize),
    /// Re-read the device list (after plugging/unplugging).
    ReloadDevices,
    /// Diesen Display teilen (Index aus `MeetView::monitore`).
    PickScreen(usize),
    /// Share exactly this program window (id from `MeetView::window`).
    PickWindow(isize),
    /// Switch the red frame around what is shared on/off.
    FrameBox(bool),
    /// Noise gate: on/off and strictness (0..100).
    NoiseGate(bool, u8),
    /// Avatar statt Kamera an/aus.
    Avatar(bool),
    /// Aussehen des Avatars aendern.
    AvatarLook(crate::avatar::Look),
    /// As viewer, ask the sharer for control.
    RequestControl,
    /// As sharer, decide on a request.
    ControlReply(u64, bool),
    /// Erteilte Freigabe zuruecknehmen (None = alle).
    ControlGiveBack(Option<u64>),
}

/// The join screen needs write access (name, device choice).
#[derive(Clone, Default)]
pub struct Join {
    pub room: String,
    pub heading: String,
    pub name: String,
    pub mics: Vec<String>,
    pub cams: Vec<String>,
    pub mic_sel: usize,
    pub cam_sel: usize,
    pub mic_on: bool,
    pub camera_on: bool,
    pub devices_there: bool,
    pub level: f32,
    pub hint_msg: String,
    /// Join is already running - then lock the button.
    pub running: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JoinAction {
    Join,
    Back,
    MicAround,
    CameraAround,
}

// ----------------------------------------------------------------- Farben

/// The palette plus the three signal colours FreeMeet uses. In light
/// themes they are taken darker, otherwise they would not be
/// readable on white - colour is a statement here, not decoration.
pub struct Colors {
    pub p: Palette,
    pub warn: Color32,
    pub bad: Color32,
    /// The very dark background of the tiles (#05070c in the browser).
    pub deep: Color32,
}

fn darker(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

pub fn colors() -> Colors {
    let p = crate::theme::palette();
    Colors {
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
        deep: if p.dark {
            darker(p.bg, 0.62)
        } else {
            Color32::from_rgb(0xe8, 0xec, 0xf4)
        },
        p,
    }
}

impl Colors {
    fn tone_color(&self, t: Ton) -> Color32 {
        match t {
            Ton::Neutral => self.p.muted,
            Ton::Gut => self.p.green,
            Ton::Warn => self.warn,
            Ton::Bad => self.bad,
        }
    }
}

/// Initials for the circle when no camera is running.
pub fn initials(name: &str) -> String {
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
    f: &Colors,
    symbol: Option<&str>,
    text: &str,
    ton: Ton,
    clickable: bool,
) -> egui::Response {
    let tint = f.tone_color(ton);
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, tint);
    let sym_b = if symbol.is_some() { 14.0 + 6.0 } else { 0.0 };
    let dimension = vec2(galley.size().x + sym_b + 22.0, 25.0);
    let sinn = if clickable {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(dimension, sinn);
    let hover = clickable && resp.hovered();
    let reason = match ton {
        Ton::Neutral => {
            if hover {
                f.p.card_hi
            } else {
                f.deep
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
    mal.rect_filled(rect, 12.5, reason);
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
    if clickable {
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
    Danger,
}

/// A SMALL button: only the glyph, label as a hint on
/// hover. For the bar at the top right - every pixel counts there,
/// and the words underneath cost half the window height.
fn ctl_klein(
    ui: &mut egui::Ui,
    f: &Colors,
    symbol: &str,
    word: &str,
    state: Ctl,
    ctr: Option<u32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::click());
    let (reason, rand, front) = match state {
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
        Ctl::Danger => (f.bad.gamma_multiply(0.14), f.bad.gamma_multiply(0.32), f.bad),
    };
    let reason = if resp.hovered() {
        reason.gamma_multiply(1.35)
    } else {
        reason
    };
    let mal = ui.painter();
    mal.rect_filled(rect, 9.0, reason);
    mal.rect_stroke(rect, 9.0, egui::Stroke::new(1.0, rand), egui::StrokeKind::Inside);
    icons::image(symbol, 17.0, front)
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
    resp.on_hover_text(word)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A footer-bar button: glyph on top, word below - like Meet
/// and Teams. Exactly the shape the browser client shows.
fn ctl(
    ui: &mut egui::Ui,
    f: &Colors,
    symbol: &str,
    word: &str,
    state: Ctl,
    ctr: Option<u32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 58.0), egui::Sense::click());
    let (reason, rand, front, word_color) = match state {
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
        Ctl::Danger => (
            f.bad.gamma_multiply(0.14),
            f.bad.gamma_multiply(0.32),
            f.bad,
            f.bad,
        ),
    };
    let reason = if resp.hovered() {
        reason.gamma_multiply(1.35)
    } else {
        reason
    };
    let mal = ui.painter();
    mal.rect_filled(rect, 14.0, reason);
    mal.rect_stroke(
        rect,
        14.0,
        egui::Stroke::new(1.0, rand),
        egui::StrokeKind::Inside,
    );
    let sym = Rect::from_center_size(pos2(rect.center().x, rect.top() + 20.0), vec2(22.0, 22.0));
    icons::image(symbol, 22.0, front).paint_at(ui, sym);
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_BOTTOM,
        word,
        egui::FontId::proportional(10.5),
        word_color,
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
    f: &Colors,
    symbol: &str,
    dimension: f32,
    an: bool,
    emphasized: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(dimension), egui::Sense::click());
    let (reason, front) = if emphasized {
        (f.p.accent, f.p.on_accent)
    } else if an {
        (f.bad, Color32::from_rgb(0x2b, 0x05, 0x05))
    } else {
        (f.deep.gamma_multiply(if resp.hovered() { 1.5 } else { 1.0 }), f.p.text)
    };
    ui.painter()
        .circle_filled(rect.center(), dimension / 2.0, reason);
    if !emphasized && !an {
        ui.painter().circle_stroke(
            rect.center(),
            dimension / 2.0 - 0.5,
            egui::Stroke::new(1.0, f.p.line.gamma_multiply(2.0)),
        );
    }
    let s = dimension * 0.46;
    icons::image(symbol, s, front).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(s)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small button in lists ("Let in", "Control").
/// A small colour dot to click. The chosen one gets a ring -
/// without it nobody knows which one is currently active.
fn color_dot(ui: &mut egui::Ui, tint: [u8; 3], picked: bool, f: &Colors) -> bool {
    let (r, reply) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
    let c = Color32::from_rgb(tint[0], tint[1], tint[2]);
    ui.painter().circle_filled(r.center(), 8.0, c);
    if picked {
        ui.painter()
            .circle_stroke(r.center(), 9.5, egui::Stroke::new(2.0, f.p.accent));
    } else if reply.hovered() {
        ui.painter()
            .circle_stroke(r.center(), 9.5, egui::Stroke::new(1.0, f.p.line));
    }
    if reply.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    reply.clicked()
}

fn mini(ui: &mut egui::Ui, f: &Colors, text: &str, emphasized: bool) -> egui::Response {
    let knopf = egui::Button::new(
        egui::RichText::new(text)
            .size(11.5)
            .color(if emphasized { f.p.on_accent } else { f.p.text }),
    )
    .fill(if emphasized { f.p.accent } else { f.p.card_hi })
    .stroke(egui::Stroke::new(
        1.0,
        if emphasized {
            Color32::TRANSPARENT
        } else {
            f.p.line
        },
    ))
    .corner_radius(9);
    ui.add(knopf).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Paint an image into a rectangle. `whole` = show completely (cut off
/// nothing), otherwise crop to fill. `mirror` only for the own
/// camera image - that is how you know yourself from the mirror.
fn image_paint(
    mal: &egui::Painter,
    rect: Rect,
    tex: &egui::TextureHandle,
    mirror: bool,
    whole: bool,
) {
    image_paint_zoom(mal, rect, tex, mirror, whole, None)
}

/// Like `image_paint`, but with a section (0..1) - there to zoom into a
/// shared screen without recomputing the image.
fn image_paint_zoom(
    mal: &egui::Painter,
    rect: Rect,
    tex: &egui::TextureHandle,
    mirror: bool,
    whole: bool,
    crop: Option<Rect>,
) {
    let ar_image = tex.aspect_ratio().max(0.05);
    let ar_rect = (rect.width() / rect.height().max(1.0)).max(0.05);
    let (dest, mut uv) = if whole {
        // Alles zeigen: hineinpassen, Rand bleibt Background.
        let mut b = rect.width();
        let mut h = b / ar_image;
        if h > rect.height() {
            h = rect.height();
            b = h * ar_image;
        }
        (
            Rect::from_center_size(rect.center(), vec2(b, h)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        )
    } else {
        // Fill: the overhang is cut away via the image
        // coordinates so that no black bars appear.
        let uv = if ar_image > ar_rect {
            let u = (ar_rect / ar_image).clamp(0.0, 1.0);
            Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0))
        } else {
            let v = (ar_image / ar_rect).clamp(0.0, 1.0);
            Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0))
        };
        (rect, uv)
    };
    // The zoom section is placed INTO the already computed area -
    // so the cropping for odd aspect ratios is preserved.
    if let Some(a) = crop {
        let (b, h) = (uv.width(), uv.height());
        uv = Rect::from_min_max(
            pos2(uv.min.x + a.min.x * b, uv.min.y + a.min.y * h),
            pos2(uv.min.x + a.max.x * b, uv.min.y + a.max.y * h),
        );
    }
    if mirror {
        uv = Rect::from_min_max(
            pos2(uv.max.x, uv.min.y),
            pos2(uv.min.x, uv.max.y),
        );
    }
    mal.image(tex.id(), dest, uv, Color32::WHITE);
}

/// Which section of the image is visible at this magnification?
///
/// `mitte` is the point (0..1) being zoomed to. The result ALWAYS lies
/// completely within the image - otherwise you would see black stripes at the edge
/// or stretched pixels. That is exactly why the centre moves
/// inwards at the edge instead of letting the section stick out.
pub fn zoom_crop(zoom: f32, mitte: Vec2) -> Rect {
    let z = zoom.clamp(1.0, 8.0);
    let half = 0.5 / z;
    let x = mitte.x.clamp(half, 1.0 - half);
    let y = mitte.y.clamp(half, 1.0 - half);
    Rect::from_min_max(pos2(x - half, y - half), pos2(x + half, y + half))
}

/// Which part of the RECEIVED image shows the wanted window?
///
/// `window` and `region` are both fractions of the sender's WHOLE screen.
/// If it sends only a section (`region` smaller than everything),
/// the incoming image contains exactly this section - the magnifier must then
/// not zoom in as far again, otherwise it would be doubled.
///
/// This very calculation makes the sharp zoom possible AND correct: as soon as
/// the sender delivers the wanted area, the result is the whole
/// image (0..1) - so real pixels instead of extrapolated ones.
pub fn zoom_uv(window: Rect, region: Rect) -> Rect {
    let bw = region.width().max(0.0001);
    let bh = region.height().max(0.0001);
    let x0 = ((window.min.x - region.min.x) / bw).clamp(0.0, 1.0);
    let y0 = ((window.min.y - region.min.y) / bh).clamp(0.0, 1.0);
    let x1 = ((window.max.x - region.min.x) / bw).clamp(0.0, 1.0);
    let y1 = ((window.max.y - region.min.y) / bh).clamp(0.0, 1.0);
    // Never collapse to zero - an empty area would be a black
    // image instead of a section.
    let x1 = x1.max(x0 + 0.001).min(1.0);
    let y1 = y1.max(y0 + 0.001).min(1.0);
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

/// Is the image cropped when we draw it to fill? If the
/// aspect ratio deviates by more than a fifth (portrait from a phone,
/// 4:3 webcam, ultrawide monitor), we prefer to show everything.
fn fits_not(tex: &egui::TextureHandle, rect: Rect) -> bool {
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
pub struct Tile {
    pub id: u64,
    pub name: String,
    pub is_muted: bool,
    pub hand: bool,
    pub speaking: bool,
    pub ich: bool,
    pub screen: bool,
    /// Camera switched off - then NEVER show an old still image, but
    /// the circle with the initials. A frozen face would be
    /// a lie about the state of the other person.
    pub camera_from: bool,
}

fn tile_paint(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Tile,
    tex: Option<&egui::TextureHandle>,
    f: &Colors,
) -> egui::Response {
    tile_paint_zoom(ui, rect, k, tex, f, None)
}

fn tile_paint_zoom(
    ui: &mut egui::Ui,
    rect: Rect,
    k: &Tile,
    tex: Option<&egui::TextureHandle>,
    f: &Colors,
    crop: Option<Rect>,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        ui.id().with(("kachel", k.id, k.screen)),
        egui::Sense::click_and_drag(),
    );
    let mal = ui.painter_at(rect);
    mal.rect_filled(rect, 14.0, f.deep);
    match tex {
        Some(t) => {
            // A shared screen is NEVER cropped - cropped
            // text is worthless.
            let whole = k.screen || fits_not(t, rect);
            image_paint_zoom(&mal, rect, t, k.ich && !k.screen, whole, crop);
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
                initials(&k.name),
                egui::FontId::proportional((circle * 0.72).clamp(11.0, 24.0)),
                f.p.text,
            );
            if k.camera_from && rect.height() > 120.0 {
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
    let badge_text = if k.screen {
        format!("{} · Bildschirm", k.name)
    } else {
        k.name.clone()
    };
    let galley = mal.layout_no_wrap(badge_text, font, f.p.text);
    let glyphs = (if k.is_muted { 1 } else { 0 } + if k.hand { 1 } else { 0 }) as f32;
    let width_px = (galley.size().x + 18.0 + glyphs * 19.0).min(rect.width() - 16.0);
    let badge = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.bottom() - 8.0 - 24.0),
        vec2(width_px, 24.0),
    );
    // Semi-transparent like in the browser (rgba(5,7,12,.68)) - the image can still
    // be guessed underneath, the text is readable anyway.
    mal.rect_filled(
        badge,
        9.0,
        if f.p.dark {
            Color32::from_black_alpha(180)
        } else {
            Color32::from_white_alpha(210)
        },
    );
    mal.rect_stroke(
        badge,
        9.0,
        egui::Stroke::new(1.0, f.p.line),
        egui::StrokeKind::Inside,
    );
    let mut x = badge.left() + 9.0;
    mal.galley(
        pos2(x, badge.center().y - galley.size().y / 2.0),
        galley.clone(),
        f.p.text,
    );
    x += galley.size().x + 5.0;
    if k.is_muted {
        icons::image("mic-off", 13.0, f.bad).paint_at(
            ui,
            Rect::from_min_size(pos2(x, badge.center().y - 6.5), Vec2::splat(13.0)),
        );
        x += 19.0;
    }
    if k.hand {
        icons::image("hand", 13.0, f.warn).paint_at(
            ui,
            Rect::from_min_size(pos2(x, badge.center().y - 6.5), Vec2::splat(13.0)),
        );
    }
    // Border last, so that it lies above the image.
    let (border_color, dicke) = if k.speaking {
        (f.p.green, 2.0)
    } else if k.hand {
        (f.warn, 2.0)
    } else {
        (f.p.line, 1.0)
    };
    ui.painter().rect_stroke(
        rect,
        14.0,
        egui::Stroke::new(dicke, border_color),
        egui::StrokeKind::Inside,
    );
    resp
}

// ------------------------------------------------------------- Beitritt

/// The screen before joining - centred card like `#vorbereiten`.
pub fn join_ui(
    ctx: &egui::Context,
    b: &mut Join,
    preview: Option<&egui::TextureHandle>,
) -> Option<JoinAction> {
    let f = colors();
    let mut action = None;
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
                                        action = join_card(ui, &f, b, preview);
                                    },
                                );
                            });
                    });
                    ui.add_space(16.0);
                });
        });
    action
}

fn join_card(
    ui: &mut egui::Ui,
    f: &Colors,
    b: &mut Join,
    preview: Option<&egui::TextureHandle>,
) -> Option<JoinAction> {
    let mut action = None;
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
    let under = if b.heading.trim().is_empty() {
        format!("Meeting {}", b.room)
    } else {
        format!("{} · Meeting {}", b.heading.trim(), b.room)
    };
    ui.label(egui::RichText::new(under).size(13.0).color(f.p.muted));
    ui.add_space(10.0);

    // ---- Self-preview 16:9 with level bar and two round buttons
    let width_px = ui.available_width();
    let height_px = (width_px * 9.0 / 16.0).clamp(120.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width_px, height_px), egui::Sense::hover());
    {
        let mal = ui.painter_at(rect);
        mal.rect_filled(rect, 14.0, f.deep);
        match (b.camera_on, preview) {
            (true, Some(t)) => image_paint(&mal, rect, t, true, fits_not(t, rect)),
            _ => {
                mal.text(
                    rect.center() + vec2(0.0, 22.0),
                    egui::Align2::CENTER_CENTER,
                    if b.camera_on {
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
    if !b.camera_on || preview.is_none() {
        icons::image("cam-off", 34.0, f.p.muted.gamma_multiply(0.6)).paint_at(
            ui,
            Rect::from_center_size(rect.center() - vec2(0.0, 8.0), Vec2::splat(34.0)),
        );
    }
    // Level bar at the top - you see at once whether the microphone hears anything.
    let bar = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.top() + 12.0),
        vec2(rect.width() - 24.0, 5.0),
    );
    ui.painter()
        .rect_filled(bar, 3.0, Color32::from_white_alpha(30));
    let full = bar.width() * b.level.clamp(0.0, 1.0);
    if full > 1.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(bar.min, vec2(full, bar.height())),
            3.0,
            if b.mic_on { f.p.green } else { f.p.muted },
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
            if b.mic_on { "mic" } else { "mic-off" },
            42.0,
            !b.mic_on,
            false,
        )
        .on_hover_text("Mikrofon")
        .clicked()
        {
            action = Some(JoinAction::MicAround);
        }
        ui.add_space(8.0);
        if rund(
            ui,
            f,
            if b.camera_on { "cam" } else { "cam-off" },
            42.0,
            !b.camera_on,
            false,
        )
        .on_hover_text("Kamera")
        .clicked()
        {
                action = Some(JoinAction::CameraAround);
            }
        },
    );

    ui.add_space(12.0);
    field_caption(ui, f, None, "Dein Name");
    ui.add(
        egui::TextEdit::singleline(&mut b.name)
            .desired_width(f32::INFINITY)
            .hint_text("Name")
            .margin(egui::Margin::symmetric(10, 7)),
    );
    ui.add_space(10.0);

    // Geraetewahl nebeneinander wie ".gitter2".
    if !b.devices_there {
        ui.label(
            egui::RichText::new("Geräte werden eingelesen …")
                .size(11.5)
                .color(f.p.muted),
        );
        ui.add_space(4.0);
    }
    let half = (ui.available_width() - 12.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(half);
            field_caption(ui, f, Some("cam"), "Kamera");
            devices_pick(ui, "meet_cam_pick", half, &b.cams, &mut b.cam_sel);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(half);
            field_caption(ui, f, Some("mic"), "Mikrofon");
            devices_pick(ui, "meet_mic_pick", half, &b.mics, &mut b.mic_sel);
        });
    });

    if !b.hint_msg.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(&b.hint_msg)
                .size(11.5)
                .color(f.p.muted),
        );
    }
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let b_width = (ui.available_width() - 10.0) * 0.62;
        let gross = egui::Button::image_and_text(
            icons::image("enter", 18.0, f.p.on_accent),
            egui::RichText::new(if b.running {
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
        if ui.add_enabled(!b.running, gross).clicked() {
            action = Some(JoinAction::Join);
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
            action = Some(JoinAction::Back);
        }
    });
    action
}

fn field_caption(ui: &mut egui::Ui, f: &Colors, symbol: Option<&str>, text: &str) {
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

fn devices_pick(ui: &mut egui::Ui, id: &str, width_px: f32, namen: &[String], pick: &mut usize) {
    let now_ts = if *pick == 0 {
        "Standardgerät".to_string()
    } else {
        namen
            .get(*pick - 1)
            .cloned()
            .unwrap_or_else(|| "Standardgerät".to_string())
    };
    egui::ComboBox::from_id_salt(id)
        .width(width_px - 8.0)
        .selected_text(egui::RichText::new(short(&now_ts, 30)).size(12.5))
        .show_ui(ui, |ui| {
            if ui.selectable_label(*pick == 0, "Standardgerät").clicked() {
                *pick = 0;
            }
            for (i, n) in namen.iter().enumerate() {
                if ui.selectable_label(*pick == i + 1, n).clicked() {
                    *pick = i + 1;
                }
            }
        });
}

fn short(s: &str, max: usize) -> String {
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
pub fn waiting_screen_ui(ctx: &egui::Context, heading: &str, text: &str) -> bool {
    let f = colors();
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
                                    egui::RichText::new(if heading.trim().is_empty() {
                                        "Bitte kurz warten"
                                    } else {
                                        heading
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
    s: &MeetView,
    z: &mut MeetWindowState,
    images: &Images,
) -> Vec<Action> {
    let f = colors();
    let mut actions: Vec<Action> = Vec::new();

    // The control buttons now sit small at the top right instead of as a wide
    // bar at the bottom - that cost almost a tenth of the window height, and in the
    // meeting every pixel counts for the image.
    head(ctx, s, z, &f, &mut actions);
    if z.page_open {
        page(ctx, s, z, &f, &mut actions);
    }
    if !s.waiting.is_empty() {
        lobby_bar(ctx, s, z, &f, &mut actions);
    }
    if !s.control_request.is_empty() {
        control_bar(ctx, s, &f, &mut actions);
    }
    if z.settings_open {
        settings(ctx, s, z, &f, &mut actions);
    }
    if z.screen_pick_open {
        screen_pick(ctx, s, z, &f, &mut actions);
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(f.p.bg).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            stage(ui, s, z, images, &f);
        });
    actions
}

fn head(
    ctx: &egui::Context,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
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

                let room_text = if s.heading.trim().is_empty() {
                    s.room.clone()
                } else {
                    format!("{} · {}", s.heading.trim(), s.room)
                };
                tag(ui, f, None, &room_text, Ton::Neutral, false);
                if tag(ui, f, Some("link"), "Einladung", Ton::Neutral, true)
                    .on_hover_text("Einladung in die Zwischenablage")
                    .clicked()
                {
                    actions.push(Action::InviteCopy);
                }
                if s.is_host {
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
                if !s.bandwidth.is_empty() {
                    tag(ui, f, None, &s.bandwidth, Ton::Neutral, false);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    // Built from RIGHT to left: hang-up at the very
                    // outside, where you look for it.
                    if ctl_klein(ui, f, "leave", "Meeting verlassen", Ctl::Danger, None).clicked() {
                        actions.push(Action::LeaveMeet);
                    }
                    if s.is_host {
                        if ctl_klein(ui, f, "end", "Meeting für ALLE beenden", Ctl::Danger, None)
                            .clicked()
                        {
                            actions.push(Action::Leave);
                        }
                        if ctl_klein(ui, f, "muteall", "Alle stummschalten", Ctl::Normal, None)
                            .clicked()
                        {
                            actions.push(Action::MuteAll);
                        }
                    }
                    if ctl_klein(
                        ui,
                        f,
                        "chat",
                        "Chat",
                        if z.page_open { Ctl::An } else { Ctl::Normal },
                        Some(s.unread),
                    )
                    .clicked()
                    {
                        z.page_open = !z.page_open;
                        z.tab = Tab::Chat;
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
                        actions.push(Action::Hand(!s.hand));
                    }
                    if !s.screens.is_empty() {
                        if ctl_klein(
                            ui,
                            f,
                            if z.fullscreen { "shrink" } else { "full" },
                            "Geteilten Bildschirm groß auf die ganze Fläche",
                            if z.fullscreen { Ctl::An } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        {
                            z.fullscreen = !z.fullscreen;
                        }
                        if ctl_klein(
                            ui,
                            f,
                            "layout",
                            z.camera_slot.word(),
                            if z.camera_slot == CameraSlot::Aus { Ctl::Aus } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        {
                            z.camera_slot = z.camera_slot.advance();
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
                    if let Some((wer, fvid)) = s.control_grant.clone() {
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
                            actions.push(Action::AskControl(fvid, wer));
                        }
                    } else if s.foreign_screen
                        && ctl_klein(
                            ui,
                            f,
                            "gamepad",
                            if s.control_asked {
                                "Anfrage läuft – warte auf Antwort"
                            } else {
                                "Steuerung des geteilten Bildschirms anfragen"
                            },
                            if s.control_asked { Ctl::An } else { Ctl::Normal },
                            None,
                        )
                        .clicked()
                        && !s.control_asked
                    {
                        actions.push(Action::RequestControl);
                    }
                    if !s.control_allowed.is_empty() {
                        let namen: Vec<&str> =
                            s.control_allowed.iter().map(|(_, n)| n.as_str()).collect();
                        if ctl_klein(
                            ui,
                            f,
                            "shield",
                            &format!("{} darf steuern – Freigabe zurücknehmen", namen.join(", ")),
                            Ctl::Danger,
                            Some(s.control_allowed.len() as u32),
                        )
                        .clicked()
                        {
                            actions.push(Action::ControlGiveBack(None));
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
                            actions.push(Action::Screen(false));
                        } else if s.monitore.len() > 1 || !s.window.is_empty() {
                            // As soon as there is something to choose, we ask.
                            // Earlier, with a single screen, EVERYTHING went out
                            // unasked - exactly what Justin did not want
                            // anymore.
                            z.screen_pick_open = true;
                        } else {
                            actions.push(Action::PickScreen(0));
                        }
                    }
                    if ctl_klein(
                        ui,
                        f,
                        if s.camera_on { "cam" } else { "cam-off" },
                        if s.camera_on { "Kamera aus" } else { "Kamera an" },
                        if s.camera_on { Ctl::Normal } else { Ctl::Aus },
                        None,
                    )
                    .clicked()
                    {
                        actions.push(Action::WebCam(!s.camera_on));
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
                        actions.push(Action::Muted(!s.is_muted));
                    }
                    ui.add_space(6.0);
                    // Only icon and number - the word "participants" tells
                    // nobody anything new and eats space in the bar.
                    tag(ui, f, Some("people"), &s.people.len().to_string(), Ton::Neutral, false)
                        .on_hover_text("Wer gerade im Raum ist");
                });
            });
        });
}

/// Which screen to share? Only needed when there are several.
fn screen_pick(
    ctx: &egui::Context,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
) {
    let mut is_open = true;
    egui::Window::new("Was möchtest du teilen?")
        .open(&mut is_open)
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
                let text = format!("{}  ·  {}×{}", short(name, 26), b, h);
                if mini(ui, f, &text, i == 0).clicked() {
                    actions.push(Action::PickScreen(i));
                    z.screen_pick_open = false;
                }
            }
            if !s.window.is_empty() {
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
                        for (k, heading, prog) in s.window.iter() {
                            let text = if prog.is_empty() {
                                short(heading, 40)
                            } else {
                                format!("{}  ·  {}", short(heading, 32), short(prog, 14))
                            };
                            if mini(ui, f, &text, false).clicked() {
                                actions.push(Action::PickWindow(*k));
                                z.screen_pick_open = false;
                            }
                        }
                    });
            }
            ui.add_space(8.0);
            let mut frame_box = s.frame_on;
            if ui
                .checkbox(&mut frame_box, "Rot umranden, was geteilt wird")
                .changed()
            {
                actions.push(Action::FrameBox(frame_box));
            }
            ui.add_space(4.0);
            if mini(ui, f, "Abbrechen", false).clicked() {
                z.screen_pick_open = false;
            }
        });
    if !is_open {
        z.screen_pick_open = false;
    }
}

/// Settings IN THE MIDDLE of the meeting: camera, microphone, speaker.
///
/// WHY as a separate window and not in the sidebar: the choice is
/// a short interruption, afterwards you want it gone again. The
/// browser client does it the same way (overlay above the stage).
fn settings(
    ctx: &egui::Context,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
) {
    let mut is_open = true;
    egui::Window::new("Einstellungen")
        .open(&mut is_open)
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

            field_caption(ui, f, Some("cam"), "Kamera");
            let mut cam = s.cam_sel;
            devices_pick(ui, "set_cam", width_px, &s.cams, &mut cam);
            if cam != s.cam_sel {
                actions.push(Action::CameraDevice(cam));
            }
            if !s.camera_name.is_empty() {
                ui.label(
                    egui::RichText::new(format!("läuft: {}", short(&s.camera_name, 42)))
                        .size(10.5)
                        .color(f.p.muted),
                );
            }
            ui.add_space(6.0);

            field_caption(ui, f, Some("mic"), "Mikrofon");
            let mut mic = s.mic_sel;
            devices_pick(ui, "set_mic", width_px, &s.mics, &mut mic);
            if mic != s.mic_sel {
                actions.push(Action::MicDevice(mic));
            }
            ui.add_space(6.0);

            field_caption(ui, f, Some("sound"), "Lautsprecher");
            let mut spk = s.spk_sel;
            devices_pick(ui, "set_spk", width_px, &s.spks, &mut spk);
            if spk != s.spk_sel {
                actions.push(Action::SpeakerDevice(spk));
            }
            if !s.ton_ein.is_empty() || !s.ton_aus.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "läuft: {} → {}",
                        short(&s.ton_ein, 20),
                        short(&s.ton_aus, 20)
                    ))
                    .size(10.5)
                    .color(f.p.muted),
                );
            }
            ui.add_space(10.0);

            // ---------------------------------------------------- Avatar
            field_caption(ui, f, Some("user"), "Avatar statt Kamera");
            let mut av = s.avatar_an;
            if ui
                .checkbox(&mut av, "Gezeichnetes Gesicht senden (Mund folgt der Stimme)")
                .changed()
            {
                actions.push(Action::Avatar(av));
            }
            if s.avatar_an {
                let mut a = s.avatar;
                let mut modified = false;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Haut").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::SKIN_TONES.len() as u8 {
                        if color_dot(ui, crate::avatar::SKIN_TONES[i as usize], a.haut == i, f) {
                            a.haut = i;
                            modified = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Haare").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::HAIR_COLORS.len() as u8 {
                        if color_dot(ui, crate::avatar::HAIR_COLORS[i as usize], a.hair == i, f) {
                            a.hair = i;
                            modified = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Frisur").size(11.0).color(f.p.muted));
                    for (i, name) in crate::avatar::HAIR_SHAPES.iter().enumerate() {
                        if mini(ui, f, name, a.form == i as u8).clicked() {
                            a.form = i as u8;
                            modified = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Augen").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::EYE_COLORS.len() as u8 {
                        if color_dot(ui, crate::avatar::EYE_COLORS[i as usize], a.eyes == i, f) {
                            a.eyes = i;
                            modified = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Hintergrund").size(11.0).color(f.p.muted));
                    for i in 0..crate::avatar::BASE_COLORS.len() as u8 {
                        if color_dot(ui, crate::avatar::BASE_COLORS[i as usize], a.reason == i, f) {
                            a.reason = i;
                            modified = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let mut glasses = a.glasses;
                    if ui.checkbox(&mut glasses, "Brille").changed() {
                        a.glasses = glasses;
                        modified = true;
                    }
                    let mut bart = a.bart;
                    if ui.checkbox(&mut bart, "Bart").changed() {
                        a.bart = bart;
                        modified = true;
                    }
                });
                if modified {
                    actions.push(Action::AvatarLook(a));
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
            field_caption(ui, f, Some("shield"), "Rauschsperre");
            let mut gate_on = s.lock_on;
            if ui
                .checkbox(&mut gate_on, "Leises Rauschen nicht übertragen")
                .changed()
            {
                actions.push(Action::NoiseGate(gate_on, s.lock_strength));
            }
            if s.lock_on {
                let mut st = s.lock_strength as f32;
                let slider = ui.add(
                    egui::Slider::new(&mut st, 0.0..=100.0)
                        .show_value(false)
                        .text("weniger streng  ↔  strenger"),
                );
                if slider.changed() {
                    actions.push(Action::NoiseGate(true, st.round() as u8));
                }
                // Without feedback the slider would be flying blind: here
                // you see whether sound is currently let through or blocked
                // and what the gate has calibrated to.
                let (r, _) = ui.allocate_exact_size(vec2(width_px, 10.0), egui::Sense::hover());
                let mal = ui.painter();
                mal.rect_filled(r, 5.0, f.p.card_hi);
                let level = s.own_level.clamp(0.0, 1.0).sqrt();
                let full = Rect::from_min_size(r.min, vec2(r.width() * level, r.height()));
                mal.rect_filled(
                    full,
                    5.0,
                    if s.lock_open { f.p.accent } else { f.p.muted },
                );
                // Where the threshold roughly lies (noise floor times
                // factor), as a thin line.
                let s_share = (s.lock_reason
                    * (2.0 + s.lock_strength as f32 / 100.0 * 6.0))
                    .clamp(0.0, 1.0)
                    .sqrt();
                let x = r.left() + r.width() * s_share;
                mal.line_segment(
                    [pos2(x, r.top()), pos2(x, r.bottom())],
                    egui::Stroke::new(1.5, f.p.accent),
                );
                ui.label(
                    egui::RichText::new(if s.lock_open {
                        "geht raus"
                    } else {
                        "gesperrt – es geht gerade nichts raus"
                    })
                    .size(10.0)
                    .color(if s.lock_open { f.p.accent } else { f.p.muted }),
                );
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if mini(ui, f, "Geräte neu suchen", false).clicked() {
                    actions.push(Action::ReloadDevices);
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
    if !is_open {
        z.settings_open = false;
    }
}

// ------------------------------------------------------------- Seitenleiste

fn page(
    ctx: &egui::Context,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
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
                let full = ui.available_width() - 40.0;
                for (r, sym, word) in [
                    (Tab::Chat, "chat", "Chat"),
                    (Tab::People, "people", "Leute"),
                    (Tab::Info, "info", "Info"),
                ] {
                    if tab_button(ui, f, sym, word, z.tab == r, full / 3.0 - 4.0).clicked() {
                        z.tab = r;
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
                    z.page_open = false;
                }
            });
            ui.painter().hline(
                ui.max_rect().left()..=ui.max_rect().right(),
                ui.cursor().top(),
                egui::Stroke::new(1.0, f.p.line),
            );
            ui.add_space(1.0);
            match z.tab {
                Tab::Chat => tab_chat(ui, s, z, f, actions),
                Tab::People => tab_people(ui, s, f, actions),
                Tab::Info => tab_info(ui, s, f),
            }
        });
}

fn tab_button(
    ui: &mut egui::Ui,
    f: &Colors,
    symbol: &str,
    word: &str,
    active: bool,
    width_px: f32,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width_px.max(40.0), 44.0), egui::Sense::click());
    let tint = if active {
        f.p.accent
    } else if resp.hovered() {
        f.p.text
    } else {
        f.p.muted
    };
    if active {
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
        word,
        egui::FontId::proportional(11.0),
        tint,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn tab_chat(
    ui: &mut egui::Ui,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
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
            for text_row in s.chat.iter() {
                chat_line(ui, text_row, f);
                ui.add_space(8.0);
            }
            if !s.type_text.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("{} schreibt …", s.type_text.join(", ")))
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
        let field = ui.add(
            egui::TextEdit::singleline(&mut z.input_text)
                .desired_width((ui.available_width() - 58.0).max(60.0))
                .hint_text("Nachricht …")
                .margin(egui::Margin::symmetric(11, 7)),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let types = !z.input_text.trim().is_empty();
        if types != z.types_reported {
            z.types_reported = types;
            actions.push(Action::Types(types));
        }
        let send_out = rund(ui, f, "send", 34.0, false, true)
            .on_hover_text("Senden")
            .clicked();
        if (enter || send_out) && !z.input_text.trim().is_empty() {
            actions.push(Action::Send(z.input_text.trim().to_string()));
            z.input_text.clear();
            z.types_reported = false;
            if enter {
                field.request_focus();
            }
        }
    });
    ui.add_space(7.0);
}

fn chat_line(ui: &mut egui::Ui, text_row: &ChatLine, f: &Colors) {
    if text_row.von == 0 {
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(&text_row.text)
                    .size(11.5)
                    .color(f.p.muted),
            );
        });
        return;
    }
    let full = ui.available_width() - 20.0;
    let layout = if text_row.own {
        egui::Layout::right_to_left(egui::Align::TOP)
    } else {
        egui::Layout::left_to_right(egui::Align::TOP)
    };
    ui.allocate_ui_with_layout(vec2(full, 0.0), layout, |ui| {
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 14.0, f.p.accent);
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            initials(&text_row.name),
            egui::FontId::proportional(12.0),
            f.p.on_accent,
        );
        ui.add_space(6.0);
        // For own messages pull everything to the right - otherwise the
        // name would stick to the left edge and slide out of the bar.
        let inside = if text_row.own {
            egui::Layout::top_down(egui::Align::Max)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        ui.allocate_ui_with_layout(vec2((full - 50.0).max(80.0), 0.0), inside, |ui| {
            ui.set_max_width((full - 50.0).max(80.0));
            ui.label(
                egui::RichText::new(&text_row.name)
                    .size(10.5)
                    .color(f.p.muted),
            );
            egui::Frame::NONE
                .fill(if text_row.own {
                    f.p.accent.gamma_multiply(0.14)
                } else {
                    f.p.card_hi
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if text_row.own {
                        f.p.accent.gamma_multiply(0.35)
                    } else {
                        f.p.line
                    },
                ))
                .corner_radius(12)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&text_row.text)
                            .size(13.0)
                            .color(f.p.text),
                    );
                });
        });
    });
}

fn tab_people(ui: &mut egui::Ui, s: &MeetView, f: &Colors, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .id_salt("meet_leute")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_space(9.0);
            if s.is_host {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if mini(
                        ui,
                        f,
                        if s.waiting_room_on {
                            "Warteraum: an"
                        } else {
                            "Warteraum: aus"
                        },
                        s.waiting_room_on,
                    )
                    .clicked()
                    {
                        actions.push(Action::WaitingRoom(!s.waiting_room_on));
                    }
                });
                ui.add_space(8.0);
            }
            if !s.waiting.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Wartet vor der Tür")
                            .size(11.0)
                            .color(f.warn),
                    );
                });
                ui.add_space(3.0);
                for (id, name) in s.waiting.iter() {
                    person_line(ui, f, name, false, |ui| {
                        if mini(ui, f, "Einlassen", true).clicked() {
                            actions.push(Action::Admit(*id));
                        }
                        if mini(ui, f, "Ablehnen", false).clicked() {
                            actions.push(Action::Refuse(*id));
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if mini(ui, f, "Alle einlassen", true).clicked() {
                        actions.push(Action::AllAdmit);
                    }
                });
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!("Im Raum ({})", s.people.len()))
                        .size(11.0)
                        .color(f.p.muted),
                );
            });
            ui.add_space(3.0);
            for p in s.people.iter() {
                let name = if p.ich {
                    format!("{} (du)", p.name)
                } else {
                    p.name.clone()
                };
                person_line(ui, f, &name, p.is_host, |ui| {
                    // Right-to-left: whatever comes first sits at the far right.
                    // Buttons first, then the icons - so the mute
                    // and raised-hand icons stick to the name and not to the window edge.
                    if !p.ich && !p.fvid.is_empty() {
                        if mini(ui, f, "Steuern", false)
                            .on_hover_text(format!("{} mit FreeViewer fernsteuern", p.name))
                            .clicked()
                        {
                            actions.push(Action::AskControl(p.fvid.clone(), p.name.clone()));
                        }
                    }
                    if s.is_host && !p.ich {
                        if mini(ui, f, "Stumm", false).clicked() {
                            actions.push(Action::Mute(p.id));
                        }
                        if mini(ui, f, "Raus", false).clicked() {
                            actions.push(Action::Kick(p.id));
                        }
                    }
                    if p.camera_from {
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

fn person_line(
    ui: &mut egui::Ui,
    f: &Colors,
    name: &str,
    is_host: bool,
    right_side: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(9.0);
        egui::Frame::NONE
            .fill(f.deep)
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
                        initials(name),
                        egui::FontId::proportional(11.0),
                        f.p.on_accent,
                    );
                    ui.add_space(3.0);
                    // FIRST place the buttons on the right, THEN put the name in the
                    // remainder and cut it off there. The other way round
                    // a long name would push the buttons out of the bar.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        right_side(ui);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if is_host {
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

fn tab_info(ui: &mut egui::Ui, s: &MeetView, f: &Colors) {
    egui::ScrollArea::vertical()
        .id_salt("meet_info")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(9.0);
            for text_row in s.protocol.iter() {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(text_row)
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
fn control_bar(
    ctx: &egui::Context,
    s: &MeetView,
    f: &Colors,
    actions: &mut Vec<Action>,
) {
    // Below the waiting-room bar, otherwise the two overlap.
    let top_y = if s.waiting.is_empty() { 56.0 } else { 100.0 };
    egui::Area::new(egui::Id::new("meet_steuerfrage"))
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, top_y))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (id, name) in s.control_request.iter() {
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
                                actions.push(Action::ControlReply(*id, true));
                            }
                            if mini(ui, f, "Ablehnen", false).clicked() {
                                actions.push(Action::ControlReply(*id, false));
                            }
                        });
                    });
                ui.add_space(6.0);
            }
        });
}

fn lobby_bar(
    ctx: &egui::Context,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
    actions: &mut Vec<Action>,
) {
    let text = if s.waiting.len() == 1 {
        format!("{} wartet vor der Tür", s.waiting[0].1)
    } else {
        format!("{} warten vor der Tür", s.waiting.len())
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
                            actions.push(Action::AllAdmit);
                        }
                        if mini(ui, f, "Ansehen", false).clicked() {
                            z.page_open = true;
                            z.tab = Tab::People;
                        }
                    });
                });
        });
}

// ------------------------------------------------------------------ Buehne

fn tiles_build(s: &MeetView) -> (Vec<Tile>, Vec<Tile>) {
    let screens = s
        .screens
        .iter()
        .map(|(id, name)| Tile {
            id: *id,
            name: name.clone(),
            is_muted: false,
            hand: false,
            speaking: false,
            ich: s.people.iter().any(|p| p.id == *id && p.ich),
            screen: true,
            camera_from: false,
        })
        .collect();
    let cameras = s
        .people
        .iter()
        .map(|p| Tile {
            id: p.id,
            // The own tile is called "Du" - just like in the browser. In the
            // participant list the real name is shown, where it is useful.
            name: if p.ich { "Du".to_string() } else { p.name.clone() },
            is_muted: p.is_muted,
            hand: p.hand,
            speaking: p.speaking && !p.is_muted,
            ich: p.ich,
            screen: false,
            camera_from: p.camera_from,
        })
        .collect();
    (screens, cameras)
}

fn tex_for<'a>(k: &Tile, b: &'a Images) -> Option<&'a egui::TextureHandle> {
    if k.camera_from {
        return None;
    }
    if k.screen {
        b.screens.get(&k.id)
    } else if k.ich {
        b.own.as_ref()
    } else {
        b.cameras.get(&k.id)
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
    k: &Tile,
    tex: Option<&egui::TextureHandle>,
    s: &MeetView,
    z: &mut MeetWindowState,
    f: &Colors,
) {
    // --- Wheel: zoom in and out ---
    let pointer = ui.ctx().pointer_latest_pos();
    let via = pointer.map(|p| rect.contains(p)).unwrap_or(false);
    if via {
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
    let mut mitte = z.zoom_center;
    if z.zoom > 1.0 {
        match z.zoom_anchor {
            ZoomAnchor::Own => {
                // The pointer IN the tile area is mapped onto the screen of the
                // other: pointing at top left means seeing top left.
                // No trailing, no drifting - a real magnifier.
                if let Some(p) = pointer {
                    if rect.contains(p) {
                        let fx = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
                        let fy = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                        mitte = vec2(
                            ber.min.x + fx * ber.width(),
                            ber.min.y + fy * ber.height(),
                        );
                        z.zoom_center = mitte;
                    }
                }
            }
            ZoomAnchor::Divider => {
                if let Some((x, y)) = s.divider_pointer {
                    mitte = vec2(x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
                    z.zoom_center = mitte;
                }
            }
            ZoomAnchor::Fest => {}
        }
    }
    // The wanted window - ALWAYS in screen coordinates of the sender.
    // It is also reported upwards so that the sender can send exactly this part
    // sharp.
    let window = zoom_crop(z.zoom, mitte);
    z.zoom_window = (window.min.x, window.min.y, window.width(), window.height());
    let crop = if z.zoom > 1.0 || s.screen_region.is_some() {
        Some(zoom_uv(window, ber))
    } else {
        None
    };

    let reply = tile_paint_zoom(ui, rect, k, tex, f, crop);

    // --- Dragging moves the section (and holds it) ---
    if z.zoom > 1.0 && reply.dragged() {
        let d = reply.drag_delta();
        if d.length_sq() > 0.0 {
            z.zoom_anchor = ZoomAnchor::Fest;
            z.zoom_center = vec2(
                (z.zoom_center.x - d.x / rect.width().max(1.0) * window.width()).clamp(0.0, 1.0),
                (z.zoom_center.y - d.y / rect.height().max(1.0) * window.height()).clamp(0.0, 1.0),
            );
        }
    }
    if reply.double_clicked() {
        z.zoom = 1.0;
        z.zoom_anchor = ZoomAnchor::Own;
        z.zoom_center = vec2(0.5, 0.5);
    }
    if z.zoom > 1.0 && via {
        ui.ctx().set_cursor_icon(if reply.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }

    zoom_bar(ui, rect, s, z, f);
}

/// Kleine Leiste oben rechts im geteilten Bild: -, Stufe, +, Anker, go_back.
fn zoom_bar(ui: &mut egui::Ui, rect: Rect, s: &MeetView, z: &mut MeetWindowState, f: &Colors) {
    if rect.width() < 260.0 || rect.height() < 120.0 {
        return; // in einem Briefmarkenbild waere die Leiste nur im Weg
    }
    let anchor_text = match z.zoom_anchor {
        ZoomAnchor::Divider if s.divider_pointer.is_none() => "Maus des Teilers (kommt nicht an)",
        a => a.word(),
    };
    let level = format!("{:.0} %", z.zoom * 100.0);
    let h = 28.0;
    let mal = ui.painter_at(rect);
    // Roughly estimate the width from the texts - accurate enough for a bar.
    let width_px = 30.0 + 54.0 + 30.0 + 14.0 + anchor_text.chars().count() as f32 * 6.4 + 16.0
        + if z.zoom > 1.0 { 62.0 + 110.0 } else { 0.0 };
    let bar = Rect::from_min_size(
        pos2(rect.right() - 10.0 - width_px.min(rect.width() - 20.0), rect.top() + 10.0),
        vec2(width_px.min(rect.width() - 20.0), h),
    );
    mal.rect_filled(
        bar,
        9.0,
        if f.p.dark {
            Color32::from_black_alpha(185)
        } else {
            Color32::from_white_alpha(215)
        },
    );
    mal.rect_stroke(bar, 9.0, egui::Stroke::new(1.0, f.p.line), egui::StrokeKind::Inside);

    let mut x = bar.left() + 5.0;
    let mut knopf = |ui: &mut egui::Ui, x: &mut f32, text: &str, w: f32, tip: &str| -> bool {
        let r = Rect::from_min_size(pos2(*x, bar.top() + 3.0), vec2(w, h - 6.0));
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
    let _ = knopf(ui, &mut x, &level, 50.0, "Vergroesserung");
    if knopf(ui, &mut x, "+", 26.0, "Groesser (Mausrad geht auch)") {
        z.zoom = (z.zoom * 1.25).min(8.0);
    }
    x += 6.0;
    if knopf(
        ui,
        &mut x,
        anchor_text,
        anchor_text.chars().count() as f32 * 6.4 + 14.0,
        "Woran haengt der Ausschnitt? Klicken zum Umschalten.",
    ) {
        z.zoom_anchor = z.zoom_anchor.advance();
    }
    if z.zoom > 1.0 {
        // Show honestly WHAT is arriving: real pixels or
        // extrapolated ones. That is the whole difference.
        let (text, tip) = if !s.sharp_possible {
            (
                "unscharf",
                "Der Sender ist ein aelterer Stand und kann keinen Ausschnitt liefern - vergroessert wird nur gerechnet.",
            )
        } else if !z.sharp {
            ("nur vergroessert", "Klicken: den Sender bitten, diesen Ausschnitt scharf zu schicken")
        } else if s.screen_region.map(|(_, _, w, _)| w < 0.95).unwrap_or(false) {
            ("scharf", "Der Sender schickt genau diesen Ausschnitt - echte Bildpunkte. Klicken zum Abschalten.")
        } else {
            ("scharf (kommt)", "Angefragt - der Sender stellt gerade um")
        };
        if knopf(ui, &mut x, text, text.chars().count() as f32 * 6.4 + 14.0, tip)
            && s.sharp_possible
        {
            z.sharp = !z.sharp;
        }
        if knopf(ui, &mut x, "ganz", 50.0, "Zurueck auf das ganze Bild") {
            z.zoom = 1.0;
            z.zoom_anchor = ZoomAnchor::Own;
            z.zoom_center = vec2(0.5, 0.5);
        }
    }
}

fn stage(ui: &mut egui::Ui, s: &MeetView, z: &mut MeetWindowState, b: &Images, f: &Colors) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover());
    if area.width() < 40.0 || area.height() < 40.0 {
        return;
    }
    let (screens, cameras) = tiles_build(s);

    if screens.is_empty() {
        z.fullscreen = false;
        raster(ui, area, &cameras, b, f);
        return;
    }
    // A shared screen is THE content: large and complete.
    let main_part = screens
        .iter()
        .position(|k| !k.ich)
        .unwrap_or(0);
    if z.fullscreen {
        let t = tex_for(&screens[main_part], b);
        let r = screen_box(area.shrink(6.0), t);
        screen_tile(ui, r, &screens[main_part], t, s, z, f);
        return;
    }
    let gap = 10.0;
    let rand = 10.0;
    let inside = area.shrink(rand);
    let (screen_area, stripe) = match z.camera_slot {
        CameraSlot::Aus => (inside, None),
        CameraSlot::Page => {
            let sb = (inside.width() * 0.28).min(232.0).max(120.0);
            if cameras.is_empty() {
                (inside, None)
            } else {
                (
                    Rect::from_min_max(inside.min, pos2(inside.right() - sb - gap, inside.bottom())),
                    Some(Rect::from_min_max(
                        pos2(inside.right() - sb, inside.top()),
                        inside.max,
                    )),
                )
            }
        }
        CameraSlot::Bottom => {
            let sh = (inside.height() * 0.26).min(210.0 * 9.0 / 16.0).max(70.0);
            if cameras.is_empty() {
                (inside, None)
            } else {
                (
                    Rect::from_min_max(inside.min, pos2(inside.right(), inside.bottom() - sh - gap)),
                    Some(Rect::from_min_max(
                        pos2(inside.left(), inside.bottom() - sh),
                        inside.max,
                    )),
                )
            }
        }
    };
    // Mehrere Freigaben: untereinander, jede vollstaendig.
    let n = screens.len().max(1);
    let h = (screen_area.height() - gap * (n as f32 - 1.0)) / n as f32;
    for (i, k) in screens.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(screen_area.left(), screen_area.top() + i as f32 * (h + gap)),
            vec2(screen_area.width(), h),
        );
        if r.height() > 20.0 {
            let t = tex_for(k, b);
            // Only the MAIN share gets the magnifier. With several
            // shares at once a common zoom state would be
            // confusing - you would not know which image it means.
            if i == main_part {
                screen_tile(ui, screen_box(r, t), k, t, s, z, f);
            } else {
                tile_paint(ui, screen_box(r, t), k, t, f);
            }
        }
    }
    // Camera strip. The height is chosen so that all fit in -
    // otherwise the last camera hangs below the footer bar.
    if let Some(st) = stripe {
        match z.camera_slot {
            CameraSlot::Page => {
                let k = cameras.len().max(1) as f32;
                let full = st.width() * 9.0 / 16.0;
                let height_px =
                    ((st.height() - gap * (k - 1.0)) / k).min(full).max(48.0);
                let width_px = (height_px * 16.0 / 9.0).min(st.width());
                for (i, kk) in cameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(
                            st.center().x - width_px / 2.0,
                            st.top() + i as f32 * (height_px + gap),
                        ),
                        vec2(width_px, height_px),
                    );
                    if r.bottom() > st.bottom() + 2.0 {
                        break;
                    }
                    tile_paint(ui, r, kk, tex_for(kk, b), f);
                }
            }
            CameraSlot::Bottom => {
                let height_px = st.height();
                let width_px = height_px * 16.0 / 9.0;
                for (i, kk) in cameras.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(st.left() + i as f32 * (width_px + gap), st.top()),
                        vec2(width_px, height_px),
                    );
                    if r.right() > st.right() + 2.0 {
                        break;
                    }
                    tile_paint(ui, r, kk, tex_for(kk, b), f);
                }
            }
            CameraSlot::Aus => {}
        }
    }
}

/// How many columns? Zoom and Meet try every column count and
/// take the one where the single tile becomes LARGEST - with a fixed
/// 16:9, so that no faces are cropped. Exactly that here.
pub fn best_layout(area: Vec2, item_count: usize) -> (usize, f32) {
    if item_count == 0 {
        return (1, 0.0);
    }
    let gap = 10.0;
    let rand = 20.0;
    let slot_b = area.x - rand;
    let slot_h = area.y - rand;
    // Alone in the meeting the own tile should not fill the whole area.
    let limit = if item_count == 1 {
        (area.x * 0.7).min(880.0)
    } else {
        f32::INFINITY
    };
    let (mut best_columns, mut best_width) = (1usize, 0.0f32);
    for s in 1..=item_count {
        let rows = item_count.div_ceil(s) as f32;
        let from_width = (slot_b - gap * (s as f32 - 1.0)) / s as f32;
        let from_height = ((slot_h - gap * (rows - 1.0)) / rows) * (16.0 / 9.0);
        let width_px = from_width.min(from_height).min(limit);
        if width_px > best_width {
            best_width = width_px;
            best_columns = s;
        }
    }
    (best_columns, best_width.max(80.0))
}

fn raster(ui: &mut egui::Ui, area: Rect, tiles: &[Tile], b: &Images, f: &Colors) {
    if tiles.is_empty() {
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            "Noch niemand im Raum.",
            egui::FontId::proportional(14.0),
            f.p.muted,
        );
        return;
    }
    let gap = 10.0;
    let (spalten, width_px) = best_layout(area.size(), tiles.len());
    let height_px = width_px * 9.0 / 16.0;
    let rows = tiles.len().div_ceil(spalten);
    let total_h = rows as f32 * height_px + (rows as f32 - 1.0) * gap;
    let mut y = area.center().y - total_h / 2.0;
    let mut i = 0usize;
    for _ in 0..rows {
        let in_line = spalten.min(tiles.len() - i);
        let total_b = in_line as f32 * width_px + (in_line as f32 - 1.0) * gap;
        let mut x = area.center().x - total_b / 2.0;
        for _ in 0..in_line {
            let r = Rect::from_min_size(pos2(x, y), vec2(width_px, height_px));
            tile_paint(ui, r, &tiles[i], tex_for(&tiles[i], b), f);
            x += width_px + gap;
            i += 1;
        }
        y += height_px + gap;
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
pub fn pip_content(ui: &mut egui::Ui, s: &MeetView, b: &Images, myself: &mut bool) -> Vec<Action> {
    let f = colors();
    let mut actions: Vec<Action> = Vec::new();
    let (_, cameras) = tiles_build(s);
    // First the others, the own tile last (like in the browser).
    let mut display_items: Vec<&Tile> = cameras.iter().filter(|k| !k.ich).collect();
    if *myself {
        if let Some(ich) = cameras.iter().find(|k| k.ich) {
            display_items.push(ich);
        }
    }
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover());
    ui.painter().rect_filled(area, 0.0, f.p.bg);

    // --- Bar at the bottom: icons only, so that it fits in 260 px width ---
    let bar_h = 34.0;
    let bar = Rect::from_min_size(
        pos2(area.left(), area.bottom() - bar_h),
        vec2(area.width(), bar_h),
    );
    ui.painter().rect_filled(bar, 0.0, f.p.card);
    let buttons: [(&str, bool, &str); 4] = [
        (
            if s.is_muted { "mic-off" } else { "mic" },
            s.is_muted,
            if s.is_muted { "Stummschaltung aufheben" } else { "Stummschalten" },
        ),
        (
            if s.camera_on { "cam" } else { "cam-off" },
            !s.camera_on,
            if s.camera_on { "Kamera aus" } else { "Kamera an" },
        ),
        (
            if *myself { "eye" } else { "eye-off" },
            !*myself,
            if *myself { "Dich hier ausblenden" } else { "Dich hier einblenden" },
        ),
        ("leave", true, "Meeting verlassen"),
    ];
    let gr = 26.0;
    let gap = 8.0;
    let total = buttons.len() as f32 * gr + (buttons.len() as f32 - 1.0) * gap;
    let mut x = bar.center().x - total * 0.5;
    for (i, (name, aus, tip)) in buttons.iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, bar.center().y - gr * 0.5), Vec2::splat(gr));
        let reply = ui
            .interact(r, ui.id().with(("pipknopf", i)), egui::Sense::click())
            .on_hover_text(*tip);
        // The last button is hang-up - always red, otherwise you hit the wrong one.
        let reason = if i == 3 {
            f.bad
        } else if *aus {
            darker(f.p.card_hi, 0.9)
        } else {
            f.p.card_hi
        };
        let reason = if reply.hovered() { darker(reason, 1.25) } else { reason };
        ui.painter().circle_filled(r.center(), gr * 0.5, reason);
        let tint = if i == 3 {
            Color32::WHITE
        } else if *aus {
            f.bad
        } else {
            f.p.text
        };
        let ir = Rect::from_center_size(r.center(), Vec2::splat(gr * 0.58));
        icons::image(name, gr * 0.58, tint).paint_at(ui, ir);
        if reply.clicked() {
            match i {
                0 => actions.push(Action::Muted(!s.is_muted)),
                1 => actions.push(Action::WebCam(!s.camera_on)),
                2 => *myself = !*myself,
                _ => actions.push(Action::LeaveMeet),
            }
        }
        if reply.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        x += gr + gap;
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
    let mut hints: Vec<(String, Option<(Action, Action)>)> = Vec::new();
    for (id, name) in s.control_request.iter() {
        hints.push((
            format!("{} möchte steuern", short(name, 18)),
            Some((
                Action::ControlReply(*id, true),
                Action::ControlReply(*id, false),
            )),
        ));
    }
    for (id, name) in s.waiting.iter() {
        hints.push((
            format!("{} wartet vor der Tür", short(name, 16)),
            Some((Action::Admit(*id), Action::Refuse(*id))),
        ));
    }
    if s.unread > 0 {
        if let Some(z) = s.chat.iter().rev().find(|z| !z.own) {
            hints.push((
                format!("{}: {}", short(&z.name, 12), short(&z.text, 26)),
                None,
            ));
        }
    }
    // At most three - more would wall in the small window.
    hints.truncate(3);
    let mut hint_height = 0.0;
    if !hints.is_empty() {
        let text_row = 46.0;
        hint_height = hints.len() as f32 * text_row + 6.0;
        let region = Rect::from_min_max(
            pos2(area.left() + 6.0, bar.top() - hint_height),
            pos2(area.right() - 6.0, bar.top() - 4.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(region)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                for (text, buttons) in hints.iter() {
                    egui::Frame::NONE
                        .fill(f.p.card_hi)
                        .stroke(egui::Stroke::new(1.0, f.p.accent))
                        .corner_radius(9)
                        .inner_margin(egui::Margin::symmetric(7, 4))
                        .show(ui, |ui| {
                            ui.set_width(region.width() - 14.0);
                            ui.label(
                                egui::RichText::new(text).size(10.5).color(f.p.text),
                            );
                            if let Some((ja, nein)) = buttons {
                                ui.horizontal(|ui| {
                                    if mini(ui, &f, "Erlauben", true).clicked() {
                                        actions.push(ja.clone());
                                    }
                                    if mini(ui, &f, "Ablehnen", false).clicked() {
                                        actions.push(nein.clone());
                                    }
                                });
                            }
                        });
                }
            },
        );
    }

    // --- Kacheln darueber ---
    let top_y = Rect::from_min_max(
        area.min,
        pos2(area.right(), bar.top() - hint_height),
    );
    if display_items.is_empty() {
        ui.painter().text(
            top_y.center(),
            egui::Align2::CENTER_CENTER,
            "Niemand sonst da",
            egui::FontId::proportional(12.0),
            f.p.muted,
        );
        return actions;
    }
    let gap = 6.0;
    let n = display_items.len();
    let height_px = ((top_y.height() - gap * (n as f32 + 1.0)) / n as f32).max(40.0);
    for (i, k) in display_items.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(top_y.left() + gap, top_y.top() + gap + i as f32 * (height_px + gap)),
            vec2(top_y.width() - 2.0 * gap, height_px),
        );
        if r.bottom() > top_y.bottom() {
            break;
        }
        tile_paint(ui, r, k, tex_for(k, b), &f);
    }
    actions
}


// --------------------------------------------------- Beispiele zum Pruefen

/// How many example states there are.
pub const EXAMPLES: usize = 12;

/// Beispielzustand des Beitritts-Schirms (fuer --meetdemo).
pub fn example_join() -> Join {
    Join {
        room: "482-913-770".into(),
        heading: "Wochenbesprechung".into(),
        name: "Justin".into(),
        mics: vec!["Mikrofon (Realtek Audio)".into(), "Headset (Jabra Evolve)".into()],
        cams: vec!["Integrated Webcam".into(), "Logitech C920".into()],
        mic_sel: 1,
        cam_sel: 1,
        mic_on: true,
        camera_on: true,
        devices_there: true,
        level: 0.42,
        hint_msg: "Kamera: Integrated Webcam".into(),
        running: false,
    }
}

/// An example state: (name, view, window state).
///
/// WHY this lives here and not in the test: `--uitest` renders all
/// views headless with it AND `--meetdemo` makes real images from it. Both
/// thereby see guaranteed the same thing - otherwise you end up checking a different
/// window than the one the customer gets.
pub fn example(nr: usize) -> (&'static str, MeetView, MeetWindowState) {
    let namen = ["Justin", "Anna Berger", "Cem", "Dora", "Emil", "Frida"];
    let mach = |item_count: usize, with_screen: bool, waiting_room: bool| {
        let mut people = Vec::new();
        for i in 0..item_count {
            people.push(Person {
                id: i as u64 + 1,
                name: namen[i % namen.len()].to_string(),
                is_muted: i % 3 == 1,
                camera_from: i % 4 == 2,
                hand: i % 5 == 3,
                is_host: i == 0,
                fvid: if i == 1 { "497628420".into() } else { String::new() },
                ich: i == 0,
                speaking: i == 1,
            });
        }
        MeetView {
            window: Vec::new(),
            frame_on: true,
            lock_on: true,
            lock_strength: 45,
            lock_open: true,
            lock_reason: 0.004,
            own_level: 0.2,
            avatar_an: false,
            avatar: crate::avatar::Look::default(),
            control_request: Vec::new(),
            control_allowed: Vec::new(),
            control_asked: false,
            control_grant: None,
            foreign_screen: false,
            room: "482-913-770".into(),
            heading: "Wochenbesprechung".into(),
            is_host: true,
            connection: "direkt verbunden".into(),
            connect_tone: Ton::Gut,
            e2e: false,
            bandwidth: "742 kbit/s".into(),
            people,
            waiting: if waiting_room {
                vec![(90, "Gast am Handy".to_string())]
            } else {
                Vec::new()
            },
            waiting_room_on: waiting_room,
            chat: vec![
                ChatLine {
                    von: 0,
                    name: String::new(),
                    text: "Anna Berger ist dazugekommen".into(),
                    own: false,
                },
                ChatLine {
                    von: 2,
                    name: "Anna Berger".into(),
                    text: "Servus, ich sehe euch. Der Ton passt auch.".into(),
                    own: false,
                },
                ChatLine {
                    von: 1,
                    name: "Justin".into(),
                    text: "Passt. Ich teile gleich den Bildschirm.".into(),
                    own: true,
                },
            ],
            protocol: vec![
                "Im Raum 482-913-770 - Server 1.4.2".into(),
                "Ton verbunden".into(),
                "Kamera: Integrated Webcam".into(),
                "Ton: 1204 raus / 3611 rein · Bild: 890 raus / 2670 rein".into(),
            ],
            is_muted: false,
            camera_on: true,
            screen_on: with_screen,
            hand: false,
            control_free: true,
            screens: if with_screen {
                vec![(1, "Justin".to_string())]
            } else {
                Vec::new()
            },
            unread: 3,
            type_text: vec!["Cem".into()],
            in_waiting_room: false,
            warte_text: String::new(),
            cams: vec!["Integrated Webcam".into(), "Logitech StreamCam".into()],
            mics: vec!["Mikrofonarray".into(), "Yeti Nano".into()],
            spks: vec!["Lautsprecher (Realtek)".into(), "Kopfhoerer".into()],
            cam_sel: 1,
            mic_sel: 0,
            spk_sel: 0,
            camera_name: "Integrated Webcam (1280x720 -> 640x360)".into(),
            ton_ein: "Mikrofonarray".into(),
            ton_aus: "Lautsprecher (Realtek)".into(),
            divider_pointer: if with_screen { Some((0.62, 0.41)) } else { None },
            screen_region: None,
            sharp_possible: with_screen,
            monitore: vec![
                ("Surface Panel".to_string(), 2496, 1664),
                ("Generic PnP Monitor".to_string(), 2160, 1440),
            ],
            // (Example 10 sets the area further below by hand.)
        }
    };
    let zu = |page: bool, tab: Tab, slot: CameraSlot, full: bool| MeetWindowState {
        page_open: page,
        tab,
        camera_slot: slot,
        fullscreen: full,
        ..Default::default()
    };
    // The settings screen needs its own example state, otherwise it
    // would never be drawn in --uitest and an error in it would only be
    // noticed by the user.
    let with_settings = || MeetWindowState {
        settings_open: true,
        ..Default::default()
    };
    match nr {
        0 => (
            "allein",
            mach(1, false, false),
            zu(false, Tab::Chat, CameraSlot::Page, false),
        ),
        1 => (
            "zu zweit",
            mach(2, false, false),
            zu(false, Tab::Chat, CameraSlot::Page, false),
        ),
        2 => (
            "zu viert mit Chat",
            mach(4, false, false),
            zu(true, Tab::Chat, CameraSlot::Page, false),
        ),
        3 => (
            "sechs Leute, Liste offen",
            mach(6, false, true),
            zu(true, Tab::People, CameraSlot::Page, false),
        ),
        4 => (
            "Bildschirm geteilt, Kameras rechts",
            mach(4, true, false),
            zu(false, Tab::Chat, CameraSlot::Page, false),
        ),
        5 => (
            "Bildschirm geteilt, Kameras unten",
            mach(4, true, false),
            zu(true, Tab::Chat, CameraSlot::Bottom, false),
        ),
        6 => (
            "Bildschirm allein gross",
            mach(4, true, false),
            zu(false, Tab::Chat, CameraSlot::Aus, false),
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
            MeetWindowState {
                zoom: 2.5,
                zoom_anchor: ZoomAnchor::Divider,
                zoom_center: vec2(0.62, 0.41),
                ..Default::default()
            },
        ),
        11 => (
            "Bildschirmauswahl offen",
            mach(2, false, false),
            MeetWindowState {
                screen_pick_open: true,
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
            MeetWindowState {
                zoom: 2.5,
                zoom_anchor: ZoomAnchor::Fest,
                zoom_center: vec2(0.67, 0.46),
                ..Default::default()
            },
        ),
        _ => (
            "Info-Reiter, schmales Fenster",
            mach(5, false, false),
            zu(true, Tab::Info, CameraSlot::Page, false),
        ),
    }
}

/// Test images as camera and screen content. A chessboard shows
/// at once whether it is cropped, stretched or mirrored.
pub fn test_images(ctx: &egui::Context) -> Images {
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
    let mut b = Images {
        own: Some(kam.clone()),
        ..Default::default()
    };
    for i in 1..=6u64 {
        b.cameras.insert(i, kam.clone());
    }
    b.screens.insert(1, screen);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kuerzel_nimmt_zwei_anfangsbuchstaben() {
        assert_eq!(initials("Justin Fleischmann"), "JF");
        assert_eq!(initials("anna"), "A");
        assert_eq!(initials("   "), "?");
    }

    #[test]
    fn layout_makes_tiles_as_large_as_possible() {
        // 4 people on a wide area: 2x2 is better than 4x1.
        let (spalten, width_px) = best_layout(vec2(1200.0, 700.0), 4);
        assert_eq!(spalten, 2, "2x2 haette die groesseren Kacheln");
        assert!(width_px > 300.0, "Kachel zu klein: {}", width_px);
        // One alone gets at most 70 % of the width - otherwise it looks
        // like an error, not like a meeting.
        let (_, alone) = best_layout(vec2(1200.0, 700.0), 1);
        assert!(alone <= 1200.0 * 0.7 + 0.5, "allein zu breit: {}", alone);
    }

    #[test]
    fn layout_does_not_crash_on_zero() {
        let (s, b) = best_layout(vec2(800.0, 600.0), 0);
        assert_eq!(s, 1);
        assert_eq!(b, 0.0);
    }

    /// The section must ALWAYS lie completely within the image - otherwise you see
    /// black stripes at the edge or stretched pixels.
    #[test]
    fn zoom_crop_stays_in_frame() {
        for zoom in [1.0f32, 1.5, 2.0, 3.7, 8.0] {
            for (mx, my) in [(0.0f32, 0.0f32), (1.0, 1.0), (0.5, 0.5), (-3.0, 9.0)] {
                let a = zoom_crop(zoom, vec2(mx, my));
                assert!(a.min.x >= -0.0001 && a.min.y >= -0.0001, "links/oben raus: {:?}", a);
                assert!(a.max.x <= 1.0001 && a.max.y <= 1.0001, "rechts/unten raus: {:?}", a);
                let expected = 1.0 / zoom.clamp(1.0, 8.0);
                assert!(
                    (a.width() - expected).abs() < 0.001,
                    "Breite {} statt {}",
                    a.width(),
                    expected
                );
            }
        }
    }

    /// If the other already sends exactly the wanted window, the
    /// incoming image is to be shown COMPLETELY - then they are real pixels.
    #[test]
    fn sharp_zoom_shows_the_whole_image() {
        let window = Rect::from_min_size(pos2(0.25, 0.25), vec2(0.5, 0.5));
        let uv = zoom_uv(window, window);
        assert!((uv.min.x).abs() < 1e-5 && (uv.min.y).abs() < 1e-5);
        assert!((uv.max.x - 1.0).abs() < 1e-5 && (uv.max.y - 1.0).abs() < 1e-5);
    }

    /// If it sends everything, the magnifier behaves as before.
    #[test]
    fn without_crop_the_loupe_stays_as_before() {
        let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let window = Rect::from_min_size(pos2(0.25, 0.25), vec2(0.5, 0.5));
        assert_eq!(zoom_uv(window, whole), window);
    }

    /// Halfway: it sends more than wanted - then the rest is
    /// cut away, but NO piece is zoomed twice.
    #[test]
    fn larger_crop_is_computed_correctly() {
        let region = Rect::from_min_size(pos2(0.2, 0.2), vec2(0.6, 0.6));
        let window = Rect::from_min_size(pos2(0.35, 0.35), vec2(0.3, 0.3));
        let uv = zoom_uv(window, region);
        assert!((uv.min.x - 0.25).abs() < 1e-5, "links falsch: {}", uv.min.x);
        assert!((uv.width() - 0.5).abs() < 1e-5, "Breite falsch: {}", uv.width());
    }

    /// If the viewer drags faster than the sender switches, nothing may
    /// run out of the image - better to stay at the edge.
    #[test]
    fn window_outside_stays_in_frame() {
        let region = Rect::from_min_size(pos2(0.5, 0.5), vec2(0.2, 0.2));
        let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(0.2, 0.2));
        let uv = zoom_uv(window, region);
        assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0);
        assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
        assert!(uv.width() > 0.0 && uv.height() > 0.0, "leere Flaeche waere schwarz");
    }

    #[test]
    fn zoom_one_shows_all() {
        let a = zoom_crop(1.0, vec2(0.2, 0.9));
        assert_eq!(a, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
    }

    /// Double magnification in the middle = exactly the middle quarter.
    #[test]
    fn zoom_two_takes_the_center() {
        let a = zoom_crop(2.0, vec2(0.5, 0.5));
        assert_eq!(a, Rect::from_min_max(pos2(0.25, 0.25), pos2(0.75, 0.75)));
    }

    #[test]
    fn zoom_anchor_rotates_in_circle() {
        let mut a = ZoomAnchor::Own;
        for _ in 0..3 {
            a = a.advance();
        }
        assert_eq!(a, ZoomAnchor::Own);
        assert!(!ZoomAnchor::Divider.word().is_empty());
    }

    #[test]
    fn camera_slot_rotates_in_circle() {
        let mut k = CameraSlot::Page;
        k = k.advance();
        assert_eq!(k, CameraSlot::Bottom);
        k = k.advance();
        assert_eq!(k, CameraSlot::Aus);
        k = k.advance();
        assert_eq!(k, CameraSlot::Page);
    }

    #[test]
    fn each_example_has_people_and_a_name() {
        for i in 0..EXAMPLES {
            let (name, view_state, _z) = example(i);
            assert!(!name.is_empty(), "Beispiel {} ohne Namen", i);
            assert!(!view_state.people.is_empty(), "Beispiel {} ohne Teilnehmer", i);
            assert!(view_state.people[0].ich, "Beispiel {}: ich stehe nicht vorn", i);
        }
    }

    #[test]
    fn short_cuts_only_when_needed() {
        assert_eq!(short("kurz", 10), "kurz");
        assert_eq!(short("abcdefghijkl", 5), "abcd…");
    }
}
