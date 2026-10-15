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
pub const KAMERA_BITRATE: u32 = 700_000;
/// Avatar - drawn flat areas, they cost almost nothing.
pub const AVATAR_BITRATE: u32 = 400_000;
/// Screen up to 1920x1080 at 20 frames/s. Screen content is
/// largely uniform and compresses well.
pub const SCREEN_BITRATE: u32 = 1_800_000;

/// Lower and upper bounds - below the lower it looks like mush,
/// above the upper more brings nothing.
pub const KAMERA_MIN: u32 = 150_000;
pub const KAMERA_MAX: u32 = 1_200_000;
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
/// `schaetzung` = 0 means "no measurement" - then the
/// defaults stay in place.
pub fn bitraten_verteilen(schaetzung: u64, kamera: bool, screen: bool) -> (u32, u32) {
    if schaetzung == 0 {
        return (KAMERA_BITRATE, SCREEN_BITRATE);
    }
    let nutzbar = (schaetzung as f64 * 0.85) as u32;
    let (k, s) = match (kamera, screen) {
        (true, true) => ((nutzbar as f64 * 0.30) as u32, (nutzbar as f64 * 0.70) as u32),
        (true, false) => (nutzbar, SCREEN_BITRATE),
        (false, true) => (KAMERA_BITRATE, nutzbar),
        (false, false) => (KAMERA_BITRATE, SCREEN_BITRATE),
    };
    (
        k.clamp(KAMERA_MIN, KAMERA_MAX),
        s.clamp(SCREEN_MIN, SCREEN_MAX),
    )
}

/// Is it worth rebuilding the encoder? That costs a
/// keyframe, so not for every small change - only from a
/// third difference upwards.
pub fn bitrate_lohnt_wechsel(jetzt: u32, ziel: u32) -> bool {
    let (a, b) = (jetzt.max(1) as f64, ziel.max(1) as f64);
    (a - b).abs() / a >= 0.34
}

pub struct NativMeet {
    sig: meetsig::Session,
    ton: meetrtc::Ton,
    geraete: Option<meetaudio::Geraete>,
    /// Chatverlauf: (Absender, Text) - Absender 0 = System.
    pub chat: Vec<(u64, String)>,
    pub input_text: String,
    angeboten: bool,
    pub hand: bool,
    pub is_muted: bool,
    /// Level of the own microphone (0..1), for display.
    pub pegel: f32,
    /// Letzte Notice (FriendError, Hinweise).
    pub notice: String,
    pub raum: String,
    /// How many video frames have arrived so far (stage 2 shows them later).
    pub image_counter: u64,
    /// Format of the incoming frames ("H264"/"Vp8") - for display.
    pub bild_codec: String,
    /// Decoded CAMERA frames of the others (the latest per participant).
    /// What the camera is currently encoding at (bit/s).
    pub koder_bitrate: u32,
    /// What the screen is currently encoding at (bit/s).
    pub screen_bitrate: u32,
    /// Wann zuletzt nachgerechnet wurde.
    bitrate_geprueft: std::time::Instant,
    /// How often the bitrate has been adjusted - for measuring only.
    pub bitrate_wechsel: u32,
    pub bilder: crate::meetvideo::Dekodierer,
    /// Decoded SCREEN frames of the others. Separate bucket, otherwise
    /// the share would overwrite the same participant's face.
    pub schirme: crate::meetvideo::Dekodierer,
    /// Own camera (stage 2d) - only runs when it is switched on.
    kamera: Option<crate::meetcam::Kamera>,
    koder: Option<crate::meetvideo::Kodierer>,
    pub kamera_an: bool,
    /// Send a drawn avatar instead of the camera.
    pub avatar_an: bool,
    pub avatar: crate::avatar::Aussehen,
    /// Smoothed mouth position (0..1) - the raw level jitters too much.
    avatar_mund: f32,
    /// Counts frames; blinking and breathing are derived from it.
    avatar_takt: u64,
    /// At which tick the next blink is due.
    avatar_blinzelt: u64,
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
    pub grund_steht: bool,
    pub kamera_geraet: Option<String>,
    pub mikro_geraet: Option<String>,
    pub lautsprecher_geraet: Option<String>,
    pub camera_notice: String,
    /// Own image for the preview: (width, height, RGBA).
    pub eigen: Option<(u32, u32, Vec<u8>)>,
    /// Counts up when the preview has a new image (texture reload).
    pub eigen_stand: u64,
    /// How many own frames have gone out so far.
    pub bild_gesendet: u64,
    /// When the next frame may go out (15 frames per second).
    naechstes_bild: std::time::Instant,
    /// Video track is registered with the server.
    video_gemeldet: bool,
    /// Screen share (stage 3) - own track next to the camera.
    screen: Option<crate::meetschirm::Aufnahme>,
    screen_encoder: Option<crate::meetvideo::Kodierer>,
    pub screen_on: bool,
    /// The red frame around whatever is currently going out. Lives exactly as long
    /// as the share; it disappears by itself when dropped.
    rahmen: Option<crate::fenster::Rahmen>,
    /// Noise gate: on? And how strict (0..100)? Kept here
    /// so that a device change does not forget the setting - the
    /// audio devices are rebuilt completely in the process.
    pub sperre_an: bool,
    pub sperre_staerke: u8,
    /// Should a frame be shown at all? Some people do not want it.
    pub rahmen_an: bool,
    /// Where the frame was last placed - needless moving
    /// would be flicker.
    rahmen_lage: Option<(i32, i32, i32, i32)>,
    pub screen_notice: String,
    pub screen_sent: u64,
    /// The own shared image as RGBA - so you can see for YOURSELF what you
    /// are sharing. The browser client does exactly that ("otherwise you do not
    /// see WHAT you are sharing"); native lacked it, and a click on
    /// "Screen" therefore looked as if nothing happened at all.
    pub own_screen: Option<(u32, u32, Vec<u8>)>,
    pub own_screen_state: u64,
    naechstes_schirmbild: std::time::Instant,
    /// Wishes of the viewers: who wants which section of my
    /// screen? (participant -> area, plus when last heard from)
    wunsch_von: std::collections::HashMap<u64, (crate::meetschirm::Bereich, std::time::Instant)>,
    /// Welchen Ausschnitt SENDE ich gerade?
    pub screen_region: crate::meetschirm::Bereich,
    /// Which section are the OTHERS currently sending (for drawing)?
    pub bereich_von: std::collections::HashMap<u64, crate::meetschirm::Bereich>,
    /// Who can deliver sections at all (new client)? Whoever has
    /// reported once can - before that, "armed" would be a lie.
    pub scharf_kann: std::collections::HashSet<u64>,
    /// Last sent wish and when - repeats are useless.
    letzter_wunsch: Option<(u64, crate::meetschirm::Bereich)>,
    naechster_wunsch: std::time::Instant,
    /// Pointer positions of the others (0..1 on THEIR shared screen).
    /// This lets a viewer zoom to where the other one is pointing.
    pub zeiger_von: std::collections::HashMap<u64, (f32, f32)>,
    /// When the next own pointer state may go out (10 times per second).
    naechster_zeiger: std::time::Instant,
    /// Last sent state - unchanged values are not sent.
    letzter_zeiger: Option<(f32, f32)>,
    /// When a keyframe was last requested.
    letztes_schluesselbild: std::time::Instant,
    /// Own FreeViewer number - we announce it when sharing.
    ich_fvid: String,
    /// Stage 4: remote control released for the others?
    pub steuer_frei: bool,
    /// Open control requests TO ME: (participant, name). They only
    /// arrive while I am really sharing - the server checks that.
    steuer_anfragen: Vec<(u64, String)>,
    /// Whom I have allowed to control my shared screen.
    /// EXACTLY these people, not the whole room.
    steuer_erlaubt: Vec<u64>,
    /// I asked myself and am waiting for the decision.
    pub steuer_gefragt: bool,
    /// Approval received: (participant, FreeViewer number of the sharer).
    /// Only with this can a real FreeViewer session be set up.
    steuer_zusage: Option<(u64, String)>,
    /// Whom the server currently reports as speaker.
    pub sprecher: Option<u64>,
    /// How loud each other participant currently is (0..1). The green
    /// outline of the tile comes from this - the server does not always report the speaker.
    pub pegel_von: std::collections::HashMap<u64, f32>,
    /// Who is currently typing (for "is typing ..." in the chat).
    pub tippen: std::collections::HashSet<u64>,
    /// Log for the "Info" tab - what happened in the room.
    pub protokoll: Vec<String>,
    /// Gemessener Durchsatz in kbit/s (aus den Byte-Zaehlern, je Sekunde).
    pub kbit_raus: u32,
    pub kbit_rein: u32,
    /// Last measurement for the bandwidth.
    letzte_messung: std::time::Instant,
    letzte_bytes: (u64, u64),
}

