//! Host side: share this machine's screen and execute remote input.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use crate::crypto::{self, Cipher};
use crate::net;
use crate::proto::{self, decode, encode, Msg};
use crate::shared::Shared;

const MAX_WIDTH: u32 = 1600;
const TARGET_FPS: u64 = 15;
const JPEG_QUALITY: u8 = 62;

pub async fn run_host(shared: Arc<Shared>, secret: String) {
    loop {
        shared.set_host_status("Verbinde mit Relay...");
        match host_once(&shared, &secret).await {
            Ok(()) => shared.set_host_status("Relay-Verbindung beendet"),
            Err(e) => shared.set_host_status(format!("Relay-Fehler: {}", e)),
        }
        *shared.my_id.lock().unwrap() = String::new();
        *shared.host_peer.lock().unwrap() = "Keine aktive Sitzung".to_string();
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn host_once(shared: &Arc<Shared>, secret: &str) -> Result<()> {
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

    tx.send(WsMsg::text(net::json_register(secret)))?;

    let mut sess: Option<Session> = None;

    while let Some(item) = stream.next().await {
        let msg = item?;
        match msg {
            WsMsg::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(t.as_str()).unwrap_or(serde_json::Value::Null);
                match net::msg_type(&v) {
                    "registered" => {
                        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        *shared.my_id.lock().unwrap() = id;
                        shared.set_host_status("Bereit - warte auf Verbindungen");
                    }
                    "incoming" => {
                        if let Some(s) = sess.take() {
                            s.stop();
                        }
                        sess = Some(Session::new());
                        *shared.host_peer.lock().unwrap() =
                            "Eingehende Verbindung - Authentifizierung...".to_string();
                    }
                    "peer_gone" => {
                        if let Some(s) = sess.take() {
                            s.stop();
                        }
                        *shared.host_peer.lock().unwrap() = "Keine aktive Sitzung".to_string();
                    }
                    "replaced" => return Err(anyhow!("an anderer Stelle neu registriert")),
                    "error" => {
                        let m = v.get("msg").and_then(|x| x.as_str()).unwrap_or("?");
                        shared.set_host_status(format!("Relay meldet: {}", m));
                    }
                    _ => {}
                }
            }
            WsMsg::Binary(b) => {
                let mut drop_session = false;
                if let Some(s) = sess.as_mut() {
                    if let Err(e) = s.on_binary(b.as_ref(), &tx, shared) {
                        *shared.host_peer.lock().unwrap() = format!("Sitzung beendet: {}", e);
                        drop_session = true;
                    }
                }
                if drop_session {
                    if let Some(s) = sess.take() {
                        s.stop();
                    }
                }
            }
            WsMsg::Close(_) => break,
            _ => {}
        }
    }

    if let Some(s) = sess.take() {
        s.stop();
    }
    writer.abort();
    Ok(())
}

enum Stage {
    WaitHello,
    WaitProof {
        secret: x25519_dalek::StaticSecret,
        client_pub: [u8; 32],
        host_pub: [u8; 32],
        salt: [u8; 16],
    },
    Live,
}

struct Session {
    stage: Stage,
    cipher: Option<Arc<Mutex<Cipher>>>,
    stop: Arc<AtomicBool>,
    input_tx: Option<std::sync::mpsc::Sender<Msg>>,
}

impl Session {
    fn new() -> Self {
        Self {
            stage: Stage::WaitHello,
            cipher: None,
            stop: Arc::new(AtomicBool::new(false)),
            input_tx: None,
        }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn on_binary(
        &mut self,
        data: &[u8],
        tx: &mpsc::UnboundedSender<WsMsg>,
        shared: &Arc<Shared>,
    ) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        match (&self.stage, data[0]) {
            (Stage::WaitHello, crypto::TAG_HELLO) => {
                if data.len() < 33 {
                    return Err(anyhow!("hello zu kurz"));
                }
                let mut client_pub = [0u8; 32];
                client_pub.copy_from_slice(&data[1..33]);
                let kp = crypto::keypair();
                let mut salt = [0u8; 16];
                salt.copy_from_slice(&crypto::random_bytes(16));

                let mut out = Vec::with_capacity(49);
                out.push(crypto::TAG_HELLO_ACK);
                out.extend_from_slice(&kp.public);
                out.extend_from_slice(&salt);
                tx.send(WsMsg::Binary(out.into()))?;

                self.stage = Stage::WaitProof {
                    secret: kp.secret,
                    client_pub,
                    host_pub: kp.public,
                    salt,
                };
                Ok(())
            }
            (
                Stage::WaitProof {
                    secret,
                    client_pub,
                    host_pub,
                    salt,
                },
                crypto::TAG_PROOF,
            ) => {
                let password = shared.password.lock().unwrap().clone();
                let pw_key = crypto::password_key(&password, salt);
                let expected = crypto::auth_proof(&pw_key, client_pub, host_pub, salt);
                if !crypto::proof_matches(&expected, &data[1..]) {
                    tx.send(WsMsg::Binary(vec![crypto::TAG_FAIL].into()))?;
                    return Err(anyhow!("falsches Passwort"));
                }
                let key = crypto::session_key(secret, client_pub, salt);
                let cipher = Arc::new(Mutex::new(Cipher::new(&key, true)));
                tx.send(WsMsg::Binary(vec![crypto::TAG_OK].into()))?;

                // outgoing pipeline: plain proto bytes -> sealed -> websocket
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
                let c2 = cipher.clone();
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    while let Some(plain) = out_rx.recv().await {
                        let sealed = { c2.lock().unwrap().seal(&plain) };
                        if tx2.send(WsMsg::Binary(sealed.into())).is_err() {
                            break;
                        }
                    }
                });

                let screen = Arc::new(Mutex::new((1920u32, 1080u32)));
                let (in_tx, in_rx) = std::sync::mpsc::channel::<Msg>();

                // input worker (enigo must live on one dedicated thread)
                let screen_in = screen.clone();
                std::thread::spawn(move || input_loop(in_rx, screen_in));

                // capture worker
                let stop = self.stop.clone();
                let shared2 = shared.clone();
                let screen_cap = screen.clone();
                std::thread::spawn(move || capture_loop(stop, out_tx, screen_cap, shared2));

                self.cipher = Some(cipher);
                self.input_tx = Some(in_tx);
                self.stage = Stage::Live;
                *shared.host_peer.lock().unwrap() = "Verbunden - Bildschirm wird geteilt".to_string();
                Ok(())
            }
            (Stage::Live, crypto::TAG_DATA) => {
                let plain = {
                    let c = self.cipher.as_ref().ok_or_else(|| anyhow!("kein Schluessel"))?;
                    let mut c = c.lock().unwrap();
                    c.open(data).ok_or_else(|| anyhow!("Entschluesselung fehlgeschlagen"))?
                };
                if let Some(m) = decode(&plain) {
                    match m {
                        Msg::Ping { ts } => {
                            if let Some(c) = self.cipher.as_ref() {
                                let sealed = c.lock().unwrap().seal(&encode(&Msg::Pong { ts }));
                                tx.send(WsMsg::Binary(sealed.into()))?;
                            }
                        }
                        other => {
                            if let Some(itx) = self.input_tx.as_ref() {
                                let _ = itx.send(other);
                            }
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn fast_hash(data: &[u8]) -> u64 {
    // FNV-1a over every 97th byte: cheap "did anything change" probe
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0usize;
    while i < data.len() {
        h ^= data[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 97;
    }
    h ^= data.len() as u64;
    h
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
    }
    out
}

fn capture_loop(
    stop: Arc<AtomicBool>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    screen: Arc<Mutex<(u32, u32)>>,
    shared: Arc<Shared>,
) {
    let monitors = match xcap::Monitor::all() {
        Ok(m) => m,
        Err(e) => {
            shared.set_host_status(format!("Bildschirm nicht lesbar: {}", e));
            return;
        }
    };
    if monitors.is_empty() {
        shared.set_host_status("Kein Bildschirm gefunden");
        return;
    }
    let mon = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(&monitors[0]);

    let sw = mon.width().unwrap_or(1920);
    let sh = mon.height().unwrap_or(1080);
    *screen.lock().unwrap() = (sw, sh);
    let _ = out.send(encode(&Msg::ScreenInfo {
        width: sw,
        height: sh,
    }));

    let frame_budget = Duration::from_millis(1000 / TARGET_FPS);
    let mut last_hash = 0u64;
    let mut frames = 0u32;
    let mut bytes = 0usize;
    let mut window = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        match mon.capture_image() {
            Ok(img) => {
                let (w, h) = (img.width(), img.height());
                let (rgba, ow, oh) = if w > MAX_WIDTH {
                    let nh = ((h as f64) * (MAX_WIDTH as f64) / (w as f64)).round() as u32;
                    let small = image::imageops::thumbnail(&img, MAX_WIDTH, nh.max(1));
                    (small.into_raw(), MAX_WIDTH, nh.max(1))
                } else {
                    (img.into_raw(), w, h)
                };
                let hash = fast_hash(&rgba);
                if hash != last_hash {
                    last_hash = hash;
                    let rgb = rgba_to_rgb(&rgba);
                    let mut buf: Vec<u8> = Vec::with_capacity(rgb.len() / 8);
                    {
                        let mut enc =
                            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
                        if enc
                            .encode(&rgb, ow, oh, image::ExtendedColorType::Rgb8)
                            .is_err()
                        {
                            continue;
                        }
                    }
                    frames += 1;
                    bytes += buf.len();
                    if out
                        .send(encode(&Msg::Frame {
                            width: ow,
                            height: oh,
                            jpeg: buf,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(250));
            }
        }

        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f32();
            let mut st = shared.stats.lock().unwrap();
            st.fps = frames as f32 / secs;
            st.kbps = (bytes as f32 * 8.0 / 1000.0) / secs;
            drop(st);
            frames = 0;
            bytes = 0;
            window = Instant::now();
        }

        let dt = t0.elapsed();
        if dt < frame_budget {
            std::thread::sleep(frame_budget - dt);
        }
    }
}

fn named_key(code: u32) -> Option<enigo::Key> {
    use enigo::Key;
    let k = match code {
        proto::KEY_BACKSPACE => Key::Backspace,
        proto::KEY_ENTER => Key::Return,
        proto::KEY_TAB => Key::Tab,
        proto::KEY_ESCAPE => Key::Escape,
        proto::KEY_LEFT => Key::LeftArrow,
        proto::KEY_RIGHT => Key::RightArrow,
        proto::KEY_UP => Key::UpArrow,
        proto::KEY_DOWN => Key::DownArrow,
        proto::KEY_DELETE => Key::Delete,
        proto::KEY_HOME => Key::Home,
        proto::KEY_END => Key::End,
        proto::KEY_PAGEUP => Key::PageUp,
        proto::KEY_PAGEDOWN => Key::PageDown,
        proto::KEY_INSERT => Key::Insert,
        proto::KEY_SPACE => Key::Space,
        proto::KEY_SHIFT => Key::Shift,
        proto::KEY_CTRL => Key::Control,
        proto::KEY_ALT => Key::Alt,
        proto::KEY_META => Key::Meta,
        30 => Key::F1,
        31 => Key::F2,
        32 => Key::F3,
        33 => Key::F4,
        34 => Key::F5,
        35 => Key::F6,
        36 => Key::F7,
        37 => Key::F8,
        38 => Key::F9,
        39 => Key::F10,
        40 => Key::F11,
        41 => Key::F12,
        _ => return None,
    };
    Some(k)
}

fn input_loop(rx: std::sync::mpsc::Receiver<Msg>, screen: Arc<Mutex<(u32, u32)>>) {
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};

    #[cfg(target_os = "windows")]
    {
        let _ = enigo::set_dpi_awareness();
    }

    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(_) => return,
    };

    while let Ok(m) = rx.recv() {
        let (sw, sh) = *screen.lock().unwrap();
        match m {
            Msg::MouseMove { x, y } => {
                let px = (x as i64 * sw as i64 / 10000) as i32;
                let py = (y as i64 * sh as i64 / 10000) as i32;
                let _ = enigo.move_mouse(px, py, Coordinate::Abs);
            }
            Msg::MouseButton { button, down } => {
                let b = match button {
                    1 => Button::Right,
                    2 => Button::Middle,
                    _ => Button::Left,
                };
                let d = if down { Direction::Press } else { Direction::Release };
                let _ = enigo.button(b, d);
            }
            Msg::Wheel { lines } => {
                let _ = enigo.scroll(-lines, Axis::Vertical);
            }
            Msg::Key { code, named, down } => {
                let d = if down { Direction::Press } else { Direction::Release };
                let key = if named {
                    named_key(code)
                } else {
                    char::from_u32(code).map(enigo::Key::Unicode)
                };
                if let Some(k) = key {
                    let _ = enigo.key(k, d);
                }
            }
            _ => {}
        }
    }
}
