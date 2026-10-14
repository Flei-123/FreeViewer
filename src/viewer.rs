//! Viewer side: connect to a remote FreeViewer host through the relay.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use crate::clip::Clip;
use crate::crypto::{self, Cipher};
use crate::encoder::blit_rgb_to_rgba;
use crate::net;
use crate::proto::{decode, encode, Msg};
use crate::shared::{FrameData, Shared};

/// Keeps the last complete picture so that delta updates can be painted into it.
struct Canvas {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
    seq: u64,
}

impl Canvas {
    fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            rgba: Vec::new(),
            seq: 0,
        }
    }

    fn set_full(&mut self, w: u32, h: u32, rgba: Vec<u8>) {
        self.w = w;
        self.h = h;
        self.rgba = rgba;
        self.seq += 1;
    }

    fn matches(&self, w: u32, h: u32) -> bool {
        self.w == w && self.h == h && self.rgba.len() == w as usize * h as usize * 4
    }

    fn publish(&self, shared: &Arc<Shared>) {
        *shared.frame.lock().unwrap() = Some(FrameData {
            width: self.w,
            height: self.h,
            rgba: self.rgba.clone(),
            seq: shared.next_frame_seq(),
        });
    }
}

/// Keeps the local clipboard in sync with the remote one. Runs on its own
/// thread because the platform clipboard handles are not `Send`.
fn clipboard_worker(shared: Arc<Shared>) {
    let mut clip = Clip::new();
    if !clip.available() {
        return;
    }
    while shared.connected.load(Ordering::Relaxed) {
        let incoming = shared.clip_in.lock().unwrap().take();
        if let Some(text) = incoming {
            if shared.clip_on.load(Ordering::Relaxed) { clip.set(&text); }
        }
        if let Some(text) = clip.poll().filter(|_| shared.clip_on.load(Ordering::Relaxed)) {
            shared.send_input(Msg::Clipboard { text });
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// How this viewer wants to get in.
#[derive(Clone, Debug)]
pub enum Auth {
    /// classic: the session password of the host
    Password(String),
    /// no password - the person over there has to allow the session
    Ask,
}

pub async fn run_viewer(shared: Arc<Shared>, id: String, password: String) {
    run_viewer_auth(shared, id, Auth::Password(password)).await
}

/// Knock instead of unlocking: the host shows a dialog and decides.
pub async fn run_viewer_ask(shared: Arc<Shared>, id: String) {
    run_viewer_auth(shared, id, Auth::Ask).await
}

/// Why a viewer session ended. Typed so the UI can show a plain sentence
/// and the reconnect logic can tell "try again" from "give up".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    /// The relay says the host ID is not online.
    Offline,
    /// The host already has a session.
    Busy,
    /// The host refused our password proof (or locked us out after too
    /// many wrong ones - the wire cannot tell the two apart).
    WrongPassword,
    /// "Ask" mode: the person at the host said no or did not answer.
    Declined,
    /// The host's counter-proof was wrong (or missing while required): the
    /// other end does not know the password - possibly a man in the middle.
    HostUnverified,
    /// The relay reported that the host side left.
    PeerEnded,
    /// The relay closed the connection cleanly.
    Closed,
    /// Socket / TLS / WebSocket trouble on our side or on the way.
    Network(String),
    /// The other side said something we do not understand.
    Protocol(String),
}

impl std::fmt::Display for Fail {
    /// Technical text, for the log only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Network(d) => write!(f, "network: {}", d),
            Fail::Protocol(d) => write!(f, "protocol: {}", d),
            other => write!(f, "{:?}", other),
        }
    }
}

impl std::error::Error for Fail {}

impl Fail {
    /// Sentence for the person in front of the screen.
    pub fn user_text(&self) -> &'static str {
        use crate::i18n::t;
        match self {
            Fail::Offline => t("viewer.err.offline"),
            Fail::Busy => t("viewer.err.busy"),
            Fail::WrongPassword => t("viewer.err.wrong_pw"),
            Fail::Declined => t("viewer.err.declined"),
            Fail::HostUnverified => t("viewer.err.host_unverified"),
            Fail::PeerEnded => t("viewer.err.peer_ended"),
            Fail::Closed => t("viewer.err.closed"),
            Fail::Network(_) => t("viewer.err.network"),
            Fail::Protocol(_) => t("viewer.err.protocol"),
        }
    }

    /// Worth trying again by itself? A wrong password or a "no" never is.
    pub fn retryable(&self) -> bool {
        !matches!(
            self,
            Fail::WrongPassword | Fail::Declined | Fail::HostUnverified | Fail::Protocol(_)
        )
    }

    /// Typed error if there is one, otherwise it is socket trouble.
    pub fn from_error(e: &anyhow::Error) -> Fail {
        match e.downcast_ref::<Fail>() {
            Some(f) => f.clone(),
            None => Fail::Network(e.to_string()),
        }
    }
}

