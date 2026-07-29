//! Viewer side: connect to a remote FreeViewer host through the relay.

use std::sync::atomic::Ordering;
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
            clip.set(&text);
        }
        if let Some(text) = clip.poll() {
            shared.send_input(Msg::Clipboard { text });
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

pub async fn run_viewer(shared: Arc<Shared>, id: String, password: String) {
    shared.connecting.store(true, Ordering::Relaxed);
    shared.set_viewer_status(format!("Verbinde mit {} ...", id));

    let result = viewer_once(&shared, &id, &password).await;

    shared.connected.store(false, Ordering::Relaxed);
    shared.connecting.store(false, Ordering::Relaxed);
    if let Some(mut x) = shared.xfer.lock().unwrap().take() {
        x.shutdown();
    }
    *shared.input_tx.lock().unwrap() = None;
    *shared.frame.lock().unwrap() = None;
    crate::vinput::set_active(false);

    match result {
        Ok(()) => shared.set_viewer_status("Sitzung beendet"),
        Err(e) => shared.set_viewer_status(format!("Fehler: {}", e)),
    }
}

async fn viewer_once(shared: &Arc<Shared>, id: &str, password: &str) -> Result<()> {
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
    let started = Instant::now();
    let mut canvas = Canvas::new();
    // rolling stats for the session bar
    let mut win_start = Instant::now();
    let mut win_frames = 0u32;
    let mut win_bytes = 0usize;
    let mut ping_task: Option<tokio::task::JoinHandle<()>> = None;
    // H.264 runs on its own thread: the decoder is a COM object (not Send)
    // and decoding must never stall the socket.
    let mut video: Option<VideoPipe> = None;

    let res: Result<()> = loop {
        let item = match stream.next().await {
            Some(i) => i,
            None => break Ok(()),
        };
        let msg = match item {
            Ok(m) => m,
            Err(e) => break Err(anyhow!(e.to_string())),
        };

        match msg {
            WsMsg::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(t.as_str()).unwrap_or(serde_json::Value::Null);
                match net::msg_type(&v) {
                    "paired" => {
                        shared.set_viewer_status("Gekoppelt - Authentifizierung...");
                        let mut hello = Vec::with_capacity(33);
                        hello.push(crypto::TAG_HELLO);
                        hello.extend_from_slice(&kp.public);
                        tx.send(WsMsg::Binary(hello.into()))?;
                    }
                    "error" => {
                        let m = v.get("msg").and_then(|x| x.as_str()).unwrap_or("?");
                        break Err(anyhow!(match m {
                            "offline" => "ID ist nicht online".to_string(),
                            "busy" => "Host hat bereits eine Sitzung".to_string(),
                            other => other.to_string(),
                        }));
                    }
                    "peer_gone" => break Err(anyhow!("Gegenstelle hat die Sitzung beendet")),
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
                            break Err(anyhow!("ungueltige Antwort vom Host"));
                        }
                        let mut host_pub = [0u8; 32];
                        host_pub.copy_from_slice(&data[1..33]);
                        let mut salt = [0u8; 16];
                        salt.copy_from_slice(&data[33..49]);

                        let pw_key = crypto::password_key(password, &salt);
                        let proof = crypto::auth_proof(&pw_key, &kp.public, &host_pub, &salt);
                        let key = crypto::session_key(&kp.secret, &host_pub, &salt);
                        cipher = Some(Arc::new(Mutex::new(Cipher::new(&key, false))));

                        let mut out = Vec::with_capacity(33);
                        out.push(crypto::TAG_PROOF);
                        out.extend_from_slice(&proof);
                        tx.send(WsMsg::Binary(out.into()))?;
                    }
                    crypto::TAG_OK => {
                        let c = match cipher.as_ref() {
                            Some(c) => c.clone(),
                            None => break Err(anyhow!("Handshake nicht abgeschlossen")),
                        };
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
                    crypto::TAG_FAIL => break Err(anyhow!("Passwort falsch")),
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
                            Some(Msg::Monitors { active, list }) => {
                                *shared.monitors.lock().unwrap() = list;
                                shared.active_monitor.store(active, Ordering::Relaxed);
                            }
                            Some(Msg::Cursor { x, y, visible }) => {
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
    /// true while we wait for the keyframe that restarts the stream
    resyncing: bool,
}

/// More than this many undecoded pictures means the viewer cannot keep up.
const MAX_BACKLOG: i64 = 24;

impl VideoPipe {
    fn start(shared: Arc<Shared>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(u32, u32, bool, Vec<u8>)>();
        let pending = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let p2 = pending.clone();
        std::thread::spawn(move || video_worker(shared, rx, p2));
        Self {
            tx,
            pending,
            resyncing: false,
        }
    }

    fn push(&mut self, shared: &Arc<Shared>, w: u32, h: u32, key: bool, data: Vec<u8>) {
        if self.resyncing && !key {
            return;
        }
        self.resyncing = false;
        if self.pending.load(Ordering::Relaxed) > MAX_BACKLOG && !key {
            self.resyncing = true;
            shared.send_input(Msg::NeedKeyframe);
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