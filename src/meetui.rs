//! Native meeting - stage 1e: the UI for it.
//!
//! Ties the three building blocks together (signalling, WebRTC audio, devices)
//! and draws them into the existing meeting window: participant list,
//! mute, raised hand, chat, waiting room. Video arrives in stage 2 -
//! so the browser path stays alongside and is only removed once the
//! native client can do EVERYTHING.

use crate::meetaudio;
use crate::meetrtc;
use crate::meetsig;
use anyhow::Result;

// ------------------------------------------------------- Bitrates
// WHY SO LOW: this used to be 2 Mbit/s for a 640x360 camera
// and 4 Mbit/s for the screen - 6 Mbit/s upload together. Hardly any
// home line sustains that, and because the client did not estimate
// the bandwidth it kept sending anyway. The packets queued up in the
// router, and that is exactly where the 5 to 10 seconds of
// delay came from. A browser gives a 640x360 camera 0.3 to 0.7 Mbit/s.
/// Camera 640x360 at 30 frames/s.
pub const CAMERA_BITRATE: u32 = 700_000;
/// Avatar - drawn flat areas, they cost almost nothing.
pub const AVATAR_BITRATE: u32 = 400_000;
/// Screen up to 1920x1080 at 20 frames/s. Screen content is
/// largely uniform and compresses well.
pub const SCREEN_BITRATE: u32 = 1_800_000;

/// Lower and upper bounds - below the lower it looks like mush,
/// above the upper more brings nothing.
pub const CAMERA_MIN: u32 = 150_000;
pub const CAMERA_MAX: u32 = 1_200_000;
pub const SCREEN_MIN: u32 = 300_000;
pub const SCREEN_MAX: u32 = 3_000_000;

/// Splits the measured send bandwidth between camera and screen.
///
/// WHY 85 % AND NOT 100 %: audio, RTCP and signalling share the
/// same line, and an estimate is an estimate. Pushing it to the
/// limit fills the router again at the first fluctuation -
/// and then the delay is back.
///
/// The screen gets the larger share: it shows text that has to
/// be readable. A grainy face bothers less than an
/// unreadable line of source code.
///
/// `estimate` = 0 means "no measurement" - then the
/// defaults stay in place.
pub fn bitrates_distribute(estimate: u64, camera: bool, screen: bool) -> (u32, u32) {
    if estimate == 0 {
        return (CAMERA_BITRATE, SCREEN_BITRATE);
    }
    let usable = (estimate as f64 * 0.85) as u32;
    let (k, s) = match (camera, screen) {
        (true, true) => ((usable as f64 * 0.30) as u32, (usable as f64 * 0.70) as u32),
        (true, false) => (usable, SCREEN_BITRATE),
        (false, true) => (CAMERA_BITRATE, usable),
        (false, false) => (CAMERA_BITRATE, SCREEN_BITRATE),
    };
    (
        k.clamp(CAMERA_MIN, CAMERA_MAX),
        s.clamp(SCREEN_MIN, SCREEN_MAX),
    )
}

/// Is it worth rebuilding the encoder? That costs a
/// keyframe, so not for every small change - only from a
/// third difference upwards.
pub fn bitrate_worth_switch(now_ts: u32, dest: u32) -> bool {
    let (a, b) = (now_ts.max(1) as f64, dest.max(1) as f64);
    (a - b).abs() / a >= 0.34
}

pub struct NativeMeet {
    sig: meetsig::Session,
    ton: meetrtc::Ton,
    devices: Option<meetaudio::Devices>,
    /// Chatverlauf: (Absender, Text) - Absender 0 = System.
    pub chat: Vec<(u64, String)>,
    pub input_text: String,
    offered: bool,
    pub hand: bool,
    pub is_muted: bool,
    /// Level of the own microphone (0..1), for display.
    pub level: f32,
    /// Letzte Notice (FriendError, Hinweise).
    pub notice: String,
    pub room: String,
    /// How many video frames have arrived so far (stage 2 shows them later).
    pub image_counter: u64,
    /// Format of the incoming frames ("H264"/"Vp8") - for display.
    pub image_codec: String,
    /// Decoded CAMERA frames of the others (the latest per participant).
    /// What the camera is currently encoding at (bit/s).
    pub encoder_bitrate: u32,
    /// What the screen is currently encoding at (bit/s).
    pub screen_bitrate: u32,
    /// Wann zuletzt nachgerechnet wurde.
    bitrate_checked: std::time::Instant,
    /// How often the bitrate has been adjusted - for measuring only.
    pub bitrate_switch: u32,
    pub images: crate::meetvideo::VideoDecoder,
    /// Decoded SCREEN frames of the others. Separate bucket, otherwise
    /// the share would overwrite the same participant's face.
    pub screens: crate::meetvideo::VideoDecoder,
    /// Own camera (stage 2d) - only runs when it is switched on.
    camera: Option<crate::meetcam::WebCam>,
    koder: Option<crate::meetvideo::VideoEncoder>,
    pub camera_on: bool,
    /// Send a drawn avatar instead of the camera.
    pub avatar_an: bool,
    pub avatar: crate::avatar::Look,
    /// Smoothed mouth position (0..1) - the raw level jitters too much.
    avatar_mouth: f32,
    /// Counts frames; blinking and breathing are derived from it.
    avatar_beat: u64,
    /// At which tick the next blink is due.
    avatar_blinks: u64,
    avatar_rgb: Vec<u8>,
    avatar_nv12: Vec<u8>,
    /// Which devices are currently in use (empty = system default).
    /// WHY remembered: only this allows switching during a running meeting
    /// without leaving the room - just like in the browser client.
    /// Is end-to-end encryption running? Only then may the badge
    /// in the window turn green - a green badge without real
    /// encryption would be the worst kind of lie.
    pub e2e_an: bool,
    /// A kick/rejection has already set the reason - do not overwrite it.
    pub reason_stands: bool,
    pub camera_device: Option<String>,
    pub mic_device: Option<String>,
    pub speaker_device: Option<String>,
    pub camera_notice: String,
    /// Own image for the preview: (width, height, RGBA).
    pub own: Option<(u32, u32, Vec<u8>)>,
    /// Counts up when the preview has a new image (texture reload).
    pub own_state: u64,
    /// How many own frames have gone out so far.
    pub image_sent: u64,
    /// When the next frame may go out (15 frames per second).
    next_image: std::time::Instant,
    /// Video track is registered with the server.
    video_reported: bool,
    /// Screen share (stage 3) - own track next to the camera.
    screen: Option<crate::meetschirm::Recording>,
    screen_encoder: Option<crate::meetvideo::VideoEncoder>,
    pub screen_on: bool,
    /// The red frame around whatever is currently going out. Lives exactly as long
    /// as the share; it disappears by itself when dropped.
    frame_box: Option<crate::window::FrameBox>,
    /// Noise gate: on? And how strict (0..100)? Kept here
    /// so that a device change does not forget the setting - the
    /// audio devices are rebuilt completely in the process.
    pub lock_on: bool,
    pub lock_strength: u8,
    /// Should a frame be shown at all? Some people do not want it.
    pub frame_on: bool,
    /// Where the frame was last placed - needless moving
    /// would be flicker.
    frame_position: Option<(i32, i32, i32, i32)>,
    pub screen_notice: String,
    pub screen_sent: u64,
    /// The own shared image as RGBA - so you can see for YOURSELF what you
    /// are sharing. The browser client does exactly that ("otherwise you do not
    /// see WHAT you are sharing"); native lacked it, and a click on
    /// "Screen" therefore looked as if nothing happened at all.
    pub own_screen: Option<(u32, u32, Vec<u8>)>,
    pub own_screen_state: u64,
    next_screen_image: std::time::Instant,
    /// Wishes of the viewers: who wants which section of my
    /// screen? (participant -> area, plus when last heard from)
    wish_from: std::collections::HashMap<u64, (crate::meetschirm::Region, std::time::Instant)>,
    /// Welchen Ausschnitt SENDE ich gerade?
    pub screen_region: crate::meetschirm::Region,
    /// Which section are the OTHERS currently sending (for drawing)?
    pub region_from: std::collections::HashMap<u64, crate::meetschirm::Region>,
    /// Who can deliver sections at all (new client)? Whoever has
    /// reported once can - before that, "armed" would be a lie.
    pub sharp_can: std::collections::HashSet<u64>,
    /// Last sent wish and when - repeats are useless.
    last_wish: Option<(u64, crate::meetschirm::Region)>,
    next_wish: std::time::Instant,
    /// Pointer positions of the others (0..1 on THEIR shared screen).
    /// This lets a viewer zoom to where the other one is pointing.
    pub pointer_from: std::collections::HashMap<u64, (f32, f32)>,
    /// When the next own pointer state may go out (10 times per second).
    next_pointer: std::time::Instant,
    /// Last sent state - unchanged values are not sent.
    last_pointer: Option<(f32, f32)>,
    /// When a keyframe was last requested.
    last_keyframe: std::time::Instant,
    /// Own FreeViewer number - we announce it when sharing.
    ich_fvid: String,
    /// Stage 4: remote control released for the others?
    pub control_free: bool,
    /// Open control requests TO ME: (participant, name). They only
    /// arrive while I am really sharing - the server checks that.
    control_request: Vec<(u64, String)>,
    /// Whom I have allowed to control my shared screen.
    /// EXACTLY these people, not the whole room.
    control_allowed: Vec<u64>,
    /// I asked myself and am waiting for the decision.
    pub control_asked: bool,
    /// Approval received: (participant, FreeViewer number of the sharer).
    /// Only with this can a real FreeViewer session be set up.
    control_grant: Option<(u64, String)>,
    /// Whom the server currently reports as speaker.
    pub talker: Option<u64>,
    /// How loud each other participant currently is (0..1). The green
    /// outline of the tile comes from this - the server does not always report the speaker.
    pub level_from: std::collections::HashMap<u64, f32>,
    /// Who is currently typing (for "is typing ..." in the chat).
    pub type_text: std::collections::HashSet<u64>,
    /// Log for the "Info" tab - what happened in the room.
    pub protocol: Vec<String>,
    /// Gemessener Durchsatz in kbit/s (aus den Byte-Zaehlern, je Sekunde).
    pub kbit_out: u32,
    pub kbit_rein: u32,
    /// Last measurement for the bandwidth.
    last_measurement: std::time::Instant,
    last_bytes: (u64, u64),
}