impl NativMeet {
    /// Beitreten. `mikro`/`lautsprecher` leer = Standardgeraet.
    pub fn beitreten(
        basis: &str,
        raum: &str,
        pass: &str,
        name: &str,
        fvid: &str,
        control: bool,
        mikro: Option<String>,
        lautsprecher: Option<String>,
        e2e: Option<String>,
    ) -> Result<NativMeet> {
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
        let sig = meetsig::beitreten(
            basis,
            raum,
            pass,
            name,
            if control { fvid } else { "" },
            e2e_an,
        )?;
        let ton = meetrtc::start_with(e2e_key)?;
        // Without a sound card (server, test machine) the meeting still runs -
        // you just hear nothing. Report honestly instead of aborting.
        let (mikro_merk, lautsprecher_merk) = (mikro.clone(), lautsprecher.clone());
        let (geraete, notice) = match meetaudio::start_devices(mikro, lautsprecher) {
            Ok(g) => {
                let m = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.ausgang);
                (Some(g), m)
            }
            Err(e) => (None, format!("Kein Ton-Geraet: {}", e)),
        };
        Ok(NativMeet {
            sig,
            ton,
            geraete,
            chat: Vec::new(),
            input_text: String::new(),
            angeboten: false,
            hand: false,
            is_muted: false,
            pegel: 0.0,
            notice,
            raum: raum.to_string(),
            image_counter: 0,
            bild_codec: String::new(),
            koder_bitrate: KAMERA_BITRATE,
            screen_bitrate: SCREEN_BITRATE,
            bitrate_geprueft: std::time::Instant::now(),
            bitrate_wechsel: 0,
            bilder: crate::meetvideo::Dekodierer::neu(),
            schirme: crate::meetvideo::Dekodierer::neu(),
            kamera: None,
            koder: None,
            kamera_an: false,
            avatar_an: false,
            avatar: crate::avatar::Aussehen::default(),
            avatar_mund: 0.0,
            avatar_takt: 0,
            avatar_blinzelt: 90,
            avatar_rgb: Vec::new(),
            avatar_nv12: Vec::new(),
            e2e_an,
            grund_steht: false,
            kamera_geraet: None,
            mikro_geraet: mikro_merk,
            lautsprecher_geraet: lautsprecher_merk,
            camera_notice: String::new(),
            eigen: None,
            eigen_stand: 0,
            bild_gesendet: 0,
            naechstes_bild: std::time::Instant::now(),
            video_gemeldet: false,
            screen: None,
            screen_encoder: None,
            screen_on: false,
            rahmen: None,
            sperre_an: true,
            sperre_staerke: 45,
            rahmen_an: true,
            rahmen_lage: None,
            screen_notice: String::new(),
            screen_sent: 0,
            own_screen: None,
            own_screen_state: 0,
            naechstes_schirmbild: std::time::Instant::now(),
            wunsch_von: std::collections::HashMap::new(),
            screen_region: crate::meetschirm::GANZ,
            bereich_von: std::collections::HashMap::new(),
            scharf_kann: std::collections::HashSet::new(),
            letzter_wunsch: None,
            naechster_wunsch: std::time::Instant::now(),
            zeiger_von: std::collections::HashMap::new(),
            naechster_zeiger: std::time::Instant::now(),
            letzter_zeiger: None,
            letztes_schluesselbild: std::time::Instant::now(),
            ich_fvid: fvid.to_string(),
            steuer_frei: control && fvid.chars().any(|c| c.is_ascii_digit()),
            steuer_anfragen: Vec::new(),
            steuer_erlaubt: Vec::new(),
            steuer_gefragt: false,
            steuer_zusage: None,
            sprecher: None,
            pegel_von: std::collections::HashMap::new(),
            tippen: std::collections::HashSet::new(),
            protokoll: vec![format!("Raum {} - Beitritt laeuft", raum)],
            kbit_raus: 0,
            kbit_rein: 0,
            letzte_messung: std::time::Instant::now(),
            letzte_bytes: (0, 0),
        })
    }

    pub fn state(&self) -> meetsig::State {
        self.sig.state()
    }

    pub fn zahlen(&self) -> meetrtc::Zahlen {
        self.ton.zahlen()
    }

    /// Must be called every frame: dispatches messages and audio.
    pub fn pumpe(&mut self) {
        for e in self.sig.abholen() {
            match e {
                meetsig::Ereignis::Willkommen { .. } => {
                    if !self.angeboten {
                        self.angeboten = true;
                        self.sig
                            .roh(serde_json::json!({"t":"offer","sdp":self.ton.angebot}));
                    }
                }
                meetsig::Ereignis::Sdp { art, sdp } => {
                    if art == "answer" {
                        self.ton.antwort(&sdp);
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
                        self.video_gemeldet = true;
                        self.sig.is_muted("video", !self.kamera_an);
                    } else {
                        self.ton.server_angebot(&sdp);
                    }
                }
                meetsig::Ereignis::Spur {
                    mid,
                    peer,
                    display,
                    ..
                } => self.ton.spur_art(&mid, peer, display),
                meetsig::Ereignis::Chat { von, text, .. } => self.chat.push((von, text)),
                meetsig::Ereignis::Dazu(t) => {
                    self.chat.push((0, format!("{} ist dazugekommen", t.name)))
                }
                meetsig::Ereignis::Weg(id) => {
                    self.bilder.vergessen(id);
                    self.schirme.vergessen(id);
                    // Whoever is gone asks nothing anymore and may do nothing anymore.
                    self.steuer_anfragen.retain(|(p, _)| *p != id);
                    self.steuer_erlaubt.retain(|p| *p != id);
                    if self.steuer_zusage.as_ref().map(|(p, _)| *p) == Some(id) {
                        self.steuer_zusage = None;
                    }
                    let name = self
                        .sig
                        .state()
                        .leute
                        .iter()
                        .find(|x| x.id == id)
                        .map(|x| x.name.clone())
                        .unwrap_or_else(|| format!("#{}", id));
                    self.chat.push((0, format!("{} ist gegangen", name)));
                }
                meetsig::Ereignis::ForceMuted { art } => {
                    if art == "audio" {
                        self.is_muted = true;
                        if let Some(g) = &self.geraete {
                            g.is_muted.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.ton.is_muted(true);
                        self.chat
                            .push((0, "Der Gastgeber hat dich stummgeschaltet".into()));
                    }
                }
                meetsig::Ereignis::Rausgeworfen(m)
                | meetsig::Ereignis::Beendet(m)
                | meetsig::Ereignis::Abgewiesen(m) => {
                    // The REASON is the valuable part ("Meeting is end-to-end
                    // encrypted - please update"). It must stay
                    // put; the line is cut right after.
                    self.notice = m;
                    self.grund_steht = true;
                }
                meetsig::Ereignis::Fernsteuerung { peer, fvid } => {
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
                meetsig::Ereignis::ControlRequested { peer, name } => {
                    // Asked twice is not asked twice.
                    if !self.steuer_anfragen.iter().any(|(id, _)| *id == peer) {
                        self.steuer_anfragen.push((peer, name.clone()));
                    }
                    self.chat
                        .push((0, format!("{} bittet um die Steuerung", name)));
                }
                meetsig::Ereignis::ControlAnswered {
                    von,
                    gewaehrt,
                    fvid,
                } => {
                    self.steuer_gefragt = false;
                    let wer = self.name_von(von);
                    if gewaehrt {
                        // Without a number the approval is worthless - better to
                        // report honestly than to offer a button that
                        // leads nowhere.
                        if fvid.chars().any(|c| c.is_ascii_digit()) {
                            self.steuer_zusage = Some((von, fvid));
                            self.chat
                                .push((0, format!("{} erlaubt dir die Steuerung", wer)));
                        } else {
                            self.notice =
                                format!("{} hat zugesagt, hat aber keine FreeViewer-Nummer", wer);
                        }
                    } else {
                        self.steuer_zusage = None;
                        self.chat
                            .push((0, format!("{} hat die Steuerung abgelehnt", wer)));
                    }
                }
                meetsig::Ereignis::ControlGiveBack { .. } => {
                    // The server sends this only to the person affected - so it is
                    // always about MY share.
                    if self.steuer_zusage.take().is_some() {
                        self.chat
                            .push((0, "Die Steuerungsfreigabe wurde zurueckgenommen".into()));
                    }
                }
                meetsig::Ereignis::Zeiger { peer, x, y } => {
                    self.zeiger_von.insert(peer, (x, y));
                }
                meetsig::Ereignis::Bereichswunsch { peer, x, y, w, h } => {
                    // A viewer says which part of MY screen they are
                    // currently looking at. Remember it - it is applied in bulk
                    // so that several viewers do not chase each other.
                    self.wunsch_von
                        .insert(peer, ((x, y, w, h), std::time::Instant::now()));
                }
                meetsig::Ereignis::Bereich { peer, x, y, w, h } => {
                    self.scharf_kann.insert(peer);
                    self.bereich_von.insert(peer, (x, y, w, h));
                }
                meetsig::Ereignis::FriendError { code, text } => {
                    self.notice = format!("{}: {}", code, text);
                }
                meetsig::Ereignis::Sprecher(p) => self.sprecher = p,
                meetsig::Ereignis::Tippt { peer, an } => {
                    if an {
                        self.tippen.insert(peer);
                    } else {
                        self.tippen.remove(&peer);
                    }
                }
                meetsig::Ereignis::Willkommen { raum, server, .. } => {
                    self.protokoll
                        .push(format!("Im Raum {} - Server {}", raum, server));
                }
                meetsig::Ereignis::Getrennt(m) => {
                    // If a real reason is already there, it stays -
                    // "Connection ended" would be the worse answer.
                    if !self.grund_steht {
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
        if let Some(a) = self.ton.offene_antwort() {
            self.sig.roh(serde_json::json!({"t":"answer","sdp":a}));
        }

        // Mikrofon -> Netz
        if let Some(g) = &self.geraete {
            while let Ok(rahmen) = g.mikro.try_recv() {
                let spitze = rahmen
                    .iter()
                    .map(|v| (*v as f32 / 32768.0).abs())
                    .fold(0.0f32, f32::max);
                self.pegel = self.pegel * 0.7 + spitze * 0.3;
                self.ton.send_out(rahmen);
            }
        }
        // Netz -> Lautsprecher
        for te in self.ton.abholen() {
            match te {
                meetrtc::TonEreignis::Rahmen { quelle, pcm } => {
                    // Remember the volume per participant: it becomes the
                    // green outline "is speaking". The server reports
                    // the speaker only on changes; this is available immediately.
                    let spitze = pcm
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max);
                    let e = self.pegel_von.entry(quelle).or_insert(0.0);
                    *e = e.max(spitze);
                    if let Some(g) = &self.geraete {
                        if let Ok(mut m) = g.lautsprecher.lock() {
                            m.dazu(quelle, &pcm);
                        }
                    }
                }
                meetrtc::TonEreignis::Bild {
                    quelle,
                    display,
                    daten,
                    schluesselbild,
                    codec,
                } => {
                    self.bild_codec = codec;
                    // Stage 2: decoding is done by the platform later.
                    // For now we only count, so you can see that
                    // video really arrives.
                    self.image_counter += 1;
                    let _ = schluesselbild;
                    if display {
                        self.schirme.rahmen(quelle, &daten);
                    } else {
                        self.bilder.rahmen(quelle, &daten);
                    }
                }
                meetrtc::TonEreignis::FriendError(f) => self.notice = f,
                meetrtc::TonEreignis::Ende(f) => {
                    if !f.is_empty() {
                        self.notice = f;
                    }
                }
                meetrtc::TonEreignis::Verbunden => {
                    self.notice = "Ton verbunden".into();
                }
            }
        }
        self.bitrate_pumpe();
        self.kamera_pumpe();
        self.screen_pump();
        self.zeiger_pumpe();
        self.bereich_pumpe();
        self.messen();
    }

    /// Once per second: compute throughput, let the level decay.
    /// WHY measured and not estimated: the badge above should show a
    /// number that is really correct - guessed it would be worthless.
    fn messen(&mut self) {
        for v in self.pegel_von.values_mut() {
            *v *= 0.90;
        }
        let jetzt = std::time::Instant::now();
        let dt = jetzt.duration_since(self.letzte_messung).as_secs_f32();
        if dt < 1.0 {
            return;
        }
        let z = self.ton.zahlen();
        let (alt_raus, alt_rein) = self.letzte_bytes;
        self.kbit_raus = ((z.bytes_raus.saturating_sub(alt_raus) as f32 * 8.0) / dt / 1000.0) as u32;
        self.kbit_rein = ((z.bytes_rein.saturating_sub(alt_rein) as f32 * 8.0) / dt / 1000.0) as u32;
        self.letzte_bytes = (z.bytes_raus, z.bytes_rein);
        self.letzte_messung = jetzt;
    }

    /// Spricht dieser Participant gerade?
    pub fn spricht(&self, id: u64) -> bool {
        self.sprecher == Some(id) || self.pegel_von.get(&id).copied().unwrap_or(0.0) > 0.06
    }

    /// Gastgeber-Befehle: "mute-all" | "mute" | "kick" | "end".
    pub fn gastgeber_aktion(&self, aktion: &str, peer: Option<u64>) {
        self.sig.gastgeber_aktion(aktion, peer);
    }

    /// Report "is typing" to the others.
    pub fn tippt(&self, an: bool) {
        self.sig.tippt(an);
    }

    /// Screen -> H.264 -> meeting (own track). 15 frames per second,
    /// always the newest frame; a keyframe every 4 seconds so that
    /// late joiners do not face an empty area.
    /// Report the own pointer position while the screen is shared.
    ///
    /// 10 times per second and only on a REAL change: a few
    /// dozen bytes, but they let the viewer zoom exactly to
    /// where it is pointed.
    fn zeiger_pumpe(&mut self) {
        if !self.screen_on {
            return;
        }
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechster_zeiger {
            return;
        }
        self.naechster_zeiger = jetzt + std::time::Duration::from_millis(100);
        let quelle = match self.screen.as_ref() {
            Some(a) => a.quelle,
            None => return,
        };
        if let Some((x, y)) = crate::meetschirm::zeiger_anteil_quelle(quelle) {
            let neu = (x, y);
            let anders = match self.letzter_zeiger {
                Some((ax, ay)) => (ax - x).abs() > 0.002 || (ay - y).abs() > 0.002,
                None => true,
            };
            if anders {
                self.letzter_zeiger = Some(neu);
                self.sig.zeiger(x, y);
            }
        }
    }

    fn screen_pump(&mut self) {
        let jetzt = std::time::Instant::now();
        // Move the frame along BEFORE we abort due to the frame rate -
        // a window can be moved faster than a frame goes out
        // 20 times per second, and a lagging frame
        // looks broken.
        self.rahmen_pumpe();
        if jetzt < self.naechstes_schirmbild {
            return;
        }
        let (auf, kod) = match (self.screen.as_ref(), self.screen_encoder.as_mut()) {
            (Some(a), Some(k)) => (a, k),
            _ => return,
        };
        let bild = match auf.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 50 ms = 20 frames per second. More than text and windows need,
        // but clearly smoother than the earlier 15 - and a shared
        // screen costs less per frame than a camera (much stays the same).
        self.naechstes_schirmbild = jetzt + std::time::Duration::from_millis(50);
        if jetzt.duration_since(self.letztes_schluesselbild) >= std::time::Duration::from_secs(4) {
            self.letztes_schluesselbild = jetzt;
            kod.schluesselbild();
        }
        match kod.nv12_rahmen(&bild.nv12) {
            Ok(teile) => {
                for t in teile {
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
                &bild.nv12,
                bild.width_px,
                bild.height_px,
                bild.width_px as usize,
                bild.height_px,
                &mut rgba,
            ) {
                self.own_screen = Some((bild.width_px, bild.height_px, rgba));
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
    fn rahmen_pumpe(&mut self) {
        if !self.screen_on {
            return;
        }
        let Some(a) = self.screen.as_ref() else { return };
        let quelle = a.quelle;
        match (self.rahmen_an, quelle.lage()) {
            (true, Some(lage)) => {
                if self.rahmen.is_none() {
                    self.rahmen = Some(crate::fenster::Rahmen::zeigen(
                        lage.0, lage.1, lage.2, lage.3,
                    ));
                    self.rahmen_lage = Some(lage);
                } else if self.rahmen_lage != Some(lage) {
                    if let Some(r) = self.rahmen.as_ref() {
                        r.setzen(lage.0, lage.1, lage.2, lage.3);
                    }
                    self.rahmen_lage = Some(lage);
                }
            }
            _ => {
                self.rahmen = None;
                self.rahmen_lage = None;
            }
        }
    }

    /// Switch the red frame on or off. Takes effect immediately.
    pub fn rahmen_schalten(&mut self, an: bool) {
        self.rahmen_an = an;
        if !an {
            self.rahmen = None;
            self.rahmen_lage = None;
        }
    }

    /// Welche Programmfenster lassen sich teilen?
    pub fn fenster() -> Vec<crate::fenster::Fenster> {
        crate::fenster::liste()
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
        self.quelle_schalten_melden(an, crate::meetschirm::Quelle::Display(index))
    }

    /// Like `report_toggle_screen`, but the source may also be a single
    /// program window.
    pub fn quelle_schalten_melden(
        &mut self,
        an: bool,
        quelle: crate::meetschirm::Quelle,
    ) -> Option<String> {
        self.quelle_schalten(an, quelle);
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
        self.quelle_schalten(an, crate::meetschirm::Quelle::Display(index))
    }

    /// Freigabe einer beliebigen Quelle schalten.
    pub fn quelle_schalten(&mut self, an: bool, quelle: crate::meetschirm::Quelle) {
        if an {
            if self.screen.is_some() {
                return;
            }
            match crate::meetschirm::open_source(quelle, 1920, 1080, 15) {
                Ok(a) => match crate::meetvideo::Kodierer::neu(a.width_px, a.height_px, 20, SCREEN_BITRATE) {
                    Ok(k) => {
                        self.screen_notice = format!("Teile {}", a.name);
                        self.screen = Some(a);
                        self.screen_encoder = Some(k);
                        self.screen_bitrate = SCREEN_BITRATE;
                        self.screen_on = true;
                        self.screen_sent = 0;
                        self.letztes_schluesselbild =
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
                        self.rahmen = None;
                        self.rahmen_lage = None;
                        if self.rahmen_an {
                            if let Some((x, y, b, h)) = quelle.lage() {
                                self.rahmen = Some(crate::fenster::Rahmen::zeigen(x, y, b, h));
                                self.rahmen_lage = Some((x, y, b, h));
                            }
                        }
                    }
                    Err(e) => {
                        a.stoppen();
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
                a.stoppen();
            }
            self.screen_encoder = None;
            self.screen_on = false;
            self.rahmen = None;
            self.rahmen_lage = None;
            self.own_screen = None;
            self.own_screen_state += 1;
            self.screen_notice = String::new();
            // Whoever no longer shares also releases nothing anymore. Open
            // requests are moot, granted releases must go -
            // otherwise a permission would remain that nobody sees anymore.
            if !self.steuer_erlaubt.is_empty() {
                self.steuer_erlaubt.clear();
                self.sig.revoke_control(None);
            }
            self.steuer_anfragen.clear();
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
        if self.bitrate_geprueft.elapsed() < std::time::Duration::from_secs(3) {
            return;
        }
        self.bitrate_geprueft = std::time::Instant::now();
        let schaetzung = self.ton.zahlen().bandbreite;
        let (k, s) = bitraten_verteilen(schaetzung, self.kamera_an, self.screen_on);
        if self.koder.is_some() && bitrate_lohnt_wechsel(self.koder_bitrate, k) {
            let (b, h, fps) = match (&self.kamera, self.avatar_an) {
                (Some(kam), _) => (kam.width_px, kam.height_px, 30),
                (None, true) => (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 20),
                _ => (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 30),
            };
            if let Ok(c) = crate::meetvideo::Kodierer::neu(b, h, fps, k) {
                self.koder = Some(c);
                self.koder_bitrate = k;
                self.bitrate_wechsel += 1;
            }
        }
        if self.screen_encoder.is_some() && bitrate_lohnt_wechsel(self.screen_bitrate, s) {
            if let Some(a) = self.screen.as_ref() {
                if let Ok(c) = crate::meetvideo::Kodierer::neu(a.width_px, a.height_px, 20, s) {
                    self.screen_encoder = Some(c);
                    self.screen_bitrate = s;
                    self.bitrate_wechsel += 1;
                }
            }
        }
    }

    fn kamera_pumpe(&mut self) {
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechstes_bild {
            return;
        }
        if self.avatar_an {
            self.avatar_pumpe(jetzt);
            return;
        }
        let (kam, kod) = match (self.kamera.as_ref(), self.koder.as_mut()) {
            (Some(k), Some(c)) => (k, c),
            _ => return,
        };
        let bild = match kam.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 33 ms = 30 frames per second. Before it was 66 ms (15/s) - that
        // together with the slow redrawing was the reason for the
        // choppy picture. The camera delivers 30 anyway.
        self.naechstes_bild = jetzt + std::time::Duration::from_millis(33);
        match kod.nv12_rahmen(&bild.nv12) {
            Ok(teile) => {
                for t in teile {
                    self.ton.send_frame(t.data);
                }
                self.bild_gesendet += 1;
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
                &bild.nv12,
                bild.width_px,
                bild.height_px,
                bild.width_px as usize,
                bild.height_px,
                &mut rgba,
            ) {
                self.eigen = Some((bild.width_px, bild.height_px, rgba));
                self.eigen_stand += 1;
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
    fn avatar_pumpe(&mut self, jetzt: std::time::Instant) {
        let Some(kod) = self.koder.as_mut() else { return };
        // 20 frames per second are enough: the avatar has little motion, and
        // it should not eat CPU time.
        self.naechstes_bild = jetzt + std::time::Duration::from_millis(50);
        self.avatar_takt += 1;
        let t = self.avatar_takt;
        self.avatar_mund = crate::avatar::mundstellung(self.pegel, self.avatar_mund);
        // A blink lasts three frames, then the next comes in
        // 2 to 6 seconds - regular blinking looks mechanical.
        let blinzeln = if t >= self.avatar_blinzelt && t < self.avatar_blinzelt + 3 {
            let i = (t - self.avatar_blinzelt) as f32;
            1.0 - (i - 1.0).abs()
        } else {
            if t >= self.avatar_blinzelt + 3 {
                // Derive an irregular interval from the tick
                // itself - no random generator needed for that.
                let streu = (t.wrapping_mul(2654435761) % 80) as u64;
                self.avatar_blinzelt = t + 40 + streu;
            }
            0.0
        };
        let atmen = (t as f32 * 0.06).sin();
        let (b, h) = (crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT);
        crate::avatar::zeichnen(
            &self.avatar,
            b,
            h,
            self.avatar_mund,
            blinzeln.clamp(0.0, 1.0),
            atmen,
            &mut self.avatar_rgb,
        );
        if !crate::meetcam::rgb_nach_nv12(&self.avatar_rgb, b, h, b, h, &mut self.avatar_nv12) {
            return;
        }
        match kod.nv12_rahmen(&self.avatar_nv12) {
            Ok(teile) => {
                for teil in teile {
                    self.ton.send_frame(teil.data);
                }
                self.bild_gesendet += 1;
            }
            Err(e) => self.camera_notice = format!("Kodierer: {}", e),
        }
        // Own preview: the avatar is exactly what the others see.
        let mut rgba = Vec::with_capacity((b * h * 4) as usize);
        for p in self.avatar_rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        self.eigen = Some((b, h, rgba));
        self.eigen_stand += 1;
    }

    /// Avatar instead of camera. Really switches the camera off in the process - otherwise
    /// it would keep running invisibly, and the light next to it too.
    pub fn avatar_schalten(&mut self, an: bool) {
        if an == self.avatar_an {
            return;
        }
        if an {
            if self.kamera.is_some() {
                if let Some(k) = self.kamera.take() {
                    k.stoppen();
                }
            }
            if self.koder.is_none() {
                match crate::meetvideo::Kodierer::neu(
                    crate::meetvideo::WIDTH,
                    crate::meetvideo::HEIGHT,
                    20,
                    AVATAR_BITRATE,
                ) {
                    Ok(c) => {
                        self.koder = Some(c);
                        self.koder_bitrate = AVATAR_BITRATE;
                    }
                    Err(e) => {
                        self.camera_notice = format!("Kein Kodierer: {}", e);
                        return;
                    }
                }
            }
            self.avatar_an = true;
            self.kamera_an = true;
            self.camera_notice = "Avatar statt Kamera".into();
        } else {
            self.avatar_an = false;
            self.koder = None;
            self.eigen = None;
            self.eigen_stand += 1;
            self.kamera_an = false;
            self.camera_notice = String::new();
        }
        if self.video_gemeldet {
            self.sig.is_muted("video", !self.kamera_an);
        }
    }

    /// Aussehen des Avatars aendern (wirkt beim naechsten Bild).
    pub fn avatar_setzen(&mut self, a: crate::avatar::Aussehen) {
        self.avatar = a;
    }

    /// Switch the camera on or off.
    pub fn kamera_schalten(&mut self, an: bool) {
        // Camera and avatar are mutually exclusive - there is only one video track.
        if an && self.avatar_an {
            self.avatar_schalten(false);
        }
        if an {
            if self.kamera.is_some() {
                return;
            }
            match crate::meetcam::open(
                self.kamera_geraet.clone(),
                crate::meetvideo::WIDTH,
                crate::meetvideo::HEIGHT,
                30,
            ) {
                Ok(k) => {
                    match crate::meetvideo::Kodierer::neu(k.width_px, k.height_px, 30, KAMERA_BITRATE) {
                        Ok(c) => {
                            self.camera_notice = format!("Kamera: {}", k.name);
                            self.kamera = Some(k);
                            self.koder = Some(c);
                            self.koder_bitrate = KAMERA_BITRATE;
                            self.kamera_an = true;
                        }
                        Err(e) => {
                            self.camera_notice = format!("Kein Kodierer: {}", e);
                            self.kamera_an = false;
                        }
                    }
                }
                Err(e) => {
                    self.camera_notice = format!("Keine Kamera: {}", e);
                    self.kamera_an = false;
                }
            }
        } else {
            if let Some(k) = self.kamera.take() {
                k.stoppen();
            }
            self.koder = None;
            self.eigen = None;
            self.eigen_stand += 1;
            self.kamera_an = false;
        }
        if self.video_gemeldet {
            self.sig.is_muted("video", !self.kamera_an);
        }
    }

    /// Name of the running camera (empty = off).
    pub fn kamera_name(&self) -> String {
        self.kamera.as_ref().map(|k| k.name.clone()).unwrap_or_default()
    }

    pub fn toggle_muted(&mut self, an: bool) {
        self.is_muted = an;
        self.ton.is_muted(an);
        if let Some(g) = &self.geraete {
            g.is_muted.store(an, std::sync::atomic::Ordering::Relaxed);
        }
        self.sig.is_muted("audio", an);
    }

    /// Change the camera in the RUNNING meeting. `geraet` = None is the
    /// default device. If the camera is running, it is reopened -
    /// otherwise the meeting remembers the choice for the next switch-on.
    pub fn kamera_waehlen(&mut self, geraet: Option<String>) {
        if self.kamera_geraet == geraet {
            return;
        }
        self.kamera_geraet = geraet;
        if self.kamera_an {
            // Off and on again: the capture thread is bound to the device,
            // changing while running is not supported.
            self.kamera_schalten(false);
            self.kamera_schalten(true);
        }
    }

    /// Change microphone and/or speaker in the running meeting.
    ///
    /// WHY both together: capture and playback hang on ONE
    /// building block (shared echo cancellation). Swapping them individually would only work
    /// by throwing away the echo estimate - then it whistles.
    pub fn ton_waehlen(&mut self, mikro: Option<String>, lautsprecher: Option<String>) {
        if self.mikro_geraet == mikro && self.lautsprecher_geraet == lautsprecher {
            return;
        }
        self.mikro_geraet = mikro.clone();
        self.lautsprecher_geraet = lautsprecher.clone();
        // First release the old one (drop stops the threads), then the new one.
        self.geraete = None;
        match meetaudio::start_devices(mikro, lautsprecher) {
            Ok(g) => {
                g.is_muted
                    .store(self.is_muted, std::sync::atomic::Ordering::Relaxed);
                g.sperre_an
                    .store(self.sperre_an, std::sync::atomic::Ordering::Relaxed);
                g.sperre_staerke.store(
                    self.sperre_staerke as u32,
                    std::sync::atomic::Ordering::Relaxed,
                );
                self.notice = format!("Mikrofon: {} / Lautsprecher: {}", g.eingang, g.ausgang);
                self.protokoll.push(self.notice.clone());
                self.geraete = Some(g);
            }
            Err(e) => {
                self.notice = format!("Kein Ton-Geraet: {}", e);
                self.protokoll.push(self.notice.clone());
            }
        }
    }

    /// Set the noise gate. Takes effect immediately, without rebuilding the audio.
    pub fn rauschsperre_setzen(&mut self, an: bool, staerke: u8) {
        self.sperre_an = an;
        self.sperre_staerke = staerke.min(100);
        if let Some(g) = &self.geraete {
            g.sperre_an.store(an, std::sync::atomic::Ordering::Relaxed);
            g.sperre_staerke.store(
                self.sperre_staerke as u32,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    /// Is the noise gate currently letting sound through? For display only - without
    /// this feedback the slider would be flying blind.
    pub fn sperre_offen(&self) -> bool {
        match &self.geraete {
            Some(g) => g.sperre_offen.load(std::sync::atomic::Ordering::Relaxed),
            None => true,
        }
    }

    /// Estimated noise floor (0..1) - shows WHAT the gate
    /// has currently adjusted to.
    pub fn sperre_grundrauschen(&self) -> f32 {
        match &self.geraete {
            Some(g) => {
                g.sperre_grund.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10_000.0
            }
            None => 0.0,
        }
    }

    /// Names of the audio devices currently in use (for display).
    pub fn ton_namen(&self) -> (String, String) {
        match &self.geraete {
            Some(g) => (g.eingang.clone(), g.ausgang.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// As VIEWER: say which part of the foreign screen I am
    /// looking at. The sender then cuts exactly that out of its native
    /// capture - same encode size, so same bandwidth,
    /// but real pixels instead of extrapolated ones.
    ///
    /// `scharf = false` means: please keep sending everything (then it stays
    /// plain magnifying).
    pub fn zoom_wunsch(&mut self, peer: u64, bereich: crate::meetschirm::Bereich, scharf: bool) {
        let will = if scharf {
            bereich
        } else {
            crate::meetschirm::GANZ
        };
        let jetzt = std::time::Instant::now();
        let neu = match self.letzter_wunsch {
            Some((p, alt)) => p != peer || crate::meetschirm::bereich_lohnt_wechsel(alt, will),
            None => true,
        };
        // Repeat every 3 s even without change: the sender forgets
        // wishes it no longer hears from (otherwise it would stay on the
        // section of a long-gone viewer).
        if !neu && jetzt < self.naechster_wunsch {
            return;
        }
        self.naechster_wunsch = jetzt + std::time::Duration::from_secs(3);
        self.letzter_wunsch = Some((peer, will));
        self.sig.bereichswunsch(peer, will.0, will.1, will.2, will.3);
    }

    /// As SENDER: build the section from all wishes and apply it.
    ///
    /// Runs on every pass, but only changes something on a worthwhile
    /// difference - every switch costs a keyframe and is
    /// visible as a small jump.
    fn bereich_pumpe(&mut self) {
        if !self.screen_on {
            if self.screen_region != crate::meetschirm::GANZ {
                self.screen_region = crate::meetschirm::GANZ;
            }
            return;
        }
        // Wishes we have not heard from for 8 s no
        // longer count - the viewer is gone or zoomed out again.
        let jetzt = std::time::Instant::now();
        self.wunsch_von
            .retain(|_, (_, wann)| jetzt.duration_since(*wann) < std::time::Duration::from_secs(8));
        let wuensche: Vec<crate::meetschirm::Bereich> =
            self.wunsch_von.values().map(|(b, _)| *b).collect();
        let neu = crate::meetschirm::bereich_vereinen(&wuensche);
        if neu == self.screen_region {
            return;
        }
        if self.screen_region != crate::meetschirm::GANZ
            && neu != crate::meetschirm::GANZ
            && !crate::meetschirm::bereich_lohnt_wechsel(self.screen_region, neu)
        {
            return;
        }
        self.screen_region = neu;
        if let Some(a) = self.screen.as_ref() {
            a.bereich_setzen(neu);
        }
        // The image content jumps - without a fresh keyframe the
        // others would see garbled video for a moment.
        if let Some(k) = self.screen_encoder.as_mut() {
            k.schluesselbild();
        }
        self.letztes_schluesselbild = jetzt;
        // And tell the others WHAT is coming now. Without this message
        // they would zoom once more into the already cropped image.
        self.sig.bereich(neu.0, neu.1, neu.2, neu.3);
    }

    /// Pointer of whoever is sharing (0..1). None = does not (yet) arrive.
    pub fn zeiger_des_teilers(&self, peer: u64) -> Option<(f32, f32)> {
        self.zeiger_von.get(&peer).copied()
    }

    /// The OWN mouse pointer on the currently shared screen.
    ///
    /// WHY extra: the server sends pointer positions only to the OTHERS -
    /// you never get your own back. Whoever looks at their own share
    /// therefore kept reading "Sharer's mouse (does not arrive)", although the
    /// information was right in front of them.
    pub fn eigener_zeiger(&self) -> Option<(f32, f32)> {
        let a = self.screen.as_ref()?;
        crate::meetschirm::zeiger_anteil_quelle(a.quelle)
    }

    /// Welchen Ausschnitt sendet dieser Participant gerade?
    pub fn bereich_des_teilers(&self, peer: u64) -> Option<crate::meetschirm::Bereich> {
        self.bereich_von.get(&peer).copied()
    }

    /// Can this participant deliver sections at all?
    pub fn kann_scharf(&self, peer: u64) -> bool {
        self.scharf_kann.contains(&peer)
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

    pub fn einlassen(&self, peer: u64) {
        self.sig.warteraum("admit", Some(peer));
    }
    pub fn abweisen(&self, peer: u64) {
        self.sig.warteraum("deny", Some(peer));
    }
    pub fn alle_einlassen(&self) {
        self.sig.warteraum("admit-all", None);
    }
    pub fn warteraum(&self, an: bool) {
        self.sig.warteraum(if an { "on" } else { "off" }, None);
    }

    pub fn verlassen(&self) {
        if let Some(k) = self.kamera.as_ref() {
            k.stoppen();
        }
        if let Some(a) = self.screen.as_ref() {
            a.stoppen();
            self.sig.roh(serde_json::json!({"t":"screen","on":false}));
        }
        self.sig.verlassen();
        self.ton.finish();
    }

    /// Name of a participant (for the chat).
    pub fn name_von(&self, id: u64) -> String {
        let z = self.sig.state();
        if id == z.ich {
            return "Du".into();
        }
        z.leute
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
        self.steuer_frei = an;
        self.sig.fernsteuerung(an, &self.ich_fvid);
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
        self.steuer_gefragt = true;
        self.sig.request_control();
        self.chat.push((0, "Du hast um die Steuerung gebeten".into()));
    }

    /// As sharer: accept or reject a request.
    pub fn reply_control(&mut self, peer: u64, ja: bool) {
        self.steuer_anfragen.retain(|(p, _)| *p != peer);
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
            if !self.steuer_erlaubt.contains(&peer) {
                self.steuer_erlaubt.push(peer);
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
            Some(p) => self.steuer_erlaubt.retain(|x| *x != p),
            None => self.steuer_erlaubt.clear(),
        }
        self.sig.revoke_control(peer);
        self.chat
            .push((0, "Steuerungsfreigabe zurueckgenommen".into()));
    }

    /// Offene Anfragen an mich (Participant, Name).
    pub fn steuer_anfragen(&self) -> Vec<(u64, String)> {
        self.steuer_anfragen.clone()
    }

    /// Wem ich es erlaubt habe (Participant, Name).
    pub fn steuer_erlaubt(&self) -> Vec<(u64, String)> {
        self.steuer_erlaubt
            .iter()
            .map(|p| (*p, self.name_von(*p)))
            .collect()
    }

    /// Meine Zusage: (Name des Teilenden, seine FreeViewer-Nummer).
    pub fn steuer_zusage(&self) -> Option<(String, String)> {
        self.steuer_zusage
            .as_ref()
            .map(|(p, f)| (self.name_von(*p), f.clone()))
    }

    /// Is someone ELSE sharing right now? Only then is the request button worth it - otherwise
    /// it would be a promise the server immediately rejects with
    /// "kein-teilender".
    pub fn fremder_teilt(&self) -> bool {
        let ich = self.sig.state().ich;
        self.schirme.bilder.keys().any(|id| *id != ich)
    }

    /// Who in the room can be remote controlled? (participant, name, FreeViewer no.)
    pub fn steuerbare(&self) -> Vec<(u64, String, String)> {
        let z = self.sig.state();
        z.leute
            .iter()
            .filter(|t| t.id != z.ich && !t.fvid.is_empty())
            .map(|t| (t.id, t.name.clone(), t.fvid.clone()))
            .collect()
    }

    /// Own FreeViewer number (for display).
    pub fn meine_fvid(&self) -> &str {
        &self.ich_fvid
    }

    pub fn bin_gastgeber(&self) -> bool {
        let z = self.sig.state();
        z.gastgeber != 0 && z.gastgeber == z.ich
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
pub struct Vorprobe {
    kamera: Option<crate::meetcam::Kamera>,
    ton: Option<meetaudio::Geraete>,
    /// Own image for display: (width, height, RGBA).
    pub eigen: Option<(u32, u32, Vec<u8>)>,
    /// Counts up as soon as a new image is there (reload texture).
    pub stand: u64,
    pub pegel: f32,
    pub notice: String,
    naechstes: std::time::Instant,
    /// What the user WANTS - separate from the actual state. Without this separation
    /// a device error would trigger a new attempt every frame.
    pub wunsch_kamera: bool,
    pub wunsch_mikro: bool,
    pub mikro_geraet: Option<String>,
}

impl Default for Vorprobe {
    fn default() -> Self {
        Vorprobe {
            kamera: None,
            ton: None,
            eigen: None,
            stand: 0,
            pegel: 0.0,
            notice: String::new(),
            naechstes: std::time::Instant::now(),
            wunsch_kamera: false,
            wunsch_mikro: false,
            mikro_geraet: None,
        }
    }
}

impl Vorprobe {
    pub fn kamera_an(&self) -> bool {
        self.kamera.is_some()
    }

    /// Switch the camera on or off for the preview.
    pub fn kamera(&mut self, an: bool) {
        self.wunsch_kamera = an;
        if an == self.kamera.is_some() {
            return;
        }
        if !an {
            if let Some(k) = self.kamera.take() {
                k.stoppen();
            }
            self.eigen = None;
            self.stand += 1;
            return;
        }
        match crate::meetcam::open(None, crate::meetvideo::WIDTH, crate::meetvideo::HEIGHT, 30) {
            Ok(k) => {
                self.notice = format!("Kamera: {}", k.name);
                self.kamera = Some(k);
            }
            Err(e) => {
                self.notice = format!("Keine Kamera: {}", e);
            }
        }
    }

    /// Switch the microphone on or off for the level bar.
    pub fn mikro(&mut self, an: bool, geraet: Option<String>) {
        let wechsel = geraet != self.mikro_geraet;
        self.wunsch_mikro = an;
        self.mikro_geraet = geraet.clone();
        if !an {
            self.ton = None;
            self.pegel = 0.0;
            return;
        }
        if self.ton.is_some() && !wechsel {
            return;
        }
        self.ton = None;
        match meetaudio::start_devices(geraet, None) {
            Ok(g) => self.ton = Some(g),
            Err(e) => self.notice = format!("Kein Mikrofon: {}", e),
        }
    }

    /// In jedem Bild aufrufen: Bild abholen, Pegel messen.
    pub fn pumpe(&mut self) {
        if let Some(g) = &self.ton {
            let mut spitze = 0.0f32;
            while let Ok(rahmen) = g.mikro.try_recv() {
                spitze = spitze.max(
                    rahmen
                        .iter()
                        .map(|v| (*v as f32 / 32768.0).abs())
                        .fold(0.0f32, f32::max),
                );
            }
            self.pegel = self.pegel * 0.7 + spitze * 0.3;
        }
        let jetzt = std::time::Instant::now();
        if jetzt < self.naechstes {
            return;
        }
        let kam = match self.kamera.as_ref() {
            Some(k) => k,
            None => return,
        };
        let bild = match kam.neuestes() {
            Some(b) => b,
            None => return,
        };
        // 12 frames per second are entirely enough for a preview.
        self.naechstes = jetzt + std::time::Duration::from_millis(80);
        let mut rgba = Vec::new();
        if crate::h264::nv12_to_rgba(
            &bild.nv12,
            bild.width_px,
            bild.height_px,
            bild.width_px as usize,
            bild.height_px,
            &mut rgba,
        ) {
            self.eigen = Some((bild.width_px, bild.height_px, rgba));
            self.stand += 1;
        }
        let f = kam.error();
        if !f.is_empty() {
            self.notice = f;
        }
    }

    /// Release everything again (before joining and on closing).
    pub fn aus(&mut self) {
        if let Some(k) = self.kamera.take() {
            k.stoppen();
        }
        self.wunsch_kamera = false;
        self.wunsch_mikro = false;
        self.mikro_geraet = None;
        self.ton = None;
        self.eigen = None;
        self.pegel = 0.0;
        self.stand += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vorprobe_ohne_kamera_und_mikrofon_meldet_sauber() {
        // On a server without camera/sound card the preview must not
        // blow up - it only has to report honestly that nothing is there.
        let mut v = Vorprobe::default();
        v.kamera(true);
        v.mikro(true, None);
        v.pumpe();
        #[cfg(not(windows))]
        assert!(!v.kamera_an(), "ohne Kamera darf keine laufen");
        assert!(v.eigen.is_none() || v.kamera_an());
        v.aus();
        assert!(!v.kamera_an());
        assert_eq!(v.pegel, 0.0);
    }

    #[test]
    fn geraeteliste_stuerzt_nicht_ab() {
        // On a server without sound card this must give an empty list
        // and not blow up.
        let (ein, aus) = meetaudio::geraete_liste();
        println!("Mikrofone {:?} Lautsprecher {:?}", ein.len(), aus.len());
    }

    #[test]
    fn kamera_schalten_ohne_kamera_meldet_sauber() {
        // Server without camera: the switch must set a message and
        // stay off - no crash, no hanging thread.
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None, None,);
        if let Ok(mut m) = r {
            m.kamera_schalten(true);
            #[cfg(not(windows))]
            {
                assert!(!m.kamera_an, "ohne Kamera darf sie nicht an sein");
                assert!(!m.camera_notice.is_empty(), "keine Meldung");
            }
            m.kamera_schalten(false);
            assert!(!m.kamera_an);
            m.verlassen();
        }
    }

    #[test]
    fn toggle_screen_without_display_reports_cleanly() {
        // Server without screen: report cleanly, do not crash.
        let r = NativMeet::beitreten("http://127.0.0.1:1", "000-000-000", "x", "T", "", false, None, None, None,);
        if let Ok(mut m) = r {
            m.toggle_screen(true, 0);
            if !m.screen_on {
                assert!(!m.screen_notice.is_empty(), "keine Meldung");
            }
            m.toggle_screen(false, 0);
            assert!(!m.screen_on);
            m.verlassen();
        }
    }

    #[test]
    fn control_without_number_is_not_released() {
        // Without own FreeViewer number the release would be a lie:
        // the others would get a button that leads nowhere.
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "",
            true,
            None,
            None, None,);
        if let Ok(mut m) = r {
            assert!(!m.steuer_frei, "ohne Nummer darf nichts freigegeben sein");
            m.release_control(true);
            assert!(!m.steuer_frei, "ohne Nummer darf die Freigabe nicht greifen");
            assert!(!m.notice.is_empty(), "keine Meldung");
            m.verlassen();
        }
    }

    #[test]
    fn control_with_number_switches() {
        let r = NativMeet::beitreten(
            "http://127.0.0.1:1",
            "000-000-000",
            "x",
            "T",
            "497628420",
            false,
            None,
            None, None,);
        if let Ok(mut m) = r {
            assert!(!m.steuer_frei, "beim Beitreten war die Freigabe aus");
            m.release_control(true);
            assert!(m.steuer_frei, "Freigabe hat nicht gegriffen");
            m.release_control(false);
            assert!(!m.steuer_frei, "Zuruecknehmen hat nicht gegriffen");
            assert_eq!(m.meine_fvid(), "497628420");
            // Without a connection the room knows nobody - the list
            // must still only run, not blow up.
            assert!(m.steuerbare().is_empty());
            m.verlassen();
        }
    }

    #[test]
    fn beitreten_ohne_server_meldet_sauber() {
        // Wrong port -> the signalling must not panic,
        // but must return an error or report a silent disconnect.
        let r = NativMeet::beitreten(
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
                assert!(!m.state().verbunden || m.state().last_error.is_empty());
                m.verlassen();
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
    fn ohne_messung_bleiben_die_voreinstellungen() {
        assert_eq!(
            bitraten_verteilen(0, true, true),
            (KAMERA_BITRATE, SCREEN_BITRATE)
        );
    }

    #[test]
    fn die_gemessene_leitung_wird_nicht_ueberschritten() {
        // Exactly the value the real server reported on 11.09.2026.
        let (k, s) = bitraten_verteilen(1_021_098, true, true);
        assert!(
            (k + s) as u64 <= 1_021_098,
            "zusammen {} bit/s ueber einer Leitung von 1021098",
            k + s
        );
        // And the old state would have sent 6 Mbit/s here:
        assert!(
            (k + s) < 2_000_000 + 4_000_000,
            "nicht besser als vorher"
        );
    }

    #[test]
    fn bitrate_bleibt_in_den_schranken() {
        let (k, s) = bitraten_verteilen(50_000, true, true);
        assert_eq!(k, KAMERA_MIN);
        assert_eq!(s, SCREEN_MIN);
        let (k, s) = bitraten_verteilen(500_000_000, true, true);
        assert_eq!(k, KAMERA_MAX);
        assert_eq!(s, SCREEN_MAX);
    }

    #[test]
    fn display_gets_more_than_camera() {
        let (k, s) = bitraten_verteilen(4_000_000, true, true);
        assert!(s > k, "Bildschirm {} <= Kamera {}", s, k);
    }

    #[test]
    fn kleine_schwankungen_setzen_den_kodierer_nicht_neu() {
        assert!(!bitrate_lohnt_wechsel(700_000, 750_000));
        assert!(!bitrate_lohnt_wechsel(700_000, 650_000));
        assert!(bitrate_lohnt_wechsel(700_000, 300_000));
        assert!(bitrate_lohnt_wechsel(300_000, 900_000));
    }
}
