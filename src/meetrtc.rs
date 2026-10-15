//! Native meeting - stage 1b: audio without a browser.
//!
//! This is where what the browser used to do happens: build a real
//! WebRTC connection to the media server (ICE, DTLS, SRTP) and send audio
//! back and forth in Opus. We use the same library as the
//! server itself (str0m) - what runs on one side is guaranteed to be understood
//! by the other.
//!
//! Division:
//!   meetsig.rs  - the control line (who is there, chat, hand, waiting room)
//!   meetrtc.rs  - this module: audio (later also video and screen)
//!
//! Deliberately without device access: this module receives video/audio as
//! number series and hands them out the same way. Microphone and
//! speaker (cpal) and the echo cancellation sit one level
//! above - so the whole network path can be measured on a server without
//! a sound card and not just claimed.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer};
use str0m::media::{Direction, KeyframeRequestKind, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

/// Sample rate and frame length as in the browser: 48 kHz, 20 ms, mono.
pub const RATE: u32 = 48_000;
pub const RAHMEN: usize = 960; // 20 ms bei 48 kHz

/// What the audio part reports upwards.
#[derive(Debug, Clone, PartialEq)]
pub enum TonEreignis {
    /// ICE and DTLS are up - audio flows from now on.
    Verbunden,
    /// Ein entschluesselter, dekodierter Tonrahmen eines anderen Teilnehmers.
    Rahmen { quelle: u64, pcm: Vec<i16> },
    /// An encoded video frame (H.264) of another participant. Decoding
    /// is done by the platform (Media Foundation / VideoToolbox),
    /// not by this module.
    Bild {
        quelle: u64,
        /// true = shared screen, false = camera. Without it camera
        /// and screen of the same participant would land in the same bucket.
        bildschirm: bool,
        daten: Vec<u8>,
        schluesselbild: bool,
        /// Which video format the server sends ("H264", "VP8", ...).
        /// Decides which decoder is used.
        codec: String,
    },
    FriendError(String),
    Ende(String),
}

/// Running numbers - so that claims can be checked by calculation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Zahlen {
    pub gesendet: u64,
    pub receive: u64,
    pub bild_gesendet: u64,
    pub frame_received: u64,
    /// Frames of the own SCREEN track (stage 3).
    pub screen_sent: u64,
    pub bytes_raus: u64,
    pub bytes_rein: u64,
    /// Lautstaerke des zuletzt empfangenen Rahmens (0..1).
    pub pegel_rein: f32,
    /// Audio frames that were dropped because of backlog. Previously this did not
    /// exist - the backlog stayed forever as delay.
    pub ton_verworfen: u64,
    /// Wie viele 20-ms-Rahmen gerade auf ihren Sendeschlitz warten.
    pub ton_stau: u32,
    /// Geschaetzte Sendebandbreite in bit/s (0 = keine Schaetzung).
    pub bandbreite: u64,
    pub verbunden: bool,
}

/// Control des Ton-Teils von aussen.
pub struct Ton {
    /// PCM frames that should go out (20 ms, mono, 48 kHz).
    raus: std::sync::mpsc::Sender<Vec<i16>>,
    /// Encoded video frames that should go out.
    bild_raus: std::sync::mpsc::Sender<Vec<u8>>,
    /// Kodierte Bildschirmrahmen (eigene Spur, Stufe 3).
    screen_out: std::sync::mpsc::Sender<Vec<u8>>,
    ereignisse: std::sync::mpsc::Receiver<TonEreignis>,
    zahlen: Arc<Mutex<Zahlen>>,
    is_muted: Arc<AtomicBool>,
    ende: Arc<AtomicBool>,
    /// The SDP offer that has to be sent to the server.
    pub angebot: String,
    /// m-line of our own audio (the server wants to know it in "publish").
    pub mid: String,
    /// m-line unseres eigenen Bildes.
    pub vid: String,
    /// m-line unserer eigenen Bildschirmfreigabe.
    pub vid2: String,
    /// Which m-line belongs to whom - and whether it is a screen
    /// (from the "track" messages of the signalling).
    spuren: Arc<Mutex<HashMap<String, (u64, bool)>>>,
    antwort: std::sync::mpsc::Sender<Sdp>,
}

#[derive(Debug, Clone)]
pub enum Sdp {
    /// Antwort auf UNSER Angebot.
    Antwort(String),
    /// The server offers something itself (new participants) - we answer.
    Angebot(String),
}