/// Reconnects in a row without a live session before we give up.
pub const MAX_RECONNECTS: u32 = 6;

/// Wait before reconnect number `attempt` (1-based): 1, 2, 4, 8, 16, 30 s.
pub fn backoff(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.saturating_sub(1).min(5);
    Duration::from_secs(secs.min(30))
}

/// Should the viewer reconnect by itself? `ever_live`: this run had a
/// working session before. `attempt`: reconnects already tried since then.
/// Only a session that was working gets rebuilt - a first connect that fails
/// shows its error right away. "Ask" mode never reconnects silently: every
/// new try would pop a question on the other screen.
pub fn next_retry(fail: &Fail, ever_live: bool, ask: bool, attempt: u32) -> Option<Duration> {
    if ask || !ever_live || !fail.retryable() || attempt >= MAX_RECONNECTS {
        return None;
    }
    Some(backoff(attempt + 1))
}

fn reconnect_text(secs: u64, attempt: u32) -> String {
    crate::i18n::t("viewer.reconnecting")
        .replacen("{}", &secs.to_string(), 1)
        .replacen("{}", &attempt.to_string(), 1)
        .replacen("{}", &MAX_RECONNECTS.to_string(), 1)
}

pub async fn run_viewer_auth(shared: Arc<Shared>, id: String, auth: Auth) {
    let ask = matches!(auth, Auth::Ask);
    let mut ever_live = false;
    let mut attempt: u32 = 0;
    shared.viewer_failed.store(false, Ordering::Relaxed);

    loop {
        shared.connecting.store(true, Ordering::Relaxed);
        if attempt == 0 {
            shared.set_viewer_status(format!("Verbinde mit {} ...", id));
        } else {
            shared.set_viewer_status(format!("Verbinde neu mit {} ...", id));
        }

        let result = viewer_once(&shared, &id, &auth).await;
        let was_live = shared.connected.load(Ordering::Relaxed);

        shared.connected.store(false, Ordering::Relaxed);
        if let Some(mut x) = shared.xfer.lock().unwrap_or_else(|p| p.into_inner()).take() {
            x.shutdown();
        }
        *shared.input_tx.lock().unwrap_or_else(|p| p.into_inner()) = None;
        crate::vinput::set_active(false);

        let fail = match &result {
            Ok(()) => Fail::Closed,
            Err(e) => Fail::from_error(e),
        };
        crate::capture::log_line(&format!("viewer ended: {} (live: {})", fail, was_live));
        if was_live {
            // a working session: the counter starts over
            ever_live = true;
            attempt = 0;
        }

        match next_retry(&fail, ever_live, ask, attempt) {
            Some(wait) => {
                attempt += 1;
                // keep the last picture and the "connecting" state while
                // waiting, so the window does not jump around
                let total = wait.as_secs().max(1);
                for left in (1..=total).rev() {
                    shared.set_viewer_status(reconnect_text(left, attempt));
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            None => {
                shared.connecting.store(false, Ordering::Relaxed);
                *shared.frame.lock().unwrap_or_else(|p| p.into_inner()) = None;
                shared.viewer_failed.store(true, Ordering::Relaxed);
                let text = if ever_live && attempt >= MAX_RECONNECTS {
                    crate::i18n::t("viewer.gave_up").to_string()
                } else if was_live && matches!(fail, Fail::Closed) {
                    crate::i18n::t("viewer.ended").to_string()
                } else {
                    fail.user_text().to_string()
                };
                shared.set_viewer_status(text);
                return;
            }
        }
    }
}

async fn viewer_once(shared: &Arc<Shared>, id: &str, auth: &Auth) -> Result<()> {
    let ws = net::connect(&shared.relay_url).await?;
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMsg>();

    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
    });

    tx.send(WsMsg::text(net::json_connect(id)))?;

    let kp = crypto::keypair();
    let mut cipher: Option<Arc<Mutex<Cipher>>> = None;
    let mut session_key: Option<[u8; 32]> = None;
    // host counter-proof we expect in the host's 0x04 (password mode)
    let mut expected_host_proof: Option<[u8; 32]> = None;
    let started = Instant::now();
    let mut canvas = Canvas::new();
    // rolling stats for the session bar
    let mut win_start = Instant::now();
    let mut win_frames = 0u32;
    let mut win_bytes = 0usize;
    let mut ping_task: Option<tokio::task::JoinHandle<()>> = None;
    // voice link of this session (dropped when the session ends)
    let mut voice: Option<crate::audio::Voice> = None;
    // H.264 runs on its own thread: the decoder is a COM object (not Send)
    // and decoding must never stall the socket.
    let mut video: Option<VideoPipe> = None;
    let mut p2p: Option<Arc<crate::p2p::P2p>> = None;

    let res: Result<()> = loop {
        let item = match stream.next().await {
            Some(i) => i,
            None => break Ok(()),
        };
        let msg = match item {
            Ok(m) => m,
            Err(e) => break Err(anyhow::Error::new(Fail::Network(e.to_string()))),
        };

        match msg {
            WsMsg::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(t.as_str()).unwrap_or(serde_json::Value::Null);
                match net::msg_type(&v) {
                    "paired" => {
                        shared.set_viewer_status("Gekoppelt - Authentifizierung...");
                        let mut hello = Vec::with_capacity(34);
                        hello.push(crypto::TAG_HELLO);
                        hello.extend_from_slice(&kp.public);
                        // ask the host for its counter-proof (password mode
                        // only; ask mode has no shared secret). Old hosts
                        // ignore the extra byte.
                        if matches!(auth, Auth::Password(_)) {
                            hello.push(crypto::CAP_HOST_PROOF);
                        }
                        tx.send(WsMsg::Binary(hello.into()))?;
                    }
                    "error" => {
                        let m = v.get("msg").and_then(|x| x.as_str()).unwrap_or("?");
                        break Err(anyhow::Error::new(match m {
                            "offline" => Fail::Offline,
                            "busy" => Fail::Busy,
                            other => Fail::Protocol(other.to_string()),
                        }));
                    }
                    "peer_gone" => break Err(anyhow::Error::new(Fail::PeerEnded)),
                    _ => {}
                }
            }
            WsMsg::Binary(b) => {
                let data = b.as_ref();
                if data.is_empty() {
                    continue;
                }
                match data[0] {
                    crypto::TAG_HELLO_ACK => {
                        if data.len() < 49 {
                            break Err(anyhow::Error::new(Fail::Protocol(
                                "short hello ack".to_string(),
                            )));
                        }
                        let mut host_pub = [0u8; 32];
                        host_pub.copy_from_slice(&data[1..33]);
                        let mut salt = [0u8; 16];
                        salt.copy_from_slice(&data[33..49]);

                        let key = crypto::session_key(&kp.secret, &host_pub, &salt);
                        cipher = Some(Arc::new(Mutex::new(Cipher::new(&key, false))));
                        session_key = Some(key);
                        let code = crypto::session_code(&key);
                        *shared.session_code.lock().unwrap() = code.clone();

                        match auth {
                            Auth::Password(password) => {
                                let pw_key = crypto::password_key(password, &salt);
                                let proof =
                                    crypto::auth_proof(&pw_key, &kp.public, &host_pub, &salt);
                                expected_host_proof = Some(crypto::host_proof(
                                    &pw_key, &kp.public, &host_pub, &salt, &key,
                                ));
                                let mut out = Vec::with_capacity(33);
                                out.push(crypto::TAG_PROOF);
                                out.extend_from_slice(&proof);
                                tx.send(WsMsg::Binary(out.into()))?;
                            }
                            Auth::Ask => {
                                // tell them who is knocking, then wait
                                let me = shared.device_name.lock().unwrap().clone();
                                let mut out = Vec::with_capacity(1 + me.len());
                                out.push(crypto::TAG_ASK);
                                out.extend_from_slice(me.as_bytes());
                                tx.send(WsMsg::Binary(out.into()))?;
                                shared.set_viewer_status(format!(
                                    "Warte auf Bestaetigung ... (Code {})",
                                    code
                                ));
                            }
                        }
                    }
                    crypto::TAG_OK => {
                        let c = match cipher.as_ref() {
                            Some(c) => c.clone(),
                            None => {
                                break Err(anyhow::Error::new(Fail::Protocol(
                                    "ok before handshake".to_string(),
                                )))
                            }
                        };
                        if let Some(expected) = expected_host_proof.as_ref() {
                            match crypto::check_ok_frame(data, expected, crypto::REQUIRE_HOST_PROOF) {
                                crypto::HostCheck::Verified => {}
                                crypto::HostCheck::Legacy => crate::capture::log_line(
                                    "host did not send a counter-proof (old host version): host not verified",
                                ),
                                crypto::HostCheck::Refused => {
                                    crate::capture::log_line("host counter-proof wrong: refusing session");
                                    break Err(anyhow::Error::new(Fail::HostUnverified));
                                }
                            }
                        }
                        shared.connected.store(true, Ordering::Relaxed);
                        shared.set_viewer_status("Verbunden");

                        // input pipeline: GUI -> encrypt -> relay
                        let (in_tx, mut in_rx) = mpsc::unbounded_channel::<Msg>();
                        *shared.input_tx.lock().unwrap() = Some(in_tx);
                        let c2 = c.clone();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            while let Some(m) = in_rx.recv().await {
                                let sealed = { c2.lock().unwrap().seal(&encode(&m)) };
                                if tx2.send(WsMsg::Binary(sealed.into())).is_err() {
                                    break;
                                }
                            }
                        });

                        // file transfer engine for this session
                        {
                            let sh = shared.clone();
                            let send_msg: Arc<dyn Fn(Msg) + Send + Sync> =
                                Arc::new(move |m: Msg| sh.send_input(m));
                            shared.xfers.lock().unwrap().clear();
                            *shared.xfer.lock().unwrap() =
                                Some(crate::xfer::Xfer::new(shared.clone(), send_msg));
                        }

                        // tell the host what we can decode. Without this the
                        // host keeps sending JPEG tiles, which is exactly what
                        // older builds expect.
                        shared.send_input(Msg::Caps {
                            h264: cfg!(windows) && std::env::var("FV_NOH264").is_err(),
                        });

                        // direct UDP path: video only, everything else stays
                        // on the relay. The decoder pipeline is started here
                        // so both transports can feed the same worker.
                        let pipe = video.get_or_insert_with(|| VideoPipe::start(shared.clone()));
                        if std::env::var("FV_NOP2P").is_err() {
                            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                            match crate::p2p::P2p::new(session_key.unwrap_or([0u8; 32]), false, stop) {
                                Ok(p) => {
                                    let gate = pipe.gate();
                                    let gate_loss = gate.clone();
                                    let sh_v = shared.clone();
                                    let sh_loss = shared.clone();
                                    let sh_off = shared.clone();
                                    let sh_state = shared.clone();
                                    let p_off = p.clone();
                                    tokio::spawn(async move {
                                        // STUN shares this socket with the
                                        // receive loop, so ask first and only
                                        // then start listening - otherwise the
                                        // loop swallows the STUN answer and we
                                        // never learn our public address.
                                        let addrs = p_off.candidates().await;
                                        crate::capture::log_line(&format!(
                                            "p2p eigene Kandidaten: {:?}",
                                            addrs
                                        ));
                                        sh_off.send_input(Msg::P2pOffer { token: 0, addrs });
                                        let p_punch = p_off.clone();
                                        tokio::spawn(p_punch.punch_loop(move |direct, _rtt| {
                                            sh_state.direct.store(direct, Ordering::Relaxed);
                                        }));
                                        tokio::spawn(p_off.recv_loop(
                                            move |plain| {
                                                sh_v.video_bytes.fetch_add(
                                                    plain.len() as u64,
                                                    Ordering::Relaxed,
                                                );
                                                if let Some(Msg::Video {
                                                    width,
                                                    height,
                                                    key,
                                                    data,
                                                }) = decode(&plain)
                                                {
                                                    sh_v.udp_frames
                                                        .fetch_add(1, Ordering::Relaxed);
                                                    gate.push(&sh_v, width, height, key, data);
                                                }
                                            },
                                            move || gate_loss.lost(&sh_loss),
                                        ));
                                    });
                                    p2p = Some(p);
                                }
                                Err(e) => {
                                    crate::capture::log_line(&format!("p2p aus: {}", e))
                                }
                            }
                        }

                        // voice link: microphone out, speaker in
                        {
                            let sh = shared.clone();
                            let vsend: Arc<dyn Fn(Msg) + Send + Sync> =
                                Arc::new(move |m: Msg| sh.send_input(m));
                            voice = Some(crate::audio::Voice::start(
                                shared.voice.clone(),
                                vsend,
                            ));
                        }
                        // clipboard sync (own thread, clipboard handles are not Send)
                        let sh_clip = shared.clone();
                        std::thread::spawn(move || clipboard_worker(sh_clip));

                        // latency probe
                        let c3 = c.clone();
                        let tx3 = tx.clone();
                        ping_task = Some(tokio::spawn(async move {
                            loop {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                let ts = started.elapsed().as_millis() as u64;
                                let sealed =
                                    { c3.lock().unwrap().seal(&encode(&Msg::Ping { ts })) };
                                if tx3.send(WsMsg::Binary(sealed.into())).is_err() {
                                    break;
                                }
                            }
                        }));
                    }
                    crypto::TAG_FAIL => {
                        break Err(anyhow::Error::new(match auth {
                            Auth::Password(_) => Fail::WrongPassword,
                            Auth::Ask => Fail::Declined,
                        }))
                    }
                    crypto::TAG_DATA => {
                        let plain = {
                            let c = match cipher.as_ref() {
                                Some(c) => c,
                                None => continue,
                            };
                            match c.lock().unwrap().open(data) {
                                Some(p) => p,
                                None => continue,
                            }
                        };
                        match decode(&plain) {
                            Some(Msg::ScreenInfo { width, height }) => {
                                *shared.remote_size.lock().unwrap() = (width, height);
                            }
                            Some(Msg::Resolutions { list }) => {
                                *shared.remote_resolutions.lock().unwrap() = list;
                            }
                            Some(Msg::Monitors { active, list }) => {
                                *shared.monitors.lock().unwrap() = list;
                                shared.active_monitor.store(active, Ordering::Relaxed);
                            }
                            Some(Msg::Audio { seq, data }) => {
                                if let Some(v) = voice.as_ref() {
                                    v.feed(seq, &data);
                                }
                            }                            Some(Msg::Cursor { x, y, visible }) => {
                                *shared.remote_cursor.lock().unwrap() = (x, y, visible);
                            }
                            Some(Msg::Clipboard { text }) => {
                                shared.clip_from_host.fetch_add(1, Ordering::Relaxed);
                                *shared.clip_in.lock().unwrap() = Some(text);
                            }
                            Some(Msg::Frame {
                                width,
                                height,
                                jpeg,
                            }) => {
                                win_frames += 1;
                                win_bytes += jpeg.len();
                                if let Ok(img) = image::load_from_memory_with_format(
                                    &jpeg,
                                    image::ImageFormat::Jpeg,
                                ) {
                                    canvas.set_full(width, height, img.to_rgba8().into_raw());
                                    canvas.publish(shared);
                                }
                            }
                            Some(Msg::Video {
                                width,
                                height,
                                key,
                                data,
                            }) => {
                                win_bytes += data.len();
                                let pipe = video
                                    .get_or_insert_with(|| VideoPipe::start(shared.clone()));
                                pipe.push(shared, width, height, key, data);
                            }
                            Some(Msg::Tiles {
                                width,
                                height,
                                tiles,
                            }) => {
                                if !canvas.matches(width, height) {
                                    // no keyframe for this size yet - ignore until one arrives
                                    continue;
                                }
                                let mut painted = false;
                                for t in tiles {
                                    win_bytes += t.jpeg.len();
                                    if let Ok(img) = image::load_from_memory_with_format(
                                        &t.jpeg,
                                        image::ImageFormat::Jpeg,
                                    ) {
                                        let rgb = img.to_rgb8();
                                        if blit_rgb_to_rgba(
                                            &mut canvas.rgba,
                                            width,
                                            height,
                                            t.x,
                                            t.y,
                                            rgb.as_raw(),
                                            rgb.width(),
                                            rgb.height(),
                                        ) {
                                            painted = true;
                                        }
                                    }
                                }
                                if painted {
                                    win_frames += 1;
                                    canvas.seq += 1;
                                    canvas.publish(shared);
                                }
                            }
                            Some(m) if crate::xfer::is_file_msg(&m) => {
                                if let Some(x) = shared.xfer.lock().unwrap().as_mut() {
                                    x.on_msg(m);
                                }
                            }
                            Some(Msg::P2pOffer { addrs, .. }) => {
                                if let Some(p) = p2p.as_ref() {
                                    p.set_remote(&addrs);
                                }
                            }
                            Some(Msg::Pong { ts }) => {
                                let now = started.elapsed().as_millis() as u64;
                                let rtt = now.saturating_sub(ts) as f32;
                                shared.stats.lock().unwrap().latency_ms = rtt;
                            }
                            _ => {}
                        }

                        if win_start.elapsed() >= Duration::from_secs(1) {
                            let secs = win_start.elapsed().as_secs_f32();
                            win_frames += shared.video_frames.swap(0, Ordering::Relaxed);
                            win_bytes += shared.video_bytes.swap(0, Ordering::Relaxed) as usize;
                            {
                                let mut st = shared.stats.lock().unwrap();
                                st.fps = win_frames as f32 / secs;
                                st.kbps = (win_bytes as f32 * 8.0 / 1000.0) / secs;
                            }
                            win_frames = 0;
                            win_bytes = 0;
                            win_start = Instant::now();
                        }
                    }
                    _ => {}
                }
            }
            WsMsg::Close(_) => break Ok(()),
            _ => {}
        }
    };

    if let Some(p) = ping_task {
        p.abort();
    }
    let _ = tx.send(WsMsg::text(net::json_bye()));
    writer.abort();
    res
}