impl NativeMeet {
    /// Beitreten. `mic_name`/`speaker` leer = Standardgeraet.
    pub fn join_meeting(
        basis: &str,
        room: &str,
        pass: &str,
        name: &str,
        fvid: &str,
        control: bool,
        mic_name: Option<String>,
        speaker: Option<String>,
        e2e: Option<String>,
    ) -> Result<NativeMeet> {
        // We ALWAYS remember our own number (otherwise the share
        // could not be switched on later) - it is only announced
        // when "offer remote control" is really on.
        // End-to-end: the key comes from the link (fragment). It must be
        // fixed BEFORE joining - the server learns in the join message
        // whether we encrypt, and from that decides
        // who may enter the room. Without a key everything stays as before.
        let e2e_key = e2e
            .as_deref()
            .and_then(crate::meete2e::Key::aus_text);
        let e2e_an = e2e_key.is_some();
        let sig = meetsig::join_meeting(
            basis,
            room,
            pass,
            name,
            if control { fvid } else { "" },
            e2e_an,
        )?;
        let ton = meetrtc::start_with(e2e_key)?;
        // Without a sound card (server, test machine) the meeting still runs -
        // you just hear nothing. Report honestly instead of aborting.
        let (mic_remember, speaker_remember) = (mic_name.clone(), speaker.clone());
        let (devices, notice) = match meetaudio::start_devices(mic_name, speaker) {
            Ok(g) => {
                let m = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.exit);
                (Some(g), m)
            }
            Err(e) => (None, format!("Kein Ton-Geraet: {}", e)),
        };
        Ok(NativeMeet {
            sig,
            ton,
            devices,
            chat: Vec::new(),
            input_text: String::new(),
            offered: false,
            hand: false,
            is_muted: false,
            level: 0.0,
            notice,
            room: room.to_string(),
            image_counter: 0,
            image_codec: String::new(),
            encoder_bitrate: CAMERA_BITRATE,
            screen_bitrate: SCREEN_BITRATE,
            bitrate_checked: std::time::Instant::now(),
            bitrate_switch: 0,
            images: crate::meetvideo::VideoDecoder::neu(),
            screens: crate::meetvideo::VideoDecoder::neu(),
            camera: None,
            koder: None,
            camera_on: false,
            avatar_an: false,
            avatar: crate::avatar::Look::default(),
            avatar_mouth: 0.0,
            avatar_beat: 0,
            avatar_blinks: 90,
            avatar_rgb: Vec::new(),
            avatar_nv12: Vec::new(),
            e2e_an,
            reason_stands: false,
            camera_device: None,
            mic_device: mic_remember,
            speaker_device: speaker_remember,
            camera_notice: String::new(),
            own: None,
            own_state: 0,
            image_sent: 0,
            next_image: std::time::Instant::now(),
            video_reported: false,
            screen: None,
            screen_encoder: None,
            screen_on: false,
            frame_box: None,
            lock_on: true,
            lock_strength: 45,
            frame_on: true,
            frame_position: None,
            screen_notice: String::new(),
            screen_sent: 0,
            own_screen: None,
            own_screen_state: 0,
            next_screen_image: std::time::Instant::now(),
            wish_from: std::collections::HashMap::new(),
            screen_region: crate::meetschirm::WHOLE,
            region_from: std::collections::HashMap::new(),
            sharp_can: std::collections::HashSet::new(),
            last_wish: None,
            next_wish: std::time::Instant::now(),
            pointer_from: std::collections::HashMap::new(),
            next_pointer: std::time::Instant::now(),
            last_pointer: None,
            last_keyframe: std::time::Instant::now(),
            ich_fvid: fvid.to_string(),
            control_free: control && fvid.chars().any(|c| c.is_ascii_digit()),
            control_request: Vec::new(),
            control_allowed: Vec::new(),
            control_asked: false,
            control_grant: None,
            talker: None,
            level_from: std::collections::HashMap::new(),
            type_text: std::collections::HashSet::new(),
            protocol: vec![format!("Raum {} - Beitritt laeuft", room)],
            kbit_out: 0,
            kbit_rein: 0,
            last_measurement: std::time::Instant::now(),
            last_bytes: (0, 0),
        })
    }

    pub fn state(&self) -> meetsig::State {
        self.sig.state()
    }

    pub fn numbers(&self) -> meetrtc::Numbers {
        self.ton.numbers()
    }

    /// Must be called every frame: dispatches messages and audio.
    pub fn pumpe(&mut self) {
        for e in self.sig.poll_events() {
            match e {
                meetsig::Event::Welcome { .. } => {
                    if !self.offered {
                        self.offered = true;
                        self.sig
                            .roh(serde_json::json!({"t":"offer","sdp":self.ton.offer}));
                    }
                }
                meetsig::Event::Sdp { art, sdp } => {
                    if art == "answer" {
                        self.ton.reply(&sdp);
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.mid,"screen":false}),
                        );
                        // Also register the video track - otherwise the server
                        // would not know later where to put our camera image. Until the
                        // camera is on, it counts as switched off (the others
                        // then see a placeholder instead of black).
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid,"screen":false}),
                        );
                        // Register the screen track AS SUCH right away -
                        // otherwise the other side takes it for a second
                        // camera as soon as data flows later. A tile
                        // is only created once real video arrives.
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid2,"screen":true}),
                        );
                        self.video_reported = true;
                        self.sig.is_muted("video", !self.camera_on);
                    } else {
                        self.ton.server_offer(&sdp);
                    }
                }
                meetsig::Event::Spur {
                    mid,
                    peer,
                    display,
                    ..
                } => self.ton.spur_art(&mid, peer, display),
                meetsig::Event::Chat { von, text, .. } => self.chat.push((von, text)),
                meetsig::Event::Also(t) => {
                    self.chat.push((0, format!("{} ist dazugekommen", t.name)))
                }
                meetsig::Event::Weg(id) => {
                    self.images.forget(id);
                    self.screens.forget(id);
                    // Whoever is gone asks nothing anymore and may do nothing anymore.
                    self.control_request.retain(|(p, _)| *p != id);
                    self.control_allowed.retain(|p| *p != id);
                    if self.control_grant.as_ref().map(|(p, _)| *p) == Some(id) {
                        self.control_grant = None;
                    }
                    let name = self
                        .sig
                        .state()
                        .people
                        .iter()
                        .find(|x| x.id == id)
                        .map(|x| x.name.clone())
                        .unwrap_or_else(|| format!("#{}", id));
                    self.chat.push((0, format!("{} ist gegangen", name)));
                }
                meetsig::Event::ForceMuted { art } => {
                    if art == "audio" {
                        self.is_muted = true;
                        if let Some(g) = &self.devices {
                            g.is_muted.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.ton.is_muted(true);
                        self.chat
                            .push((0, "Der Gastgeber hat dich stummgeschaltet".into()));
                    }
                }
                meetsig::Event::Kicked(m)
                | meetsig::Event::Ended(m)
                | meetsig::Event::Refused(m) => {
                    // The REASON is the valuable part ("Meeting is end-to-end
                    // encrypted - please update"). It must stay
                    // put; the line is cut right after.
                    self.notice = m;
                    self.reason_stands = true;
                }
                meetsig::Event::RemoteControl { peer, fvid } => {
                    // Do not report about the own share itself.
                    let ich = self.sig.state().ich;
                    // Only STARTING the share is worth a message. Stopping it
                    // used to show up in the chat as well - that is
                    // noise, the button visibly disappears anyway.
                    if peer != ich && !fvid.is_empty() {
                        let wer = self.name_von(peer);
                        self.chat
                            .push((0, format!("{} erlaubt Fernsteuerung ({})", wer, fvid)));
                    }
                }
                meetsig::Event::ControlRequested { peer, name } => {
                    // Asked twice is not asked twice.
                    if !self.control_request.iter().any(|(id, _)| *id == peer) {
                        self.control_request.push((peer, name.clone()));
                    }
                    self.chat
                        .push((0, format!("{} bittet um die Steuerung", name)));
                }
                meetsig::Event::ControlAnswered {
                    von,
                    granted,
                    fvid,
                } => {
                    self.control_asked = false;
                    let wer = self.name_von(von);
                    if granted {
                        // Without a number the approval is worthless - better to
                        // report honestly than to offer a button that
                        // leads nowhere.
                        if fvid.chars().any(|c| c.is_ascii_digit()) {
                            self.control_grant = Some((von, fvid));
                            self.chat
                                .push((0, format!("{} erlaubt dir die Steuerung", wer)));
                        } else {
                            self.notice =
                                format!("{} hat zugesagt, hat aber keine FreeViewer-Nummer", wer);
                        }
                    } else {
                        self.control_grant = None;
                        self.chat
                            .push((0, format!("{} hat die Steuerung abgelehnt", wer)));
                    }
                }
                meetsig::Event::ControlGiveBack { .. } => {
                    // The server sends this only to the person affected - so it is
                    // always about MY share.
                    if self.control_grant.take().is_some() {
                        self.chat
                            .push((0, "Die Steuerungsfreigabe wurde zurueckgenommen".into()));
                    }
                }
                meetsig::Event::Pointer { peer, x, y } => {
                    self.pointer_from.insert(peer, (x, y));
                }
                meetsig::Event::RegionWish { peer, x, y, w, h } => {
                    // A viewer says which part of MY screen they are
                    // currently looking at. Remember it - it is applied in bulk
                    // so that several viewers do not chase each other.
                    self.wish_from
                        .insert(peer, ((x, y, w, h), std::time::Instant::now()));
                }
                meetsig::Event::Region { peer, x, y, w, h } => {
                    self.sharp_can.insert(peer);
                    self.region_from.insert(peer, (x, y, w, h));
                }
                meetsig::Event::FriendError { code, text } => {
                    self.notice = format!("{}: {}", code, text);
                }
                meetsig::Event::Talker(p) => self.talker = p,
                meetsig::Event::Types { peer, an } => {
                    if an {
                        self.type_text.insert(peer);
                    } else {
                        self.type_text.remove(&peer);
                    }
                }
                meetsig::Event::Welcome { room, server, .. } => {
                    self.protocol
                        .push(format!("Im Raum {} - Server {}", room, server));
                }
                meetsig::Event::Separated(m) => {
                    // If a real reason is already there, it stays -
                    // "Connection ended" would be the worse answer.
                    if !self.reason_stands {
                        self.notice = if m.is_empty() {
                            "Verbindung beendet".into()
                        } else {
                            format!("Verbindung weg: {}", m)
                        };
                    }
                }
                _ => {}
            }
        }
        if let Some(a) = self.ton.open_reply() {
            self.sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }

        // Mikrofon -> Netz
        if let Some(g) = &self.devices {
            while let Ok(frame_box) = g.mic_name.try_recv() {
                let spitze = frame_box
                    .iter()
                    .map(|v| (*v as f32 / 32768.0).abs())
                    .fold(0.0f32, f32::max);
                self.level = self.level * 0.7 + spitze * 0.3;
                self.ton.send_out(frame_box);
            }
        }
        // Netz -> Lautsprecher
        for te in self.ton.poll_events() {
            match te {
                meetrtc::ToneEvent::FrameBox { origin, pcm } => {
                    // Remember the volume per participant: it becomes the
                    // green outline "is speaking". The server reports
                    // the speaker only on changes; this is available immediately.
                    let spitze = pcm
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max);
                    let e = self.level_from.entry(origin).or_insert(0.0);
                    *e = e.max(spitze);
                    if let Some(g) = &self.devices {
                        if let Ok(mut m) = g.speaker.lock() {
                            m.also(origin, &pcm);
                        }
                    }
                }
                meetrtc::ToneEvent::Image {
                    origin,
                    display,
                    daten,
                    keyframe,
                    codec,
                } => {
                    self.image_codec = codec;
                    // Stage 2: decoding is done by the platform later.
                    // For now we only count, so you can see that
                    // video really arrives.
                    self.image_counter += 1;
                    let _ = keyframe;
                    if display {
                        self.screens.frame_box(origin, &daten);
                    } else {
                        self.images.frame_box(origin, &daten);
                    }
                }
                meetrtc::ToneEvent::FriendError(f) => self.notice = f,
                meetrtc::ToneEvent::End(f) => {
                    if !f.is_empty() {
                        self.notice = f;
                    }
                }
                meetrtc::ToneEvent::PeerConnected => {
                    self.notice = "Ton verbunden".into();
                }
            }
        }
        self.bitrate_pumpe();
        self.camera_pump();
        self.screen_pump();
        self.pointer_pump();
        self.region_pump();
        self.measure();
    }

    /// Once per second: compute throughput, let the level decay.
    /// WHY measured and not estimated: the badge above should show a
    /// number that is really correct - guessed it would be worthless.
    fn measure(&mut self) {
        for v in self.level_from.values_mut() {
            *v *= 0.90;
        }
        let now_ts = std::time::Instant::now();
        let dt = now_ts.duration_since(self.last_measurement).as_secs_f32();
        if dt < 1.0 {
            return;
        }
        let z = self.ton.numbers();
        let (old_out, alt_rein) = self.last_bytes;
        self.kbit_out = ((z.bytes_out.saturating_sub(old_out) as f32 * 8.0) / dt / 1000.0) as u32;
        self.kbit_rein = ((z.bytes_rein.saturating_sub(alt_rein) as f32 * 8.0) / dt / 1000.0) as u32;
        self.last_bytes = (z.bytes_out, z.bytes_rein);
        self.last_measurement = now_ts;
    }

    /// Spricht dieser Participant gerade?
    pub fn speaking(&self, id: u64) -> bool {
        self.talker == Some(id) || self.level_from.get(&id).copied().unwrap_or(0.0) > 0.06
    }

    /// Gastgeber-Befehle: "mute-all" | "mute" | "kick" | "end".
    pub fn host_action(&self, action: &str, peer: Option<u64>) {
        self.sig.host_action(action, peer);
    }

    /// Report "is typing" to the others.
    pub fn types(&self, an: bool) {
        self.sig.types(an);
    }

    /// Screen -> H.264 -> meeting (own track). 15 frames per second,
    /// always the newest frame; a keyframe every 4 seconds so that
    /// late joiners do not face an empty area.
    /// Report the own pointer position while the screen is shared.
    ///
    /// 10 times per second and only on a REAL change: a few
    /// dozen bytes, but they let the viewer zoom exactly to
    /// where it is pointed.
    fn pointer_pump(&mut self) {
        if !self.screen_on {
            return;
        }
        let now_ts = std::time::Instant::now();
        if now_ts < self.next_pointer {
            return;
        }
        self.next_pointer = now_ts + std::time::Duration::from_millis(100);
        let origin = match self.screen.as_ref() {
            Some(a) => a.origin,
            None => return,
        };
        if let Some((x, y)) = crate::meetschirm::pointer_share_source(origin) {
            let neu = (x, y);
            let other = match self.last_pointer {
                Some((ax, ay)) => (ax - x).abs() > 0.002 || (ay - y).abs() > 0.002,
                None => true,
            };
            if other {
                self.last_pointer = Some(neu);
                self.sig.pointer(x, y);
            }
        }
    }

    fn screen_pump(&mut self) {
        let now_ts = std::time::Instant::now();
        // Move the frame along BEFORE we abort due to the frame rate -
        // a window can be moved faster than a frame goes out
        // 20 times per second, and a lagging frame
        // looks broken.
        self.frame_pump();
        if now_ts < self.next_screen_image {
            return;
        }
        let (auf, kod) = match (self.screen.as_ref(), self.screen_encoder.as_mut()) {
            (Some(a), Some(k)) => (a, k),
            _ => return,
        };
        let picture = match auf.newest() {
            Some(b) => b,
            None => return,
        };
        // 50 ms = 20 frames per second. More than text and windows need,
        // but clearly smoother than the earlier 15 - and a shared
        // screen costs less per frame than a camera (much stays the same).
        self.next_screen_image = now_ts + std::time::Duration::from_millis(50);
        if now_ts.duration_since(self.last_keyframe) >= std::time::Duration::from_secs(4) {
            self.last_keyframe = now_ts;
            kod.keyframe();
        }
        match kod.nv12_frame(&picture.nv12) {
            Ok(parts) => {
                for t in parts {
                    self.ton.screen_send(t.data);
                }
                self.screen_sent += 1;
            }
            Err(e) => self.screen_notice = format!("Kodierer: {}", e),
        }
        // Own preview: only every SECOND frame (10 times per second). A
        // screen is large, converting to RGBA costs more than for
        // the camera - and for checking the coarse version is enough.
        if self.screen_sent % 2 == 0 {
            let mut rgba = Vec::new();
            if crate::h264::nv12_to_rgba(
                &picture.nv12,
                picture.width_px,
                picture.height_px,
                picture.width_px as usize,
                picture.height_px,
                &mut rgba,
            ) {
                self.own_screen = Some((picture.width_px, picture.height_px, rgba));
                self.own_screen_state += 1;
            }
        }
        let f = auf.error();
        if !f.is_empty() {
            self.screen_notice = f;
        }
    }

    /// Track the red frame of the shared source.
    ///
    /// With a screen nothing changes, with a window constantly - it
    /// moves, grows, gets minimised. If it disappears, the
    /// frame is removed instead of staying in a wrong place.
    fn frame_pump(&mut self) {
        if !self.screen_on {
            return;
        }
        let Some(a) = self.screen.as_ref() else { return };
        let origin = a.origin;
        match (self.frame_on, origin.lage()) {
            (true, Some(lage)) => {
                if self.frame_box.is_none() {
                    self.frame_box = Some(crate::window::FrameBox::display_items(
                        lage.0, lage.1, lage.2, lage.3,
                    ));
                    self.frame_position = Some(lage);
                } else if self.frame_position != Some(lage) {
                    if let Some(r) = self.frame_box.as_ref() {
                        r.setzen(lage.0, lage.1, lage.2, lage.3);
                    }
                    self.frame_position = Some(lage);
                }
            }
            _ => {
                self.frame_box = None;
                self.frame_position = None;
            }
        }
    }

    /// Switch the red frame on or off. Takes effect immediately.
    pub fn frame_toggle(&mut self, an: bool) {
        self.frame_on = an;
        if !an {
            self.frame_box = None;
            self.frame_position = None;
        }
    }

    /// Which program windows can be shared?
    pub fn window() -> Vec<crate::window::WindowInfo> {
        crate::window::liste()
    }

    /// How many screens are there here?
    pub fn monitore() -> Vec<crate::meetschirm::Screen> {
        crate::meetschirm::liste()
    }

    /// Screen share on/off. `index` selects the screen.
    /// Switch the screen share. Returns a MESSAGE if it
    /// did not work - it belongs in front of the user, not just
    /// in the log. This is exactly why a failure looked like
    /// "nothing happens".
    pub fn report_toggle_screen(&mut self, an: bool, index: usize) -> Option<String> {
        self.source_toggle_report(an, crate::meetschirm::Source::Display(index))
    }

    /// Like `report_toggle_screen`, but the source may also be a single
    /// program window.
    pub fn source_toggle_report(
        &mut self,
        an: bool,
        origin: crate::meetschirm::Source,
    ) -> Option<String> {
        self.source_toggle(an, origin);
        if an && !self.screen_on {
            let m = self.screen_notice.clone();
            return Some(if m.is_empty() {
                "Bildschirm laesst sich nicht teilen".to_string()
            } else {
                m
            });
        }
        None
    }

    pub fn toggle_screen(&mut self, an: bool, index: usize) {
        self.source_toggle(an, crate::meetschirm::Source::Display(index))
    }

    /// Freigabe einer beliebigen Quelle schalten.
    pub fn source_toggle(&mut self, an: bool, origin: crate::meetschirm::Source) {
        if an {
            if self.screen.is_some() {
                return;
            }
            match crate::meetschirm::open_source(origin, 1920, 1080, 15) {
                Ok(a) => match crate::meetvideo::VideoEncoder::neu(a.width_px, a.height_px, 20, SCREEN_BITRATE) {
                    Ok(k) => {
                        self.screen_notice = format!("Teile {}", a.name);
                        self.screen = Some(a);
                        self.screen_encoder = Some(k);
                        self.screen_bitrate = SCREEN_BITRATE;
                        self.screen_on = true;
                        self.screen_sent = 0;
                        self.last_keyframe =
                            std::time::Instant::now() - std::time::Duration::from_secs(9);
                        // First register the track as screen, then announce
                        // the share - in that order, otherwise the other side
                        // creates the tile wrongly.
                        self.sig.roh(
                            serde_json::json!({"t":"publish","mid":self.ton.vid2,"screen":true}),
                        );
                        self.sig.roh(serde_json::json!({"t":"screen","on":true}));
                        // Outline in red whatever goes out. The frame lies
                        // above everything, accepts no clicks and is excluded from
                        // the own capture - otherwise the viewer would see
                        // it twice.
                        self.frame_box = None;
                        self.frame_position = None;
                        if self.frame_on {
                            if let Some((x, y, b, h)) = origin.lage() {
                                self.frame_box = Some(crate::window::FrameBox::display_items(x, y, b, h));
                                self.frame_position = Some((x, y, b, h));
                            }
                        }
                    }
                    Err(e) => {
                        a.stop_capture();
                        self.screen_notice = format!("Kein Kodierer: {}", e);
                        self.screen_on = false;
                    }
                },
                Err(e) => {
                    self.screen_notice = format!("Kein Bildschirm: {}", e);
                    self.screen_on = false;
                }
            }
        } else {
            if let Some(a) = self.screen.take() {
                a.stop_capture();
            }
            self.screen_encoder = None;
            self.screen_on = false;
            self.frame_box = None;
            self.frame_position = None;
            self.own_screen = None;
            self.own_screen_state += 1;
            self.screen_notice = String::new();
            // Whoever no longer shares also releases nothing anymore. Open
            // requests are moot, granted releases must go -
            // otherwise a permission would remain that nobody sees anymore.
            if !self.control_allowed.is_empty() {
                self.control_allowed.clear();
                self.sig.revoke_control(None);
            }
            self.control_request.clear();
            self.sig.roh(serde_json::json!({"t":"screen","on":false}));
        }
    }

    /// Name of the running screen share (empty = off).
    pub fn screen_name(&self) -> String {
        self.screen.as_ref().map(|a| a.name.clone()).unwrap_or_default()
    }

    /// Camera -> H.264 -> meeting. Runs at 15 frames per second; the
    /// frame is taken on pickup, not queued.
    /// Follow the bitrate of the measured line.
    ///
    /// The encoder cannot be reconfigured while running, so it is
    /// rebuilt. That costs a keyframe - hence at most
    /// every three seconds and only from a third difference upwards.
    fn bitrate_pumpe(&mut self) {
        if self.bitrate_checked.elapsed() < std::time::Duration::from_secs(3) {
            return;
        }
        self.bitrate_checked = std::time::Instant::now();
        let estimate = self.ton.numbers().bandwidth;
        let (k, s) = bitrates_distribute(estimate, self.camera_on, self.screen_on);
        if self.koder.is_some() && bitrate_worth_switch(self.encoder_bitrate, k) {
            let (b, h, fps) = match (&self.camera, self.avatar_an) {
                (Some(kam), _) => (kam.width_px, kam.height_px, 30),
                (None, true) => (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 20),
                _ => (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 30),
            };
            if let Ok(c) = crate::meetvideo::VideoEncoder::neu(b, h, fps, k) {
                self.koder = Some(c);
                self.encoder_bitrate = k;
                self.bitrate_switch += 1;
            }
        }
        if self.screen_encoder.is_some() && bitrate_worth_switch(self.screen_bitrate, s) {
            if let Some(a) = self.screen.as_ref() {
                if let Ok(c) = crate::meetvideo::VideoEncoder::neu(a.width_px, a.height_px, 20, s) {
                    self.screen_encoder = Some(c);
                    self.screen_bitrate = s;
                    self.bitrate_switch += 1;
                }
            }
        }
    }

    fn camera_pump(&mut self) {
        let now_ts = std::time::Instant::now();
        if now_ts < self.next_image {
            return;
        }
        if self.avatar_an {
            self.avatar_pumpe(now_ts);
            return;
        }
        let (kam, kod) = match (self.camera.as_ref(), self.koder.as_mut()) {
            (Some(k), Some(c)) => (k, c),
            _ => return,
        };
        let picture = match kam.newest() {
            Some(b) => b,
            None => return,
        };
        // 33 ms = 30 frames per second. Before it was 66 ms (15/s) - that
        // together with the slow redrawing was the reason for the
        // choppy picture. The camera delivers 30 anyway.
        self.next_image = now_ts + std::time::Duration::from_millis(33);
        match kod.nv12_frame(&picture.nv12) {
            Ok(parts) => {
                for t in parts {
                    self.ton.send_frame(t.data);
                }
                self.image_sent += 1;
            }
            Err(e) => self.camera_notice = format!("Kodierer: {}", e),
        }
        // Own preview. Used to be only every THIRD frame - at 15 frames
        // per second that is 5/s, and the own tile visibly stuttered
        // more than those of the others. Converting to RGBA costs hardly anything at
        // 640x360; now every frame.
        {
            let mut rgba = Vec::new();
            if crate::h264::nv12_to_rgba(
                &picture.nv12,
                picture.width_px,
                picture.height_px,
                picture.width_px as usize,
                picture.height_px,
                &mut rgba,
            ) {
                self.own = Some((picture.width_px, picture.height_px, rgba));
                self.own_state += 1;
            }
        }
        let f = kam.error();
        if !f.is_empty() {
            self.camera_notice = f;
        }
    }

    /// Draw the avatar and send it like a camera image.
    ///
    /// The mouth follows the own microphone level, which the meeting
    /// measures anyway. Blinking comes at irregular intervals, plus a
    /// slight breathing movement - without it a still image looks like an
    /// away sign.
    fn avatar_pumpe(&mut self, now_ts: std::time::Instant) {
        let Some(kod) = self.koder.as_mut() else { return };
        // 20 frames per second are enough: the avatar has little motion, and
        // it should not eat CPU time.
        self.next_image = now_ts + std::time::Duration::from_millis(50);
        self.avatar_beat += 1;
        let t = self.avatar_beat;
        self.avatar_mouth = crate::avatar::mouth_shape(self.level, self.avatar_mouth);
        // A blink lasts three frames, then the next comes in
        // 2 to 6 seconds - regular blinking looks mechanical.
        let blink = if t >= self.avatar_blinks && t < self.avatar_blinks + 3 {
            let i = (t - self.avatar_blinks) as f32;
            1.0 - (i - 1.0).abs()
        } else {
            if t >= self.avatar_blinks + 3 {
                // Derive an irregular interval from the tick
                // itself - no random generator needed for that.
                let streu = (t.wrapping_mul(2654435761) % 80) as u64;
                self.avatar_blinks = t + 40 + streu;
            }
            0.0
        };
        let breathe = (t as f32 * 0.06).sin();
        let (b, h) = (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT);
        crate::avatar::draw(
            &self.avatar,
            b,
            h,
            self.avatar_mouth,
            blink.clamp(0.0, 1.0),
            breathe,
            &mut self.avatar_rgb,
        );
        if !crate::meetcam::rgb_into_nv12(&self.avatar_rgb, b, h, b, h, &mut self.avatar_nv12) {
            return;
        }
        match kod.nv12_frame(&self.avatar_nv12) {
            Ok(parts) => {
                for part in parts {
                    self.ton.send_frame(part.data);
                }
                self.image_sent += 1;
            }
            Err(e) => self.camera_notice = format!("Kodierer: {}", e),
        }
        // Own preview: the avatar is exactly what the others see.
        let mut rgba = Vec::with_capacity((b * h * 4) as usize);
        for p in self.avatar_rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        self.own = Some((b, h, rgba));
        self.own_state += 1;
    }

    /// Avatar instead of camera. Really switches the camera off in the process - otherwise
    /// it would keep running invisibly, and the light next to it too.
    pub fn avatar_toggle(&mut self, an: bool) {
        if an == self.avatar_an {
            return;
        }
        if an {
            if self.camera.is_some() {
                if let Some(k) = self.camera.take() {
                    k.stop_capture();
                }
            }
            if self.koder.is_none() {
                match crate::meetvideo::VideoEncoder::neu(
                    crate::meetvideo::WIDTH,
                    crate::meetvideo::HEIGHT,
                    20,
                    AVATAR_BITRATE,
                ) {
                    Ok(c) => {
                        self.koder = Some(c);
                        self.encoder_bitrate = AVATAR_BITRATE;
                    }
                    Err(e) => {
                        self.camera_notice = format!("Kein Kodierer: {}", e);
                        return;
                    }
                }
            }
            self.avatar_an = true;
            self.camera_on = true;
            self.camera_notice = "Avatar statt Kamera".into();
        } else {
            self.avatar_an = false;
            self.koder = None;
            self.own = None;
            self.own_state += 1;
            self.camera_on = false;
            self.camera_notice = String::new();
        }
        if self.video_reported {
            self.sig.is_muted("video", !self.camera_on);
        }
    }

    /// Aussehen des Avatars aendern (wirkt beim naechsten Bild).
    pub fn avatar_setzen(&mut self, a: crate::avatar::Look) {
        self.avatar = a;
    }

    /// Switch the camera on or off.
    pub fn camera_toggle(&mut self, an: bool) {
        // Camera and avatar are mutually exclusive - there is only one video track.
        if an && self.avatar_an {
            self.avatar_toggle(false);
        }
        if an {
            if self.camera.is_some() {
                return;
            }
            match crate::meetcam::open(
                self.camera_device.clone(),
                crate::meetvideo::WIDTH,
                crate::meetvideo::HEIGHT,
                30,
            ) {
                Ok(k) => {
                    match crate::meetvideo::VideoEncoder::neu(k.width_px, k.height_px, 30, CAMERA_BITRATE) {
                        Ok(c) => {
                            self.camera_notice = format!("Kamera: {}", k.name);
                            self.camera = Some(k);
                            self.koder = Some(c);
                            self.encoder_bitrate = CAMERA_BITRATE;
                            self.camera_on = true;
                        }
                        Err(e) => {
                            self.camera_notice = format!("Kein Kodierer: {}", e);
                            self.camera_on = false;
                        }
                    }
                }
                Err(e) => {
                    self.camera_notice = format!("Keine Kamera: {}", e);
                    self.camera_on = false;
                }
            }
        } else {
            if let Some(k) = self.camera.take() {
                k.stop_capture();
            }
            self.koder = None;
            self.own = None;
            self.own_state += 1;
            self.camera_on = false;
        }
        if self.video_reported {
            self.sig.is_muted("video", !self.camera_on);
        }
    }

    /// Name of the running camera (empty = off).
    pub fn camera_name(&self) -> String {
        self.camera.as_ref().map(|k| k.name.clone()).unwrap_or_default()
    }

    pub fn toggle_muted(&mut self, an: bool) {
        self.is_muted = an;
        self.ton.is_muted(an);
        if let Some(g) = &self.devices {
            g.is_muted.store(an, std::sync::atomic::Ordering::Relaxed);
        }
        self.sig.is_muted("audio", an);
    }

    /// Change the camera in the RUNNING meeting. `device` = None is the
    /// default device. If the camera is running, it is reopened -
    /// otherwise the meeting remembers the choice for the next switch-on.
    pub fn camera_pick(&mut self, device: Option<String>) {
        if self.camera_device == device {
            return;
        }
        self.camera_device = device;
        if self.camera_on {
            // Off and on again: the capture thread is bound to the device,
            // changing while running is not supported.
            self.camera_toggle(false);
            self.camera_toggle(true);
        }
    }

    /// Change microphone and/or speaker in the running meeting.
    ///
    /// WHY both together: capture and playback hang on ONE
    /// building block (shared echo cancellation). Swapping them individually would only work
    /// by throwing away the echo estimate - then it whistles.
    pub fn tone_pick(&mut self, mic_name: Option<String>, speaker: Option<String>) {
        if self.mic_device == mic_name && self.speaker_device == speaker {
            return;
        }
        self.mic_device = mic_name.clone();
        self.speaker_device = speaker.clone();
        // First release the old one (drop stops the threads), then the new one.
        self.devices = None;
        match meetaudio::start_devices(mic_name, speaker) {
            Ok(g) => {
                g.is_muted
                    .store(self.is_muted, std::sync::atomic::Ordering::Relaxed);
                g.lock_on
                    .store(self.lock_on, std::sync::atomic::Ordering::Relaxed);
                g.lock_strength.store(
                    self.lock_strength as u32,
                    std::sync::atomic::Ordering::Relaxed,
                );
                self.notice = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.exit);
                self.protocol.push(self.notice.clone());
                self.devices = Some(g);
            }
            Err(e) => {
                self.notice = format!("Kein Ton-Geraet: {}", e);
                self.protocol.push(self.notice.clone());
            }
        }
    }

    /// Set the noise gate. Takes effect immediately, without rebuilding the audio.
    pub fn noise_gate_set(&mut self, an: bool, strength: u8) {
        self.lock_on = an;
        self.lock_strength = strength.min(100);
        if let Some(g) = &self.devices {
            g.lock_on.store(an, std::sync::atomic::Ordering::Relaxed);
            g.lock_strength.store(
                self.lock_strength as u32,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    /// Is the noise gate currently letting sound through? For display only - without
    /// this feedback the slider would be flying blind.
    pub fn lock_open(&self) -> bool {
        match &self.devices {
            Some(g) => g.lock_open.load(std::sync::atomic::Ordering::Relaxed),
            None => true,
        }
    }

    /// Estimated noise floor (0..1) - shows WHAT the gate
    /// has currently adjusted to.
    pub fn lock_noise_floor(&self) -> f32 {
        match &self.devices {
            Some(g) => {
                g.lock_reason.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10_000.0
            }
            None => 0.0,
        }
    }

    /// Names of the audio devices currently in use (for display).
    pub fn ton_namen(&self) -> (String, String) {
        match &self.devices {
            Some(g) => (g.eingang.clone(), g.exit.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// As VIEWER: say which part of the foreign screen I am
    /// looking at. The sender then cuts exactly that out of its native
    /// capture - same encode size, so same bandwidth,
    /// but real pixels instead of extrapolated ones.
    ///
    /// `sharp = false` means: please keep sending everything (then it stays
    /// plain magnifying).
    pub fn zoom_wish(&mut self, peer: u64, region: crate::meetschirm::Region, sharp: bool) {
        let will = if sharp {
            region
        } else {
            crate::meetschirm::WHOLE
        };
        let now_ts = std::time::Instant::now();
        let neu = match self.last_wish {
            Some((p, alt)) => p != peer || crate::meetschirm::region_worth_switch(alt, will),
            None => true,
        };
        // Repeat every 3 s even without change: the sender forgets
        // wishes it no longer hears from (otherwise it would stay on the
        // section of a long-gone viewer).
        if !neu && now_ts < self.next_wish {
            return;
        }
        self.next_wish = now_ts + std::time::Duration::from_secs(3);
        self.last_wish = Some((peer, will));
        self.sig.region_wish(peer, will.0, will.1, will.2, will.3);
    }

    /// As SENDER: build the section from all wishes and apply it.
    ///
    /// Runs on every pass, but only changes something on a worthwhile
    /// difference - every switch costs a keyframe and is
    /// visible as a small jump.
    fn region_pump(&mut self) {
        if !self.screen_on {
            if self.screen_region != crate::meetschirm::WHOLE {
                self.screen_region = crate::meetschirm::WHOLE;
            }
            return;
        }
        // Wishes we have not heard from for 8 s no
        // longer count - the viewer is gone or zoomed out again.
        let now_ts = std::time::Instant::now();
        self.wish_from
            .retain(|_, (_, wann)| now_ts.duration_since(*wann) < std::time::Duration::from_secs(8));
        let wishes: Vec<crate::meetschirm::Region> =
            self.wish_from.values().map(|(b, _)| *b).collect();
        let neu = crate::meetschirm::region_merge(&wishes);
        if neu == self.screen_region {
            return;
        }
        if self.screen_region != crate::meetschirm::WHOLE
            && neu != crate::meetschirm::WHOLE
            && !crate::meetschirm::region_worth_switch(self.screen_region, neu)
        {
            return;
        }
        self.screen_region = neu;
        if let Some(a) = self.screen.as_ref() {
            a.region_set(neu);
        }
        // The image content jumps - without a fresh keyframe the
        // others would see garbled video for a moment.
        if let Some(k) = self.screen_encoder.as_mut() {
            k.keyframe();
        }
        self.last_keyframe = now_ts;
        // And tell the others WHAT is coming now. Without this message
        // they would zoom once more into the already cropped image.
        self.sig.region(neu.0, neu.1, neu.2, neu.3);
    }

    /// Pointer of whoever is sharing (0..1). None = does not (yet) arrive.
    pub fn pointer_the_divider(&self, peer: u64) -> Option<(f32, f32)> {
        self.pointer_from.get(&peer).copied()
    }

    /// The OWN mouse pointer on the currently shared screen.
    ///
    /// WHY extra: the server sends pointer positions only to the OTHERS -
    /// you never get your own back. Whoever looks at their own share
    /// therefore kept reading "Sharer's mouse (does not arrive)", although the
    /// information was right in front of them.
    pub fn own_pointer(&self) -> Option<(f32, f32)> {
        let a = self.screen.as_ref()?;
        crate::meetschirm::pointer_share_source(a.origin)
    }

    /// Welchen Ausschnitt sendet dieser Participant gerade?
    pub fn region_the_divider(&self, peer: u64) -> Option<crate::meetschirm::Region> {
        self.region_from.get(&peer).copied()
    }

    /// Can this participant deliver sections at all?
    pub fn can_sharp(&self, peer: u64) -> bool {
        self.sharp_can.contains(&peer)
    }

    pub fn hand_heben(&mut self, an: bool) {
        self.hand = an;
        self.sig.hand(an);
    }

    pub fn send_out(&mut self) {
        let t = self.input_text.trim().to_string();
        if t.is_empty() {
            return;
        }
        self.sig.chat(&t);
        let ich = self.sig.state().ich;
        self.chat.push((ich, t));
        self.input_text.clear();
    }

    pub fn admit(&self, peer: u64) {
        self.sig.waiting_room("admit", Some(peer));
    }
    pub fn refuse(&self, peer: u64) {
        self.sig.waiting_room("deny", Some(peer));
    }
    pub fn all_admit(&self) {
        self.sig.waiting_room("admit-all", None);
    }
    pub fn waiting_room(&self, an: bool) {
        self.sig.waiting_room(if an { "on" } else { "off" }, None);
    }

    pub fn leave(&self) {
        if let Some(k) = self.camera.as_ref() {
            k.stop_capture();
        }
        if let Some(a) = self.screen.as_ref() {
            a.stop_capture();
            self.sig.roh(serde_json::json!({"t":"screen","on":false}));
        }
        self.sig.leave();
        self.ton.finish();
    }

    /// Name of a participant (for the chat).
    pub fn name_von(&self, id: u64) -> String {
        let z = self.sig.state();
        if id == z.ich {
            return "Du".into();
        }
        z.people
            .iter()
            .find(|x| x.id == id)
            .map(|x| x.name.clone())
            .unwrap_or_else(|| format!("#{}", id))
    }

    /// Stage 4: release or withdraw the own remote control. The
    /// others thereby get a button that sets up a real FreeViewer
    /// session to us - it is still only admitted here in the
    /// program, so never behind the owner's back.
    pub fn release_control(&mut self, an: bool) {
        // Without own number the release would be worthless - better to report it.
        if an && self.ich_fvid.chars().filter(|c| c.is_ascii_digit()).count() == 0 {
            self.notice = "Keine FreeViewer-Nummer - Steuerung nicht freigebbar".into();
            return;
        }
        self.control_free = an;
        self.sig.remote_control(an, &self.ich_fvid);
        if an {
            self.chat
                .push((0, "Du erlaubst jetzt Fernsteuerung".to_string()));
        }
    }

    /// As viewer: ask the sharer for control.
    ///
    /// We do NOT claim who is sharing - the server decides that from
    /// the tracks. If nobody is sharing, an error comes back
    /// ("kein-teilender"), and it lands in `notice`.
    pub fn request_control(&mut self) {
        self.control_asked = true;
        self.sig.request_control();
        self.chat.push((0, "Du hast um die Steuerung gebeten".into()));
    }

    /// As sharer: accept or reject a request.
    pub fn reply_control(&mut self, peer: u64, ja: bool) {
        self.control_request.retain(|(p, _)| *p != peer);
        // Without own number the other side cannot do anything with the
        // approval - better to say right away what is wrong than to feign
        // a release.
        if ja && self.ich_fvid.chars().filter(|c| c.is_ascii_digit()).count() == 0 {
            self.notice = "Keine FreeViewer-Nummer - Steuerung nicht freigebbar".into();
            self.sig.answer_control(peer, false, "");
            return;
        }
        self.sig.answer_control(peer, ja, &self.ich_fvid);
        if ja {
            if !self.control_allowed.contains(&peer) {
                self.control_allowed.push(peer);
            }
            let wer = self.name_von(peer);
            self.chat.push((
                0,
                format!(
                    "{} darf jetzt steuern - die Sitzung selbst musst du hier im Programm trotzdem zulassen",
                    wer
                ),
            ));
        }
    }

    /// As sharer: withdraw a granted release (None = all).
    pub fn revoke_control(&mut self, peer: Option<u64>) {
        match peer {
            Some(p) => self.control_allowed.retain(|x| *x != p),
            None => self.control_allowed.clear(),
        }
        self.sig.revoke_control(peer);
        self.chat
            .push((0, "Steuerungsfreigabe zurueckgenommen".into()));
    }

    /// Offene Anfragen an mich (Participant, Name).
    pub fn control_request(&self) -> Vec<(u64, String)> {
        self.control_request.clone()
    }

    /// Wem ich es erlaubt habe (Participant, Name).
    pub fn control_allowed(&self) -> Vec<(u64, String)> {
        self.control_allowed
            .iter()
            .map(|p| (*p, self.name_von(*p)))
            .collect()
    }

    /// Meine Zusage: (Name des Teilenden, seine FreeViewer-Nummer).
    pub fn control_grant(&self) -> Option<(String, String)> {
        self.control_grant
            .as_ref()
            .map(|(p, f)| (self.name_von(*p), f.clone()))
    }

    /// Is someone ELSE sharing right now? Only then is the request button worth it - otherwise
    /// it would be a promise the server immediately rejects with
    /// "kein-teilender".
    pub fn foreign_shares(&self) -> bool {
        let ich = self.sig.state().ich;
        self.screens.images.keys().any(|id| *id != ich)
    }

    /// Who in the room can be remote controlled? (participant, name, FreeViewer no.)
    pub fn controllable(&self) -> Vec<(u64, String, String)> {
        let z = self.sig.state();
        z.people
            .iter()
            .filter(|t| t.id != z.ich && !t.fvid.is_empty())
            .map(|t| (t.id, t.name.clone(), t.fvid.clone()))
            .collect()
    }

    /// Own FreeViewer number (for display).
    pub fn my_fvid(&self) -> &str {
        &self.ich_fvid
    }

    pub fn am_host(&self) -> bool {
        let z = self.sig.state();
        z.is_host != 0 && z.is_host == z.ich
    }
}

/// The preview BEFORE joining: own camera image and microphone level,
/// without a meeting running yet.
///
/// WHY a separate building block: the browser shows a live self-view on its join screen
/// - without it the page feels like a form,
/// and that is exactly what Justin complained about. NativMeet cannot provide this because it
/// only exists after joining. On joining the devices are released
/// here again, otherwise the preview holds on to the camera and the
/// meeting would not get it.
pub struct Preflight {
    camera: Option<crate::meetcam::WebCam>,
    ton: Option<meetaudio::Devices>,
    /// Own image for display: (width, height, RGBA).
    pub own: Option<(u32, u32, Vec<u8>)>,
    /// Counts up as soon as a new image is there (reload texture).
    pub stand: u64,
    pub level: f32,
    pub notice: String,
    next: std::time::Instant,
    /// What the user WANTS - separate from the actual state. Without this separation
    /// a device error would trigger a new attempt every frame.
    pub wish_camera: bool,
    pub wish_mic: bool,
    pub mic_device: Option<String>,
}

impl Default for Preflight {
    fn default() -> Self {
        Preflight {
            camera: None,
            ton: None,
            own: None,
            stand: 0,
            level: 0.0,
            notice: String::new(),
            next: std::time::Instant::now(),
            wish_camera: false,
            wish_mic: false,
            mic_device: None,
        }
    }
}

impl Preflight {
    pub fn camera_on(&self) -> bool {
        self.camera.is_some()
    }

    /// Switch the camera on or off for the preview.
    pub fn camera(&mut self, an: bool) {
        self.wish_camera = an;
        if an == self.camera.is_some() {
            return;
        }
        if !an {
            if let Some(k) = self.camera.take() {
                k.stop_capture();
            }
            self.own = None;
            self.stand += 1;
            return;
        }
        match crate::meetcam::open(None, crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 30) {
            Ok(k) => {
                self.notice = format!("Kamera: {}", k.name);
                self.camera = Some(k);
            }
            Err(e) => {
                self.notice = format!("Keine Kamera: {}", e);
            }
        }
    }

    /// Switch the microphone on or off for the level bar.
    pub fn mic_name(&mut self, an: bool, device: Option<String>) {
        let switch = device != self.mic_device;
        self.wish_mic = an;
        self.mic_device = device.clone();
        if !an {
            self.ton = None;
            self.level = 0.0;
            return;
        }
        if self.ton.is_some() && !switch {
            return;
        }
        self.ton = None;
        match meetaudio::start_devices(device, None) {
            Ok(g) => self.ton = Some(g),
            Err(e) => self.notice = format!("Kein Mikrofon: {}", e),
        }
    }

    /// In jedem Bild aufrufen: Bild abholen, Pegel messen.
    pub fn pumpe(&mut self) {
        if let Some(g) = &self.ton {
            let mut spitze = 0.0f32;
            while let Ok(frame_box) = g.mic_name.try_recv() {
                spitze = spitze.max(
                    frame_box
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max),
                );
            }
            self.level = self.level * 0.7 + spitze * 0.3;
        }
        let now_ts = std::time::Instant::now();
        if now_ts < self.next {
            return;
        }
        let kam = match self.camera.as_ref() {
            Some(k) => k,
            None => return,
        };
        let picture = match kam.newest() {
            Some(b) => b,
            None => return,
        };
        // 12 frames per second are entirely enough for a preview.
        self.next = now_ts + std::time::Duration::from_millis(80);
        let mut rgba = Vec::new();
        if crate::h264::nv12_to_rgba(
            &picture.nv12,
            picture.width_px,
            picture.height_px,
            picture.width_px as usize,
            picture.height_px,
            &mut rgba,
        ) {
            self.own = Some((picture.width_px, picture.height_px, rgba));
            self.stand += 1;
        }
        let f = kam.error();
        if !f.is_empty() {
            self.notice = f;
        }
    }

    /// Release everything again (before joining and on closing).
    pub fn aus(&mut self) {
        if let Some(k) = self.camera.take() {
            k.stop_capture();
        }
        self.wish_camera = false;
        self.wish_mic = false;
        self.mic_device = None;
        self.ton = None;
        self.own = None;
        self.level = 0.0;
        self.stand += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_without_camera_and_mic_reports_cleanly() {
        // On a server without camera/sound card the preview must not
        // blow up - it only has to report honestly that nothing is there.
        let mut v = Preflight::default();
        v.camera(true);
        v.mic_name(true, None);
        v.pumpe();
        #[cfg(not(windows))]
        assert!(!v.camera_on(), "none may be running without a camera");
        assert!(v.own.is_none() || v.camera_on());
        v.aus();
        assert!(!v.camera_on());
        assert_eq!(v.level, 0.0);
    }

    #[test]
    fn device_list_does_not_crash() {
        // On a server without sound card this must give an empty list
        // and not blow up.
        let (ein, aus) = meetaudio::devices_list();
        println!("Mikrofone {:?} Lautsprecher {:?}", ein.len(), aus.len());
    }

    #[test]
    fn toggle_camera_without_camera_reports_cleanly() {
        // Server without camera: the switch must set a message and
        // stay off - no crash, no hanging thread.
        let r = NativeMeet::join_meeting("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None, None,);
        if let Ok(mut m) = r {
            m.camera_toggle(true);
            #[cfg(not(windows))]
            {
                assert!(!m.camera_on, "it must not be on without a camera");
                assert!(!m.camera_notice.is_empty(), "keine Meldung");
            }
            m.camera_toggle(false);
            assert!(!m.camera_on);
            m.leave();
        }
    }

    #[test]
    fn toggle_screen_without_display_reports_cleanly() {
        // Server without screen: report cleanly, do not crash.
        let r = NativeMeet::join_meeting("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None, None,);
        if let Ok(mut m) = r {
            m.toggle_screen(true, 0);
            if !m.screen_on {
                assert!(!m.screen_notice.is_empty(), "keine Meldung");
            }
            m.toggle_screen(false, 0);
            assert!(!m.screen_on);
            m.leave();
        }
    }

    #[test]
    fn control_without_number_is_not_released() {
        // Without own FreeViewer number the release would be a lie:
        // the others would get a button that leads nowhere.
        let r = NativeMeet::join_meeting(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "",
            true,
            None,
            None, None,);
        if let Ok(mut m) = r {
            assert!(!m.control_free, "ohne Nummer darf nichts freigegeben sein");
            m.release_control(true);
            assert!(!m.control_free, "without a number the grant must not take effect");
            assert!(!m.notice.is_empty(), "keine Meldung");
            m.leave();
        }
    }

    #[test]
    fn control_with_number_switches() {
        let r = NativeMeet::join_meeting(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "497628420",
            false,
            None,
            None, None,);
        if let Ok(mut m) = r {
            assert!(!m.control_free, "the grant was off when joining");
            m.release_control(true);
            assert!(m.control_free, "Freigabe hat nicht gegriffen");
            m.release_control(false);
            assert!(!m.control_free, "Zuruecknehmen hat nicht gegriffen");
            assert_eq!(m.my_fvid(), "497628420");
            // Without a connection the room knows nobody - the list
            // must still only run, not blow up.
            assert!(m.controllable().is_empty());
            m.leave();
        }
    }

    #[test]
    fn join_without_server_reports_cleanly() {
        // Wrong port -> the signalling must not panic,
        // but must return an error or report a silent disconnect.
        let r = NativeMeet::join_meeting(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "Test",
            "",
            false,
            None,
            None, None,);
        match r {
            Ok(m) => {
                // Connection runs in the background - after a short time
                // a message must be there, no crash.
                std::thread::sleep(std::time::Duration::from_millis(300));
                let mut m = m;
                m.pumpe();
                assert!(!m.state().connected || m.state().last_error.is_empty());
                m.leave();
            }
            Err(e) => {
                assert!(!e.to_string().is_empty());
            }
        }
    }
}


#[cfg(test)]
mod bitrate_tests {
    use super::*;

    #[test]
    fn without_measurement_presets_remain() {
        assert_eq!(
            bitrates_distribute(0, true, true),
            (CAMERA_BITRATE, SCREEN_BITRATE)
        );
    }

    #[test]
    fn measured_line_is_not_exceeded() {
        // Exactly the value the real server reported on 11.09.2026.
        let (k, s) = bitrates_distribute(1_021_098, true, true);
        assert!(
            (k + s) as u64 <= 1_021_098,
            "together {} bit/s over a line of 1021098",
            k + s
        );
        // And the old state would have sent 6 Mbit/s here:
        assert!(
            (k + s) < 2_000_000 + 4_000_000,
            "not better than before"
        );
    }

    #[test]
    fn bitrate_stays_within_bounds() {
        let (k, s) = bitrates_distribute(50_000, true, true);
        assert_eq!(k, CAMERA_MIN);
        assert_eq!(s, SCREEN_MIN);
        let (k, s) = bitrates_distribute(500_000_000, true, true);
        assert_eq!(k, CAMERA_MAX);
        assert_eq!(s, SCREEN_MAX);
    }

    #[test]
    fn display_gets_more_than_camera() {
        let (k, s) = bitrates_distribute(4_000_000, true, true);
        assert!(s > k, "Bildschirm {} <= Kamera {}", s, k);
    }

    #[test]
    fn small_fluctuations_do_not_reset_the_encoder() {
        assert!(!bitrate_worth_switch(700_000, 750_000));
        assert!(!bitrate_worth_switch(700_000, 650_000));
        assert!(bitrate_worth_switch(700_000, 300_000));
        assert!(bitrate_worth_switch(300_000, 900_000));
    }
}