impl Ton {
    pub fn send_out(&self, pcm: Vec<i16>) {
        let _ = self.raus.send(pcm);
    }
    /// Send a fully encoded H.264 frame (Annex-B or AVCC
    /// without start code - str0m packs it into RTP itself).
    pub fn send_frame(&self, daten: Vec<u8>) {
        let _ = self.bild_raus.send(daten);
    }
    /// Send an H.264 frame of the SCREEN track. Own track,
    /// so that the other side can separate camera and screen.
    pub fn screen_send(&self, daten: Vec<u8>) {
        let _ = self.screen_out.send(daten);
    }
    pub fn abholen(&self) -> Vec<TonEreignis> {
        let mut v = Vec::new();
        while let Ok(e) = self.ereignisse.try_recv() {
            v.push(e);
        }
        v
    }
    pub fn zahlen(&self) -> Zahlen {
        self.zahlen.lock().map(|z| *z).unwrap_or_default()
    }
    pub fn is_muted(&self, an: bool) {
        self.is_muted.store(an, Ordering::Relaxed);
    }
    /// Antwort des Servers auf unser Angebot einspielen.
    pub fn antwort(&self, sdp: &str) {
        let _ = self.antwort.send(Sdp::Antwort(sdp.to_string()));
    }
    /// An offer from the server - the answer comes back as an event.
    pub fn server_angebot(&self, sdp: &str) {
        let _ = self.antwort.send(Sdp::Angebot(sdp.to_string()));
    }
    /// Fetch the answer SDP that arose in response to a server offer.
    pub fn offene_antwort(&self) -> Option<String> {
        ANTWORT_RAUS.lock().ok().and_then(|mut a| a.pop())
    }
    /// Add the mapping from the signalling afterwards (camera).
    pub fn spur(&self, mid: &str, peer: u64) {
        self.spur_art(mid, peer, false);
    }
    /// Zuordnung samt Streamart nachtragen.
    pub fn spur_art(&self, mid: &str, peer: u64, bildschirm: bool) {
        if let Ok(mut m) = self.spuren.lock() {
            m.insert(mid.to_string(), (peer, bildschirm));
        }
    }
    pub fn finish(&self) {
        self.ende.store(true, Ordering::Relaxed);
    }
}