/// Hands encoded video to a decoder thread and keeps an eye on the backlog.
///
/// Dropping H.264 frames breaks the reference chain, so we never drop single
/// pictures - if the decoder falls too far behind we throw the whole queue
/// away and ask the host for a fresh keyframe instead.
struct VideoPipe {
    tx: std::sync::mpsc::Sender<(u32, u32, bool, Vec<u8>)>,
    pending: Arc<std::sync::atomic::AtomicI64>,
    /// true while we wait for the keyframe that restarts the stream. Shared,
    /// because the direct UDP path feeds the very same decoder from another
    /// task and must not slip broken pictures past the gate.
    resyncing: Arc<AtomicBool>,
}

/// More than this many undecoded pictures means the viewer cannot keep up.
const MAX_BACKLOG: i64 = 24;

impl VideoPipe {
    /// Everything the direct UDP path needs to feed the same decoder through
    /// the same gate.
    fn gate(&self) -> VideoGate {
        VideoGate {
            tx: self.tx.clone(),
            pending: self.pending.clone(),
            resyncing: self.resyncing.clone(),
        }
    }

    fn start(shared: Arc<Shared>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(u32, u32, bool, Vec<u8>)>();
        let pending = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let p2 = pending.clone();
        std::thread::spawn(move || video_worker(shared, rx, p2));
        Self {
            tx,
            pending,
            resyncing: Arc::new(AtomicBool::new(false)),
        }
    }

