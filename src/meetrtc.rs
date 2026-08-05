//! Natives Meeting - Stufe 1b: Ton ohne Browser.
//!
//! Hier passiert das, was bisher der Browser gemacht hat: eine echte
//! WebRTC-Verbindung zum Medienserver aufbauen (ICE, DTLS, SRTP) und Ton in
//! Opus hin- und herschicken. Wir benutzen dieselbe Bibliothek wie der
//! Server selbst (str0m) - was auf der einen Seite laeuft, versteht die
//! andere garantiert.
//!
//! Aufteilung:
//!   meetsig.rs  - die Steuerleitung (wer ist da, Chat, Hand, Warteraum)
//!   meetrtc.rs  - dieses Modul: Ton (spaeter auch Bild und Bildschirm)
//!
//! Bewusst ohne Geraete-Zugriff: dieses Modul bekommt Bild/Ton als
//! Zahlenreihen herein und gibt sie so wieder heraus. Mikrofon und
//! Lautsprecher (cpal) und die Echo-Unterdrueckung haengen eine Ebene
//! darueber - so laesst sich der ganze Netzweg auf einem Server ohne
//! Soundkarte messen und nicht nur behaupten.

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpOffer};
use str0m::media::{Direction, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

/// Abtastrate und Rahmenlaenge wie im Browser: 48 kHz, 20 ms, Mono.
pub const RATE: u32 = 48_000;
pub const RAHMEN: usize = 960; // 20 ms bei 48 kHz

/// Was der Ton-Teil nach oben meldet.
#[derive(Debug, Clone, PartialEq)]
pub enum TonEreignis {
    /// ICE und DTLS stehen - ab jetzt fliesst Ton.
    Verbunden,
    /// Ein entschluesselter, dekodierter Tonrahmen eines anderen Teilnehmers.
    Rahmen { quelle: u64, pcm: Vec<i16> },
    /// Ein kodierter Bildrahmen (H.264) eines anderen Teilnehmers. Das
    /// Dekodieren macht die Plattform (Media Foundation / VideoToolbox),
    /// nicht dieses Modul.
    Bild {
        quelle: u64,
        /// true = geteilter Bildschirm, false = Kamera. Ohne das laegen
        /// Kamera und Bildschirm desselben Teilnehmers im selben Topf.
        bildschirm: bool,
        daten: Vec<u8>,
        schluesselbild: bool,
        /// Welches Bildformat der Server schickt ("H264", "VP8", ...).
        /// Entscheidet, welcher Dekodierer drankommt.
        codec: String,
    },
    Fehler(String),
    Ende(String),
}

/// Laufende Zahlen - damit man Behauptungen nachrechnen kann.
#[derive(Debug, Clone, Copy, Default)]
pub struct Zahlen {
    pub gesendet: u64,
    pub empfangen: u64,
    pub bild_gesendet: u64,
    pub bild_empfangen: u64,
    /// Rahmen der eigenen BILDSCHIRM-Spur (Stufe 3).
    pub schirm_gesendet: u64,
    pub bytes_raus: u64,
    pub bytes_rein: u64,
    /// Lautstaerke des zuletzt empfangenen Rahmens (0..1).
    pub pegel_rein: f32,
    pub verbunden: bool,
}

/// Steuerung des Ton-Teils von aussen.
pub struct Ton {
    /// PCM-Rahmen, die raus sollen (20 ms, Mono, 48 kHz).
    raus: std::sync::mpsc::Sender<Vec<i16>>,
    /// Kodierte Bildrahmen, die raus sollen.
    bild_raus: std::sync::mpsc::Sender<Vec<u8>>,
    /// Kodierte Bildschirmrahmen (eigene Spur, Stufe 3).
    schirm_raus: std::sync::mpsc::Sender<Vec<u8>>,
    ereignisse: std::sync::mpsc::Receiver<TonEreignis>,
    zahlen: Arc<Mutex<Zahlen>>,
    stumm: Arc<AtomicBool>,
    ende: Arc<AtomicBool>,
    /// Das SDP-Angebot, das an den Server geschickt werden muss.
    pub angebot: String,
    /// m-line unseres eigenen Tons (der Server will sie im "publish" wissen).
    pub mid: String,
    /// m-line unseres eigenen Bildes.
    pub vid: String,
    /// m-line unserer eigenen Bildschirmfreigabe.
    pub vid2: String,
    /// Welche m-line gehoert zu wem - und ob sie ein Bildschirm ist
    /// (aus den "track"-Meldungen der Signalisierung).
    spuren: Arc<Mutex<HashMap<String, (u64, bool)>>>,
    antwort: std::sync::mpsc::Sender<Sdp>,
}

#[derive(Debug, Clone)]
pub enum Sdp {
    /// Antwort auf UNSER Angebot.
    Antwort(String),
    /// Der Server bietet selbst etwas an (neue Teilnehmer) - wir antworten.
    Angebot(String),
}

impl Ton {
    pub fn senden(&self, pcm: Vec<i16>) {
        let _ = self.raus.send(pcm);
    }
    /// Einen fertig kodierten H.264-Rahmen verschicken (Annex-B oder AVCC
    /// ohne Startcode - str0m packt ihn selbst in RTP).
    pub fn bild_senden(&self, daten: Vec<u8>) {
        let _ = self.bild_raus.send(daten);
    }
    /// Einen H.264-Rahmen der BILDSCHIRM-Spur verschicken. Eigene Spur,
    /// damit die Gegenseite Kamera und Bildschirm trennen kann.
    pub fn schirm_senden(&self, daten: Vec<u8>) {
        let _ = self.schirm_raus.send(daten);
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
    pub fn stumm(&self, an: bool) {
        self.stumm.store(an, Ordering::Relaxed);
    }
    /// Antwort des Servers auf unser Angebot einspielen.
    pub fn antwort(&self, sdp: &str) {
        let _ = self.antwort.send(Sdp::Antwort(sdp.to_string()));
    }
    /// Ein Angebot des Servers - die Antwort kommt als Ereignis zurueck.
    pub fn server_angebot(&self, sdp: &str) {
        let _ = self.antwort.send(Sdp::Angebot(sdp.to_string()));
    }
    /// Antwort-SDP abholen, das auf ein Server-Angebot hin entstanden ist.
    pub fn offene_antwort(&self) -> Option<String> {
        ANTWORT_RAUS.lock().ok().and_then(|mut a| a.pop())
    }
    /// Zuordnung aus der Signalisierung nachtragen (Kamera).
    pub fn spur(&self, mid: &str, peer: u64) {
        self.spur_art(mid, peer, false);
    }
    /// Zuordnung samt Streamart nachtragen.
    pub fn spur_art(&self, mid: &str, peer: u64, bildschirm: bool) {
        if let Ok(mut m) = self.spuren.lock() {
            m.insert(mid.to_string(), (peer, bildschirm));
        }
    }
    pub fn beenden(&self) {
        self.ende.store(true, Ordering::Relaxed);
    }
}

impl Drop for Ton {
    fn drop(&mut self) {
        self.beenden();
    }
}

/// Antworten, die der Ton-Faden fuer die Signalisierung hinterlegt.
static ANTWORT_RAUS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Startet den Ton-Teil: baut das Angebot und laeuft danach im Hintergrund.
pub fn starten() -> Result<Ton> {
    // Krypto-Anbieter einmalig setzen (reines Rust, s. Cargo.toml).
    static EINMAL: std::sync::Once = std::sync::Once::new();
    EINMAL.call_once(|| {
        str0m::crypto::from_feature_flags().install_process_default();
    });

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_nonblocking(true)?;
    let lokal = socket.local_addr()?;

    let mut rtc = Rtc::builder().build(Instant::now());
    // Kandidaten: jede eigene Adresse, die kein Loopback ist. Der Server ist
    // oeffentlich erreichbar - wir muessen also nur hinaus telefonieren
    // koennen, ein STUN-Server ist dafuer nicht noetig.
    for ip in eigene_adressen() {
        let addr = SocketAddr::new(ip, lokal.port());
        if let Ok(k) = Candidate::host(addr, "udp") {
            let _ = rtc.add_local_candidate(k);
        }
    }

    let mut api = rtc.sdp_api();
    let mid = api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
    // Zweite Spur fuer Bild. H.264, weil Windows und Mac dafuer eine
    // Hardware-Einheit haben (Media Foundation bzw. VideoToolbox) und jeder
    // Browser es versteht. VP8 koennten wir nur in Software - das waere auf
    // aelteren Rechnern eine Zumutung.
    let vid = api.add_media(MediaKind::Video, Direction::SendRecv, None, None, None);
    // Dritte Spur: der geteilte Bildschirm. Bewusst getrennt von der
    // Kamera - sonst verschwindet man selbst aus der Runde, sobald man
    // etwas zeigt, und die Gegenseite kann nicht "Bildschirm gross,
    // Kamera klein" anordnen.
    let vid2 = api.add_media(MediaKind::Video, Direction::SendRecv, None, None, None);
    let (angebot, offen) = api
        .apply()
        .ok_or_else(|| anyhow!("Angebot laesst sich nicht bauen"))?;
    let angebot_sdp = angebot.to_sdp_string();

    let (raus_tx, raus_rx) = std::sync::mpsc::channel::<Vec<i16>>();
    let (bild_tx, bild_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (schirm_tx, schirm_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (ev_tx, ev_rx) = std::sync::mpsc::channel::<TonEreignis>();
    let (sdp_tx, sdp_rx) = std::sync::mpsc::channel::<Sdp>();
    let zahlen = Arc::new(Mutex::new(Zahlen::default()));
    let spuren: Arc<Mutex<HashMap<String, (u64, bool)>>> = Arc::new(Mutex::new(HashMap::new()));
    let stumm = Arc::new(AtomicBool::new(false));
    let ende = Arc::new(AtomicBool::new(false));

    let z2 = zahlen.clone();
    let sp2 = spuren.clone();
    let s2 = stumm.clone();
    let e2 = ende.clone();
    std::thread::Builder::new()
        .name("meetrtc".into())
        .spawn(move || {
            let r = lauf(
                rtc, offen, socket, mid, vid, vid2, raus_rx, bild_rx, schirm_rx, sdp_rx, &ev_tx,
                &z2, &s2, &e2, &sp2,
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
        schirm_raus: schirm_tx,
        ereignisse: ev_rx,
        zahlen,
        stumm,
        ende,
        angebot: angebot_sdp,
        mid: mid.to_string(),
        vid: vid.to_string(),
        vid2: vid2.to_string(),
        spuren,
        antwort: sdp_tx,
    })
}

/// Alle eigenen IP-Adressen ausser Loopback. Ohne Fremdcode: wir fragen das
/// Betriebssystem, indem wir eine Verbindung "ins Blaue" oeffnen (es fliesst
/// dabei kein Paket) und schauen, welche Adresse es dafuer waehlt.
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
    schirm_raus: std::sync::mpsc::Receiver<Vec<u8>>,
    sdp: std::sync::mpsc::Receiver<Sdp>,
    ev: &std::sync::mpsc::Sender<TonEreignis>,
    zahlen: &Arc<Mutex<Zahlen>>,
    stumm: &Arc<AtomicBool>,
    ende: &Arc<AtomicBool>,
    spuren: &Arc<Mutex<HashMap<String, (u64, bool)>>>,
) -> Result<()> {
    // Auf die Antwort warten (kommt ueber die Signalisierung herein).
    let mut offen = Some(offen);
    let start = Instant::now();
    let mut kodierer = audiopus::coder::Encoder::new(
        audiopus::SampleRate::Hz48000,
        audiopus::Channels::Mono,
        audiopus::Application::Voip,
    )
    .map_err(|e| anyhow!("Opus-Kodierer: {:?}", e))?;
    let mut dekodierer: HashMap<u64, audiopus::coder::Decoder> = HashMap::new();
    let mut puffer = vec![0u8; 4000];
    let mut aus_puffer = vec![0u8; 2000];
    // Welche m-line gehoert zu wem? Das sagt uns der Server ueber die
    // Signalisierung ("track"), es wird von aussen hereingereicht.
    let mut mid_zu_peer: HashMap<String, u64> = HashMap::new();
    let mut naechster_rahmen = Instant::now();
    let mut rtp_zeit: u64 = 0;
    let mut verbunden_gemeldet = false;
    let mut pt: Option<str0m::media::Pt> = None;
    let mut vpt: Option<str0m::media::Pt> = None;
    let mut bild_zeit: u64 = 0;
    let mut spt: Option<str0m::media::Pt> = None;
    let mut schirm_zeit: u64 = 0;

    loop {
        if ende.load(Ordering::Relaxed) {
            return Ok(());
        }

        // ---- SDP von der Signalisierung -----------------------------------
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
                            let _ = ev.send(TonEreignis::Fehler(format!("Angebot: {}", e)));
                        }
                    }
                }
            }
        }

        // ---- Nutzlastkennung fuer Opus bestimmen ---------------------------
        // Sobald die Antwort des Servers eingespielt ist, steht in der
        // m-line, welche Kennung (PT) fuer Opus ausgehandelt wurde. Frueher
        // haben wir auf ein Ereignis gewartet - das kommt aber nur fuer
        // FREMDE Spuren, deshalb wurde nie etwas gesendet.
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
            if let Ok(pcm) = raus.try_recv() {
                if !stumm.load(Ordering::Relaxed) && pt.is_some() {
                    let n = kodierer
                        .encode(&pcm, &mut aus_puffer)
                        .map_err(|e| anyhow!("Opus: {:?}", e))?;
                    let paket = aus_puffer[..n].to_vec();
                    let wanduhr = start + start.elapsed();
                    let zeit = MediaTime::new(rtp_zeit, str0m::media::Frequency::FORTY_EIGHT_KHZ);
                    if let Some(mut w) = rtc.writer(mid) {
                        if let Some(p) = pt {
                            if let Err(e) = w.write(p, wanduhr, zeit, paket.clone()) {
                                let _ = ev.send(TonEreignis::Fehler(format!("senden: {}", e)));
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
                    let laenge = daten.len() as u64;
                    if let Err(e) = w.write(p, wanduhr, zeit, daten) {
                        let _ = ev.send(TonEreignis::Fehler(format!("Bild senden: {}", e)));
                    } else if let Ok(mut z) = zahlen.lock() {
                        z.bild_gesendet += 1;
                        z.bytes_raus += laenge;
                    }
                    // 90 kHz: ein Rahmen bei 30 Bildern/s sind 3000 Schritte.
                    bild_zeit += 3000;
                }
            }
        }

        // ---- Bildschirm verschicken ----------------------------------------
        if rtc.is_alive() {
            if let Ok(daten) = schirm_raus.try_recv() {
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
                    let zeit = MediaTime::new(schirm_zeit, str0m::media::Frequency::NINETY_KHZ);
                    let laenge = daten.len() as u64;
                    if let Err(e) = w.write(p, wanduhr, zeit, daten) {
                        let _ = ev.send(TonEreignis::Fehler(format!("Bildschirm senden: {}", e)));
                    } else if let Ok(mut z) = zahlen.lock() {
                        z.schirm_gesendet += 1;
                        z.bytes_raus += laenge;
                    }
                    schirm_zeit += 3000;
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
                    Event::MediaData(d) if d.pt.to_string() != "0" && ist_video(&rtc, d.mid) => {
                        let (peer, bildschirm) = spuren
                            .lock()
                            .ok()
                            .and_then(|m| m.get(&d.mid.to_string()).copied())
                            .unwrap_or((0, false));
                        if let Ok(mut z) = zahlen.lock() {
                            z.bild_empfangen += 1;
                            z.bytes_rein += d.data.len() as u64;
                        }
                        let schluessel = d
                            .data
                            .iter()
                            .take(8)
                            .any(|b| (*b & 0x1f) == 5 || (*b & 0x1f) == 7);
                        let codec = rtc
                            .codec_config()
                            .iter()
                            .find(|c| c.pt() == d.pt)
                            .map(|c| format!("{:?}", c.spec().codec))
                            .unwrap_or_else(|| "?".into());
                        let _ = ev.send(TonEreignis::Bild {
                            quelle: peer,
                            bildschirm,
                            daten: d.data.to_vec(),
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
                        let mut pcm = vec![0i16; RAHMEN];
                        {
                            if let Ok(n) = dek.decode(Some(&d.data[..]), &mut pcm[..], false) {
                                pcm.truncate(n);
                                let pegel = pcm
                                    .iter()
                                    .map(|s| (*s as f32 / 32768.0).abs())
                                    .fold(0.0f32, f32::max);
                                if let Ok(mut z) = zahlen.lock() {
                                    z.empfangen += 1;
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

        // ---- warten und empfangen -------------------------------------------
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

/// Gehoert diese m-line zum Bild?
fn ist_video(rtc: &Rtc, mid: Mid) -> bool {
    rtc.media(mid)
        .map(|m| m.kind() == MediaKind::Video)
        .unwrap_or(false)
}

/// Ein Testton (Sinus) - fuer Messungen ohne Mikrofon.
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

    #[test]
    fn testton_hat_die_richtige_laenge_und_schwingt() {
        let mut p = 0.0;
        let a = testton(&mut p, 440.0);
        assert_eq!(a.len(), RAHMEN);
        let spitze = a.iter().map(|s| s.abs()).max().unwrap();
        assert!(spitze > 8000, "Testton zu leise: {}", spitze);
        // zweiter Rahmen setzt die Schwingung fort, faengt also nicht bei 0 an
        let b = testton(&mut p, 440.0);
        assert_eq!(b.len(), RAHMEN);
    }

    #[test]
    fn opus_kann_hin_und_zurueck() {
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
        let mut zurueck = vec![0i16; RAHMEN];
        let m = dec.decode(Some(&aus[..n]), &mut zurueck[..], false).unwrap();
        assert_eq!(m, RAHMEN);
        let spitze = zurueck.iter().map(|s| s.abs()).max().unwrap();
        assert!(spitze > 4000, "Dekodierter Ton zu leise: {}", spitze);
    }

    #[test]
    fn angebot_enthaelt_opus_und_eine_tonspur() {
        let ton = starten().expect("Ton startet");
        assert!(ton.angebot.contains("m=audio"), "kein Ton im Angebot");
        assert!(
            ton.angebot.to_lowercase().contains("opus"),
            "kein Opus im Angebot"
        );
        assert!(!ton.mid.is_empty());
        ton.beenden();
    }

    #[test]
    fn angebot_enthaelt_auch_eine_bildspur_mit_h264() {
        let ton = starten().expect("startet");
        assert!(ton.angebot.contains("m=video"), "keine Bildspur im Angebot");
        assert!(
            ton.angebot.to_uppercase().contains("H264"),
            "kein H.264 im Angebot"
        );
        assert!(!ton.vid.is_empty(), "keine Kennung fuer die Bildspur");
        assert_ne!(ton.vid, ton.mid, "Bild und Ton auf derselben Spur");
    }

    #[test]
    fn angebot_enthaelt_eine_dritte_spur_fuer_den_bildschirm() {
        let ton = starten().expect("startet");
        let zeilen = ton.angebot.lines().filter(|l| l.starts_with("m=")).count();
        assert_eq!(zeilen, 3, "Angebot:\n{}", ton.angebot);
        assert_ne!(ton.vid2, ton.vid, "Bildschirm und Kamera auf derselben Spur");
        assert_ne!(ton.vid2, ton.mid, "Bildschirm und Ton auf derselben Spur");
        assert_eq!(
            ton.angebot.matches("H264").count() >= 2,
            true,
            "beide Bildspuren muessen H.264 anbieten"
        );
        ton.beenden();
    }
}