impl Drop for Ton {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Answers that the audio thread leaves for the signalling.
static ANTWORT_RAUS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Starts the audio part: builds the offer and then runs in the background.
pub fn launch() -> Result<Ton> {
    start_with(None)
}

/// Like `launch`, but with an end-to-end key.
///
/// Without a key everything stays as before (encrypted only in transit).
/// WITH a key the payload is made secret already HERE - the server
/// then distributes only gibberish. Video and audio are treated
/// differently: with video the H.264 structure stays readable (otherwise the
/// server cannot packetise, measured on 07.08.2026), with audio
/// the whole Opus frame is encrypted.
pub fn start_with(e2e: Option<crate::meete2e::Key>) -> Result<Ton> {
    // Krypto-Anbieter einmalig setzen (reines Rust, s. Cargo.toml).
    static EINMAL: std::sync::Once = std::sync::Once::new();
    EINMAL.call_once(|| {
        str0m::crypto::from_feature_flags().install_process_default();
    });

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_nonblocking(true)?;
    let lokal = socket.local_addr()?;

    // bandwidth estimation on. Without it the client keeps sending stubbornly with a
    // fixed bitrate, even when the line has long been full -
    // the packets then pile up in the router (bufferbloat) and the
    // image arrives seconds too late. The browser does exactly this
    // on its own, our native client did not so far.
    let mut rtc = Rtc::builder()
        .enable_bwe(Some(str0m::bwe::Bitrate::bps(2_000_000)))
        .build(Instant::now());
    rtc.bwe().set_desired_bitrate(str0m::bwe::Bitrate::bps(2_500_000));
    // Candidates: every own address that is not loopback. The server is
    // publicly reachable - so we only need to be able to
    // call out, a STUN server is not necessary for that.
    for ip in eigene_adressen() {
        let addr = SocketAddr::new(ip, lokal.port());
        if let Ok(k) = Candidate::host(addr, "udp") {
            let _ = rtc.add_local_candidate(k);
        }
    }

    let mut api = rtc.sdp_api();
    let mid = api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
    // Second track for video. H.264, because Windows and Mac have a hardware
    // unit for it (Media Foundation or VideoToolbox) and every
    // browser understands it. We could only do VP8 in software - that would be
    // an imposition on older machines.
    let vid = api.add_media(MediaKind::Video, Direction::SendRecv, None, None, None);
    // Third track: the shared screen. Deliberately separate from the
    // camera - otherwise you disappear from the round yourself as soon as you show
    // something, and the other side cannot arrange "screen large,
    // camera small".
    let vid2 = api.add_media(MediaKind::Video, Direction::SendRecv, None, None, None);
    let (angebot, offen) = api
        .apply()
        .ok_or_else(|| anyhow!("Angebot laesst sich nicht bauen"))?;
    let angebot_sdp = angebot.to_sdp_string();

    let (raus_tx, raus_rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let (bild_tx, bild_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (screen_tx, screen_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<TonEreignis>();
    let (sdp_tx, sdp_rx) = std::sync::mpsc::channel::<Sdp>();
    let zahlen = Arc::new(Mutex::new(Zahlen::default()));
    let spuren: Arc<Mutex<HashMap<String, (u64, bool)>>> = Arc::new(Mutex::new(HashMap::new()));
    let is_muted = Arc::new(AtomicBool::new(false));
    let ende = Arc::new(AtomicBool::new(false));

    let z2 = zahlen.clone();
    let sp2 = spuren.clone();
    let s2 = is_muted.clone();
    let e2 = ende.clone();
    std::thread::Builder::new()
        .name("meetrtc".into())
        .spawn(move || {
            let r = lauf(
                rtc, offen, socket, mid, vid, vid2, raus_rx, bild_rx, screen_rx, sdp_rx, &ev_tx,
                &z2, &s2, &e2, &sp2, e2e,
            );
            let text = match r {
                Ok(()) => String::new(),
                Err(e) => e.to_string(),
            };
            let _ = ev_tx.send(TonEreignis::Ende(text));
        })
        .map_err(|e| anyhow!("Ton-Faden: {}", e))?;

    Ok(Ton {
        raus: raus_tx,
        bild_raus: bild_tx,
        screen_out: screen_tx,
        ereignisse: ev_rx,
        zahlen,
        is_muted,
        ende,
        angebot: angebot_sdp,
        mid: mid.to_string(),
        vid: vid.to_string(),
        vid2: vid2.to_string(),
        spuren,
        antwort: sdp_tx,
    })
}

/// All own IP addresses except loopback. Without third-party code: we ask the
/// operating system by opening a connection "into the blue" (no packet
/// flows) and looking at which address it chooses for it.
fn eigene_adressen() -> Vec<std::net::IpAddr> {
    let mut aus = Vec::new();
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:53").is_ok() {
            if let Ok(a) = s.local_addr() {
                if !a.ip().is_loopback() {
                    aus.push(a.ip());
                }
            }
        }
    }
    aus
}

#[allow(clippy::too_many_arguments)]
fn lauf(
    mut rtc: Rtc,
    offen: str0m::change::SdpPendingOffer,
    socket: UdpSocket,
    mid: Mid,
    vid: Mid,
    vid2: Mid,
    raus: std::sync::mpsc::Receiver<Vec<i16>>,
    bild_raus: std::sync::mpsc::Receiver<Vec<u8>>,
    screen_out: std::sync::mpsc::Receiver<Vec<u8>>,
    sdp: std::sync::mpsc::Receiver<Sdp>,
    ev: &std::sync::mpsc::Sender<TonEreignis>,
    zahlen: &Arc<Mutex<Zahlen>>,
    is_muted: &Arc<AtomicBool>,
    ende: &Arc<AtomicBool>,
    spuren: &Arc<Mutex<HashMap<String, (u64, bool)>>>,
    e2e: Option<crate::meete2e::Key>,
) -> Result<()> {
    // Wait for the answer (comes in via the signalling).
    let mut offen = Some(offen);
    let start = Instant::now();
    let mut kodierer = audiopus::coder::Encoder::new(
        audiopus::SampleRate::Hz48000,
        audiopus::Channels::Mono,
        audiopus::Application::Voip,
    )
    .map_err(|e| anyhow!("Opus-Kodierer: {:?}", e))?;
    let mut dekodierer: HashMap<u64, audiopus::coder::Decoder> = HashMap::new();
    // End-to-end: one `Sender` per outgoing stream (microphone, camera,
    // screen share). Each draws its own random salt, so no two streams -
    // nor a later rejoin - can ever repeat a (key, nonce) pair (roadmap r1).
    let mut e2e_ton = e2e.as_ref().map(|k| k.sender(crate::meete2e::Spur::Ton));
    let mut e2e_video = e2e.as_ref().map(|k| k.sender(crate::meete2e::Spur::Bild));
    let mut e2e_screen = e2e.as_ref().map(|k| k.sender(crate::meete2e::Spur::Bild));
    // Audio mids from which we already saw old-format (v1) frames.
    let mut e2e_old_format_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // And per incoming track a guard against replay.
    let mut e2e_wachen: HashMap<String, crate::meete2e::Wache> = HashMap::new();
    let mut puffer = vec![0u8; 4000];
    let mut aus_puffer = vec![0u8; 2000];
    // Which m-line belongs to whom? The server tells us via the
    // signalling ("track"), it is passed in from outside.
    let mut mid_zu_peer: HashMap<String, u64> = HashMap::new();
    let mut naechster_rahmen = Instant::now();
    let mut rtp_zeit: u64 = 0;
    let mut verbunden_gemeldet = false;
    let mut pt: Option<str0m::media::Pt> = None;
    let mut vpt: Option<str0m::media::Pt> = None;
    let mut bild_zeit: u64 = 0;
    let mut spt: Option<str0m::media::Pt> = None;
    let mut screen_time: u64 = 0;
    // Whoever joins in the middle of a running stream only gets P frames -
    // without a key frame no decoder in the world decodes anything. So actively
    // request one (the browser does the same) until one arrives.
    let mut has_key: HashMap<String, bool> = HashMap::new();
    // Audio frames waiting for their 20 ms slot. Bounded - see
    // stau_abbauen().
    let mut ton_warte: std::collections::VecDeque<Vec<i16>> =
        std::collections::VecDeque::new();
    // Only for the control experiment: with FV_TON_STAU=999999 the
    // client behaves like the old version (no draining). This way one can measure
    // that the draining really hits the cause and the test is not lying.
    let ton_stau_max: usize = std::env::var("FV_TON_STAU")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(TON_STAU_MAX);
    let mut zuletzt_gefragt: HashMap<String, Instant> = HashMap::new();

    loop {
        if ende.load(Ordering::Relaxed) {
            return Ok(());
        }

        // ---- SDP from the signalling --------------------------------------
        while let Ok(s) = sdp.try_recv() {
            match s {
                Sdp::Antwort(text) => {
                    let antwort = SdpAnswer::from_sdp_string(&text)
                        .map_err(|e| anyhow!("Antwort unlesbar: {:?}", e))?;
                    if let Some(o) = offen.take() {
                        rtc.sdp_api()
                            .accept_answer(o, antwort)
                            .map_err(|e| anyhow!("Antwort abgelehnt: {}", e))?;
                    }
                }
                Sdp::Angebot(text) => {
                    let angebot = SdpOffer::from_sdp_string(&text)
                        .map_err(|e| anyhow!("Angebot unlesbar: {:?}", e))?;
                    match rtc.sdp_api().accept_offer(angebot) {
                        Ok(a) => {
                            if let Ok(mut v) = ANTWORT_RAUS.lock() {
                                v.push(a.to_sdp_string());
                            }
                        }
                        Err(e) => {
                            let _ = ev.send(TonEreignis::FriendError(format!("Angebot: {}", e)));
                        }
                    }
                }
            }
        }

        // ---- determine payload type for Opus -------------------------------
        // Once the answer of the server is applied, the m-line says
        // which identifier (PT) was negotiated for Opus. Previously
        // we waited for an event - but that only comes for
        // FOREIGN tracks, so nothing was ever sent.
        if pt.is_none() {
            let pts: Vec<str0m::media::Pt> = rtc
                .media(mid)
                .map(|m| m.remote_pts().to_vec())
                .unwrap_or_default();
            if !pts.is_empty() {
                let gefunden = rtc
                    .codec_config()
                    .iter()
                    .find(|c| pts.contains(&c.pt()) && c.spec().codec == str0m::format::Codec::Opus)
                    .map(|c| c.pt());
                if let Some(p) = gefunden {
                    pt = Some(p);
                }
            }
        }

        // ---- eigenen Ton verschicken ---------------------------------------
        if rtc.is_alive() && Instant::now() >= naechster_rahmen {
            naechster_rahmen += Duration::from_millis(20);
            // If the thread stalled for longer, the slot lies far in the
            // past - then do not rush to catch up, but
            // start anew. Otherwise a flood comes instead of a stream.
            let jetzt_t = Instant::now();
            if naechster_rahmen + Duration::from_millis(100) < jetzt_t {
                naechster_rahmen = jetzt_t;
            }
            // Collect everything the UI has delivered since the last time,
            // and cap the backlog.
            while let Ok(p) = raus.try_recv() {
                ton_warte.push_back(p);
            }
            let weg = stau_abbauen(&mut ton_warte, ton_stau_max);
            if weg > 0 {
                if let Ok(mut z) = zahlen.lock() {
                    z.ton_verworfen += weg as u64;
                }
            }
            if let Ok(mut z) = zahlen.lock() {
                z.ton_stau = ton_warte.len() as u32;
            }
            if let Some(pcm) = ton_warte.pop_front() {
                if !is_muted.load(Ordering::Relaxed) && pt.is_some() {
                    let n = kodierer
                        .encode(&pcm, &mut aus_puffer)
                        .map_err(|e| anyhow!("Opus: {:?}", e))?;
                    let mut paket = aus_puffer[..n].to_vec();
                    if let Some(s) = e2e_ton.as_mut() {
                        // Encrypt audio completely - the server does not
                        // touch Opus content.
                        paket = s.schuetzen(false, &paket);
                    }
                    let wanduhr = start + start.elapsed();
                    let zeit = MediaTime::new(rtp_zeit, str0m::media::Frequency::FORTY_EIGHT_KHZ);
                    if let Some(mut w) = rtc.writer(mid) {
                        if let Some(p) = pt {
                            if let Err(e) = w.write(p, wanduhr, zeit, paket.clone()) {
                                let _ = ev.send(TonEreignis::FriendError(format!("senden: {}", e)));
                            } else if let Ok(mut z) = zahlen.lock() {
                                z.gesendet += 1;
                                z.bytes_raus += paket.len() as u64;
                            }
                        }
                    }
                    rtp_zeit += RAHMEN as u64;
                }
            }
        }

        // ---- eigenes Bild verschicken --------------------------------------
        if rtc.is_alive() {
            if let Ok(daten) = bild_raus.try_recv() {
                if vpt.is_none() {
                    let pts: Vec<str0m::media::Pt> = rtc
                        .media(vid)
                        .map(|m| m.remote_pts().to_vec())
                        .unwrap_or_default();
                    if !pts.is_empty() {
                        vpt = rtc
                            .codec_config()
                            .iter()
                            .find(|c| {
                                pts.contains(&c.pt()) && c.spec().codec == str0m::format::Codec::H264
                            })
                            .map(|c| c.pt());
                    }
                }
                if let (Some(p), Some(mut w)) = (vpt, rtc.writer(vid)) {
                    let wanduhr = start + start.elapsed();
                    let zeit = MediaTime::new(bild_zeit, str0m::media::Frequency::NINETY_KHZ);
                    // Video: only the CONTENTS of the NAL units become secret,
                    // the structure stays readable - otherwise the server cannot
                    // packetise.
                    let daten = match e2e_video.as_mut() {
                        Some(s) => s.schuetzen_h264(&daten),
                        None => daten,
                    };
                    let laenge = daten.len() as u64;
                    if let Err(e) = w.write(p, wanduhr, zeit, daten) {
                        let _ = ev.send(TonEreignis::FriendError(format!("Bild senden: {}", e)));
                    } else if let Ok(mut z) = zahlen.lock() {
                        z.bild_gesendet += 1;
                        z.bytes_raus += laenge;
                    }
                    // 90 kHz: one frame at 30 frames/s is 3000 steps.
                    bild_zeit += 3000;
                }
            }
        }

        // ---- Display verschicken ----------------------------------------
        if rtc.is_alive() {
            if let Ok(daten) = screen_out.try_recv() {
                if spt.is_none() {
                    let pts: Vec<str0m::media::Pt> = rtc
                        .media(vid2)
                        .map(|m| m.remote_pts().to_vec())
                        .unwrap_or_default();
                    if !pts.is_empty() {
                        spt = rtc
                            .codec_config()
                            .iter()
                            .find(|c| {
                                pts.contains(&c.pt()) && c.spec().codec == str0m::format::Codec::H264
                            })
                            .map(|c| c.pt());
                    }
                }
                if let (Some(p), Some(mut w)) = (spt, rtc.writer(vid2)) {
                    let wanduhr = start + start.elapsed();
                    let zeit = MediaTime::new(screen_time, str0m::media::Frequency::NINETY_KHZ);
                    let daten = match e2e_screen.as_mut() {
                        Some(s) => s.schuetzen_h264(&daten),
                        None => daten,
                    };
                    let laenge = daten.len() as u64;
                    if let Err(e) = w.write(p, wanduhr, zeit, daten) {
                        let _ = ev.send(TonEreignis::FriendError(format!("Bildschirm senden: {}", e)));
                    } else if let Ok(mut z) = zahlen.lock() {
                        z.screen_sent += 1;
                        z.bytes_raus += laenge;
                    }
                    screen_time += 3000;
                }
            }
        }

        // ---- str0m antreiben ------------------------------------------------
        let bis = match rtc.poll_output() {
            Ok(Output::Timeout(t)) => t,
            Ok(Output::Transmit(t)) => {
                let _ = socket.send_to(&t.contents, t.destination);
                continue;
            }
            Ok(Output::Event(e)) => {
                match e {
                    Event::Connected => {
                        if let Ok(mut z) = zahlen.lock() {
                            z.verbunden = true;
                        }
                        if !verbunden_gemeldet {
                            verbunden_gemeldet = true;
                            let _ = ev.send(TonEreignis::Verbunden);
                        }
                    }
                    Event::MediaAdded(_) => {}
                    Event::EgressBitrateEstimate(k) => {
                        // Only measure and make visible. Adjusting the bitrate
                        // automatically afterwards is the
                        // next step - first I want to see whether
                        // the server gives feedback at all.
                        let bps = match k {
                            str0m::bwe::BweKind::Twcc(b) => b.as_u64(),
                            str0m::bwe::BweKind::Remb(_, b) => b.as_u64(),
                            // str0m darf spaeter weitere Arten kennen.
                            _ => 0,
                        };
                        if let Ok(mut z) = zahlen.lock() {
                            z.bandbreite = bps;
                        }
                    }
                    Event::MediaData(d) if d.pt.to_string() != "0" && ist_video(&rtc, d.mid) => {
                        let (peer, bildschirm) = spuren
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&d.mid.to_string()).copied())
                            .unwrap_or((0, false));
                        if let Ok(mut z) = zahlen.lock() {
                            z.frame_received += 1;
                            z.bytes_rein += d.data.len() as u64;
                        }
                        // End-to-end: open up BEFORE anything happens with the
                        // content. The key-frame marker can also be read
                        // on the encrypted frame (NAL type 5
                        // stays readable) - so the key
                        // is not needed for that.
                        let schluessel = ist_schluesselbild(&d.data);
                        let inhalt: Vec<u8> = match &e2e {
                            Some(k) => match k.open_h264_guard(
                                e2e_wachen.entry(d.mid.to_string()).or_default(),
                                &d.data,
                            ) {
                                Some(klar) => klar,
                                None => {
                                    // Wrong key, forged, or a replayed frame
                                    // (r60): do NOT show it. Better no picture
                                    // than one that cannot be trusted.
                                    continue;
                                }
                            },
                            None => d.data.to_vec(),
                        };
                        // As long as no key frame has come: politely but
                        // persistently request one (at most every second).
                        let mid_txt = d.mid.to_string();
                        let gesehen = *has_key.get(&mid_txt).unwrap_or(&false);
                        if schluessel {
                            has_key.insert(mid_txt.clone(), true);
                        } else if !gesehen {
                            let faellig = zuletzt_gefragt
                                .get(&mid_txt)
                                .map(|t| t.elapsed() >= Duration::from_millis(1000))
                                .unwrap_or(true);
                            if faellig {
                                zuletzt_gefragt.insert(mid_txt.clone(), Instant::now());
                                if let Some(strom) =
                                    rtc.direct_api().stream_rx_by_mid(d.mid, d.rid)
                                {
                                    strom.request_keyframe(KeyframeRequestKind::Pli);
                                }
                            }
                        }
                        let codec = rtc
                            .codec_config()
                            .iter()
                            .find(|c| c.pt() == d.pt)
                            .map(|c| format!("{:?}", c.spec().codec))
                            .unwrap_or_else(|| "?".into());
                        let _ = ev.send(TonEreignis::Bild {
                            quelle: peer,
                            bildschirm,
                            daten: inhalt,
                            schluesselbild: schluessel,
                            codec,
                        });
                    }
                    Event::MediaData(d) => {
                        let peer = spuren
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&d.mid.to_string()).map(|(p, _)| *p))
                            .or_else(|| mid_zu_peer.get(&d.mid.to_string()).copied())
                            .unwrap_or(0);
                        let dek = dekodierer.entry(peer).or_insert_with(|| {
                            audiopus::coder::Decoder::new(
                                audiopus::SampleRate::Hz48000,
                                audiopus::Channels::Mono,
                            )
                            .expect("Opus-Dekodierer")
                        });
                        // End-to-end: first open up, then decode.
                        // Replay is fended off per track - otherwise
                        // someone could feed old audio snippets in again.
                        let ton_inhalt: Vec<u8> = match &e2e {
                            Some(k) => {
                                let kopf = crate::meete2e::salz_von(&d.data)
                                    .zip(crate::meete2e::counter_from(&d.data));
                                let wache = e2e_wachen.entry(d.mid.to_string()).or_default();
                                match (kopf, k.oeffnen(crate::meete2e::Spur::Ton, &d.data)) {
                                    (Some((salz, zz)), Some(klar)) if wache.pruefen(&salz, zz) => klar,
                                    _ => {
                                        // A participant on the old, nonce-reusing
                                        // format: tell the user once per stream.
                                        if crate::meete2e::ist_altes_format(&d.data)
                                            && e2e_old_format_seen.insert(d.mid.to_string())
                                        {
                                            let _ = ev.send(TonEreignis::FriendError(
                                                "A participant uses an outdated end-to-end format - they must update."
                                                    .to_string(),
                                            ));
                                        }
                                        continue;
                                    }
                                }
                            }
                            None => d.data.to_vec(),
                        };
                        let mut pcm = vec![0i16; RAHMEN];
                        {
                            if let Ok(n) = dek.decode(Some(&ton_inhalt[..]), &mut pcm[..], false) {
                                pcm.truncate(n);
                                let pegel = pcm
                                    .iter()
                                    .map(|s| (*s as f32 / 32768.0).abs())
                                    .fold(0.0f32, f32::max);
                                if let Ok(mut z) = zahlen.lock() {
                                    z.receive += 1;
                                    z.bytes_rein += d.data.len() as u64;
                                    z.pegel_rein = pegel;
                                }
                                let _ = ev.send(TonEreignis::Rahmen { quelle: peer, pcm });
                            }
                        }
                    }
                    Event::IceConnectionStateChange(s) => {
                        if s == str0m::IceConnectionState::Disconnected {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                continue;
            }
            Err(e) => return Err(anyhow!("str0m: {}", e)),
        };

        // ---- wait and receive -------------------------------------------
        let jetzt = Instant::now();
        let warte = bis.saturating_duration_since(jetzt).min(Duration::from_millis(10));
        socket.set_read_timeout(Some(warte.max(Duration::from_millis(1))))?;
        socket.set_nonblocking(false)?;
        match socket.recv_from(&mut puffer) {
            Ok((n, von)) => {
                let ziel = socket.local_addr()?;
                if let Ok(inhalt) = (&puffer[..n]).try_into() {
                    let _ = rtc.handle_input(Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source: von,
                            destination: ziel,
                            contents: inhalt,
                        },
                    ));
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(anyhow!("UDP: {}", e)),
        }
        let _ = rtc.handle_input(Input::Timeout(Instant::now()));
        if !rtc.is_alive() {
            return Ok(());
        }
        let _ = &mut mid_zu_peer;
    }
}

/// At most this many 20 ms audio frames may wait for their send slot.
/// 3 frames = 60 ms. More brings nothing: sending is strictly in
/// 20 ms steps, so every frame above that would raise the delay
/// permanently instead of draining it.
pub const TON_STAU_MAX: usize = 3;

/// Drain the backlog in the audio send buffer: the OLDEST frames are dropped.
///
/// WHY AT ALL: The microphone delivers 20 ms frames in real time, the
/// UI however only collects them at the drawing rate - if it stalls once
/// (dragging a window, sharing the screen, opening the camera), several
/// frames arrive at once. Still only one is sent per 20 ms. Without
/// draining this lead stays as delay for the rest of the
/// conversation and adds up with every further stall -
/// that is exactly how the 5 to 10 seconds arise.
///
/// Dropping happens at the FRONT, not at the back: the listener should hear the CURRENT
/// word, not an old one. Returns how many frames were dropped.
pub fn stau_abbauen(warte: &mut std::collections::VecDeque<Vec<i16>>, max: usize) -> usize {
    let mut weg = 0usize;
    while warte.len() > max {
        warte.pop_front();
        weg += 1;
    }
    weg
}

/// Does this m-line belong to video?
/// Does this access point contain a key frame (IDR) or at least
/// a parameter set (SPS)? Annex-B: start code, then NAL header.
pub fn ist_schluesselbild(daten: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 4 < daten.len() {
        // Startcode 00 00 01 oder 00 00 00 01
        if daten[i] == 0 && daten[i + 1] == 0 {
            let kopf = if daten[i + 2] == 1 {
                Some(i + 3)
            } else if daten[i + 2] == 0 && daten[i + 3] == 1 {
                Some(i + 4)
            } else {
                None
            };
            if let Some(k) = kopf {
                if k < daten.len() {
                    let typ = daten[k] & 0x1f;
                    if typ == 5 || typ == 7 {
                        return true;
                    }
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }
    false
}

fn ist_video(rtc: &Rtc, mid: Mid) -> bool {
    rtc.media(mid)
        .map(|m| m.kind() == MediaKind::Video)
        .unwrap_or(false)
}

/// A test tone (sine) - for measurements without a microphone.
pub fn testton(phase: &mut f32, hz: f32) -> Vec<i16> {
    let mut v = Vec::with_capacity(RAHMEN);
    let schritt = 2.0 * std::f32::consts::PI * hz / RATE as f32;
    for _ in 0..RAHMEN {
        v.push((phase.sin() * 12000.0) as i16);
        *phase += schritt;
        if *phase > 2.0 * std::f32::consts::PI {
            *phase -= 2.0 * std::f32::consts::PI;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The actual measurement: how many milliseconds of audio are still in the send buffer
    /// after a stall of the UI?
    ///
    /// Setup: the microphone delivers 500 frames (10 seconds of audio) at
    /// once - the way it happens after a hang. Afterwards sending is
    /// done as in operation, one frame per 20 ms slot.
    fn rueckstau_in_ms(max: usize, geliefert: usize) -> usize {
        let mut w: std::collections::VecDeque<Vec<i16>> = std::collections::VecDeque::new();
        for _ in 0..geliefert {
            w.push_back(vec![0i16; RAHMEN]);
        }
        super::stau_abbauen(&mut w, max);
        // Was uebrig bleibt, wartet - je Rahmen 20 ms Verzoegerung.
        w.len() * 20
    }

    #[test]
    fn verzoegerung_waechst_nicht_mehr_ueber_die_schranke() {
        // 500 frames at once = 10 seconds of audio in the buffer.
        let bleibt = rueckstau_in_ms(TON_STAU_MAX, 500);
        assert!(
            bleibt <= 60,
            "nach dem Abbau warten noch {} ms Ton - das ist die Verzoegerung, die bleibt",
            bleibt
        );
    }

    #[test]
    fn gegenprobe_ohne_abbau_bleiben_sekunden_stehen() {
        // The same setup, but with a limit that never takes effect - this is how
        // the old version computed. If THIS test does not fail,
        // the upper one measures nothing.
        let bleibt = rueckstau_in_ms(usize::MAX, 500);
        assert_eq!(
            bleibt, 10_000,
            "ohne Abbau muessten 10 Sekunden stehen bleiben"
        );
        assert!(
            bleibt > 5_000,
            "die Gegenprobe zeigt kein Problem - dann taugt die Messung nicht"
        );
    }

    #[test]
    fn abbau_wirft_das_alte_weg_und_behaelt_das_neue() {
        let mut w: std::collections::VecDeque<Vec<i16>> = std::collections::VecDeque::new();
        for n in 0..10i16 {
            w.push_back(vec![n; RAHMEN]);
        }
        let weg = super::stau_abbauen(&mut w, 3);
        assert_eq!(weg, 7);
        assert_eq!(w.len(), 3);
        // The youngest frame (9) must be kept, the oldest gone.
        assert_eq!(w.back().unwrap()[0], 9, "der neueste Ton wurde weggeworfen");
        assert_eq!(w.front().unwrap()[0], 7, "es wurde vom falschen Ende geworfen");
    }

    #[test]
    fn schluesselbild_wird_im_ganzen_zugriffspunkt_gefunden() {
        // SPS(7) + PPS(8) + IDR(5), each with start code - this is how it comes out of
        // the unpacking of str0m.
        let mut au = vec![0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e];
        au.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xce]);
        au.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88, 0x84]);
        assert!(ist_schluesselbild(&au));
        // Reines P-Bild (Typ 1) - kein Schluesselbild.
        let p = vec![0, 0, 0, 1, 0x41, 0x9a, 0x21, 0x6c, 0x45, 0xff];
        assert!(!ist_schluesselbild(&p));
        // The IDR is FAR back - the old check only looked at 8 bytes.
        let mut spaet = vec![0, 0, 0, 1, 0x41];
        spaet.extend(std::iter::repeat(0x33).take(60));
        spaet.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x11]);
        assert!(ist_schluesselbild(&spaet), "IDR hinter dem 8. Byte uebersehen");
    }