    fn push(&mut self, shared: &Arc<Shared>, w: u32, h: u32, key: bool, data: Vec<u8>) {
        self.gate().push(shared, w, h, key, data);
    }
}

/// The gate in front of the decoder thread. Both transports (relay WebSocket
/// and direct UDP) hand their pictures in here, so a resync started by one of
/// them really does stop the other one from pushing broken frames.
#[derive(Clone)]
struct VideoGate {
    tx: std::sync::mpsc::Sender<(u32, u32, bool, Vec<u8>)>,
    pending: Arc<std::sync::atomic::AtomicI64>,
    resyncing: Arc<AtomicBool>,
}

impl VideoGate {
    /// Something ate a picture - stop decoding until a keyframe arrives.
    fn lost(&self, shared: &Arc<Shared>) {
        if !self.resyncing.swap(true, Ordering::Relaxed) {
            shared.send_input(Msg::NeedKeyframe);
        }
    }

    fn push(&self, shared: &Arc<Shared>, w: u32, h: u32, key: bool, data: Vec<u8>) {
        if self.resyncing.load(Ordering::Relaxed) && !key {
            // still waiting for the restart - a P frame on a broken reference
            // chain would paint garbage, so it goes in the bin
            return;
        }
        self.resyncing.store(false, Ordering::Relaxed);
        if self.pending.load(Ordering::Relaxed) > MAX_BACKLOG && !key {
            self.lost(shared);
            return;
        }
        self.pending.fetch_add(1, Ordering::Relaxed);
        if self.tx.send((w, h, key, data)).is_err() {
            self.pending.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

fn video_worker(
    shared: Arc<Shared>,
    rx: std::sync::mpsc::Receiver<(u32, u32, bool, Vec<u8>)>,
    pending: Arc<std::sync::atomic::AtomicI64>,
) {
    let mut dec: Option<crate::h264::Decoder> = None;
    let mut rgba: Vec<u8> = Vec::new();

    while let Ok((w, h, key, data)) = rx.recv() {
        pending.fetch_sub(1, Ordering::Relaxed);
        let stale = dec.as_ref().map(|d| d.size() != (w, h)).unwrap_or(true);
        if stale {
            if !key {
                shared.send_input(Msg::NeedKeyframe);
                continue;
            }
            match crate::h264::Decoder::new(w, h) {
                Ok(d) => {
                    shared.set_viewer_status(format!(
                        "Verbunden - H.264 {}x{} ({})",
                        w,
                        h,
                        d.name()
                    ));
                    dec = Some(d);
                }
                Err(e) => {
                    shared.set_viewer_status(format!(
                        "H.264 nicht verfuegbar ({}) - nutze JPEG",
                        e
                    ));
                    shared.send_input(Msg::Caps { h264: false });
                    shared.send_input(Msg::NeedKeyframe);
                    continue;
                }
            }
        }
        let d = match dec.as_mut() {
            Some(d) => d,
            None => continue,
        };
        match d.decode(&data, &mut rgba) {
            Ok(Some((dw, dh))) => {
                let seq = shared.next_frame_seq();
                *shared.frame.lock().unwrap() = Some(FrameData {
                    width: dw,
                    height: dh,
                    rgba: std::mem::take(&mut rgba),
                    seq,
                });
                shared.video_frames.fetch_add(1, Ordering::Relaxed);
            }
            Ok(None) => {}
            Err(e) => {
                crate::capture::log_line(&format!("h264 decode: {}", e));
                dec = None;
                shared.send_input(Msg::NeedKeyframe);
            }
        }
    }
}
#[cfg(test)]
mod reconnect_tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        let secs: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn wrong_password_and_refusal_never_retry() {
        for f in [Fail::WrongPassword, Fail::Declined, Fail::HostUnverified, Fail::Protocol("x".into())] {
            assert!(next_retry(&f, true, false, 0).is_none(), "{:?}", f);
        }
    }

    #[test]
    fn dropped_live_session_retries_with_backoff_then_gives_up() {
        let f = Fail::Network("reset".into());
        assert_eq!(next_retry(&f, true, false, 0), Some(Duration::from_secs(1)));
        assert_eq!(next_retry(&f, true, false, 2), Some(Duration::from_secs(4)));
        assert!(next_retry(&f, true, false, MAX_RECONNECTS).is_none());
        // the same holds for the other transient reasons
        for f in [Fail::Offline, Fail::Busy, Fail::PeerEnded, Fail::Closed] {
            assert!(next_retry(&f, true, false, 0).is_some(), "{:?}", f);
        }
    }

    #[test]
    fn first_connect_failure_is_shown_not_retried() {
        assert!(next_retry(&Fail::Offline, false, false, 0).is_none());
        assert!(next_retry(&Fail::Network("x".into()), false, false, 0).is_none());
    }

    #[test]
    fn ask_mode_never_reconnects_silently() {
        assert!(next_retry(&Fail::Network("x".into()), true, true, 0).is_none());
    }

    #[test]
    fn plain_errors_count_as_network_and_typed_ones_survive() {
        let plain = anyhow!("connection refused");
        assert!(matches!(Fail::from_error(&plain), Fail::Network(_)));
        let typed = anyhow::Error::new(Fail::Busy);
        assert_eq!(Fail::from_error(&typed), Fail::Busy);
    }

    #[test]
    fn every_failure_has_a_plain_sentence() {
        for f in [
            Fail::Offline,
            Fail::Busy,
            Fail::WrongPassword,
            Fail::Declined,
            Fail::HostUnverified,
            Fail::PeerEnded,
            Fail::Closed,
            Fail::Network("x".into()),
            Fail::Protocol("x".into()),
        ] {
            let t = f.user_text();
            assert!(!t.starts_with("viewer.err."), "missing text for {:?}", f);
            assert!(!t.contains("Fehler:"));
        }
    }

    #[test]
    fn reconnect_text_fills_all_placeholders() {
        let t = reconnect_text(4, 3);
        assert!(!t.contains("{}"), "{}", t);
    }
}