    #[test]
    fn test_tone_has_right_length_and_oscillates() {
        let mut p = 0.0;
        let a = testton(&mut p, 440.0);
        assert_eq!(a.len(), RAHMEN);
        let spitze = a.iter().map(|s| s.abs()).max().unwrap();
        assert!(spitze > 8000, "Testton zu leise: {}", spitze);
        // second frame continues the oscillation, so does not start at 0
        let b = testton(&mut p, 440.0);
        assert_eq!(b.len(), RAHMEN);
    }

    #[test]
    fn opus_round_trips() {
        let mut enc = audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Mono,
            audiopus::Application::Voip,
        )
        .unwrap();
        let mut dec = audiopus::coder::Decoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Mono,
        )
        .unwrap();
        let mut p = 0.0;
        let ton = testton(&mut p, 440.0);
        let mut aus = vec![0u8; 2000];
        let n = enc.encode(&ton, &mut aus).unwrap();
        assert!(n > 10, "Opus-Paket zu klein: {}", n);
        let mut go_back = vec![0i16; RAHMEN];
        let m = dec.decode(Some(&aus[..n]), &mut go_back[..], false).unwrap();
        assert_eq!(m, RAHMEN);
        let spitze = go_back.iter().map(|s| s.abs()).max().unwrap();
        assert!(spitze > 4000, "Dekodierter Ton zu leise: {}", spitze);
    }

    #[test]
    fn angebot_enthaelt_opus_und_eine_tonspur() {
        let ton = launch().expect("Ton startet");
        assert!(ton.angebot.contains("m=audio"), "kein Ton im Angebot");
        assert!(
            ton.angebot.to_lowercase().contains("opus"),
            "kein Opus im Angebot"
        );
        assert!(!ton.mid.is_empty());
        ton.finish();
    }

    #[test]
    fn angebot_enthaelt_auch_eine_bildspur_mit_h264() {
        let ton = launch().expect("startet");
        assert!(ton.angebot.contains("m=video"), "keine Bildspur im Angebot");
        assert!(
            ton.angebot.to_uppercase().contains("H264"),
            "kein H.264 im Angebot"
        );
        assert!(!ton.vid.is_empty(), "keine Kennung fuer die Bildspur");
        assert_ne!(ton.vid, ton.mid, "Bild und Ton auf derselben Spur");
    }

    #[test]
    fn offer_has_third_track_for_display() {
        let ton = launch().expect("startet");
        let zeilen = ton.angebot.lines().filter(|l| l.starts_with("m=")).count();
        assert_eq!(zeilen, 3, "Angebot:\n{}", ton.angebot);
        assert_ne!(ton.vid2, ton.vid, "Bildschirm und Kamera auf derselben Spur");
        assert_ne!(ton.vid2, ton.mid, "Bildschirm und Ton auf derselben Spur");
        assert_eq!(
            ton.angebot.matches("H264").count() >= 2,
            true,
            "beide Bildspuren muessen H.264 anbieten"
        );
        ton.finish();
    }
}
