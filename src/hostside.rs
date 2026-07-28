//! Host side: share this machine's screen and execute remote input.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use crate::crypto::{self, Cipher};
use crate::encoder::{self, Delta};
use crate::net;
use crate::proto::{self, decode, encode, Msg};
use crate::shared::Shared;

const MAX_WIDTH: u32 = 1600;
const TARGET_FPS: u64 = 30;
const JPEG_QUALITY: u8 = encoder::FULL_QUALITY;

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
                        let id = v
                            .get("id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
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
                *shared.host_peer.lock().unwrap() =
                    "Verbunden - Bildschirm wird geteilt".to_string();
                Ok(())
            }
            (Stage::Live, crypto::TAG_DATA) => {
                let plain = {
                    let c = self
                        .cipher
                        .as_ref()
                        .ok_or_else(|| anyhow!("kein Schluessel"))?;
                    let mut c = c.lock().unwrap();
                    c.open(data)
                        .ok_or_else(|| anyhow!("Entschluesselung fehlgeschlagen"))?
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

/// Grabs the primary monitor fresh on every frame. Reusing one cached
/// `Monitor` handle can return stale device contexts on Windows, which looks
/// like a frozen remote screen.
fn grab(mon_idx: usize) -> Option<image::RgbaImage> {
    let mut all = xcap::Monitor::all().ok()?;
    if mon_idx >= all.len() {
        return None;
    }
    let m = all.swap_remove(mon_idx);
    m.capture_image().ok()
}

fn primary_index() -> Option<usize> {
    let monitors = xcap::Monitor::all().ok()?;
    if monitors.is_empty() {
        return None;
    }
    Some(
        monitors
            .iter()
            .position(|m| m.is_primary().unwrap_or(false))
            .unwrap_or(0),
    )
}

/// Streaming resolution for a captured screen (downscale wide screens).
fn target_size(w: u32, h: u32) -> (u32, u32) {
    if w > MAX_WIDTH {
        let nh = ((h as f64) * (MAX_WIDTH as f64) / (w as f64)).round() as u32;
        (MAX_WIDTH, nh.max(1))
    } else {
        (w.max(1), h.max(1))
    }
}

fn selftest_log(line: &str) {
    use std::io::Write;
    let path = crate::ident::config_dir().join("captest.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", line);
        let _ = f.flush();
    }
}

pub fn capture_selftest(rounds: u32) -> String {
    selftest_log("--- captest start ---");
    let t_all = Instant::now();
    let n = xcap::Monitor::all().map(|v| v.len()).unwrap_or(0);
    selftest_log(&format!(
        "Monitor::all -> {} monitors in {} ms",
        n,
        t_all.elapsed().as_millis()
    ));
    let idx = match primary_index() {
        Some(i) => i,
        None => return "kein Monitor gefunden".to_string(),
    };
    let mut out = String::new();
    for i in 0..rounds {
        let t = Instant::now();
        match grab(idx) {
            Some(img) => {
                let t_cap = t.elapsed().as_millis();
                let (w, h) = (img.width(), img.height());
                let (dw, dh) = target_size(w, h);
                let rgba = img.into_raw();
                let t1 = Instant::now();
                let rgb = encoder::scale_to_rgb(&rgba, w, h, dw, dh);
                let t_scale = t1.elapsed().as_millis();
                let t4 = Instant::now();
                let jpeg = encoder::jpeg_rgb(&rgb, dw, dh, JPEG_QUALITY).unwrap_or_default();
                let t_jpeg = t4.elapsed().as_millis();
                out.push_str(&format!(
                    "{}: {}x{} -> {}x{} | capture {} ms | scale+rgb {} ms | jpeg {} ms ({} KB) | total {} ms\n",
                    i, w, h, dw, dh, t_cap, t_scale, t_jpeg, jpeg.len() / 1024,
                    t.elapsed().as_millis()
                ));
            }
            None => out.push_str(&format!("{}: capture failed\n", i)),
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    out
}

/// Benchmarks the old full-frame path against the new delta path, both on the
/// real screen: first serial (pure encoder cost), then through the actual
/// capture/encode pipeline (what the session really achieves).
pub fn delta_selftest(rounds: u32) -> String {
    let idx = match primary_index() {
        Some(i) => i,
        None => return "kein Monitor gefunden".to_string(),
    };
    let mut out = String::new();

    // ---- pass A: full frame every time (v0.2 behaviour) ----
    let mut a_ms = 0u128;
    let mut a_bytes = 0usize;
    let mut a_frames = 0u32;
    for _ in 0..rounds {
        let t = Instant::now();
        if let Some(img) = grab(idx) {
            let (w, h) = (img.width(), img.height());
            let (dw, dh) = target_size(w, h);
            let nh = ((h as f64) * (dw as f64) / (w as f64)).round() as u32;
            let small = image::imageops::thumbnail(&img, dw, nh.max(1));
            let rgba = small.into_raw();
            let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
            for px in rgba.chunks_exact(4) {
                rgb.push(px[0]);
                rgb.push(px[1]);
                rgb.push(px[2]);
            }
            let jpeg = encoder::jpeg_rgb(&rgb, dw, dh, JPEG_QUALITY).unwrap_or_default();
            a_bytes += jpeg.len();
            a_frames += 1;
            a_ms += t.elapsed().as_millis();
        }
    }

    // ---- pass B: new path, serial ----
    let mut delta = Delta::new();
    let mut b_ms = 0u128;
    let mut b_cap = 0u128;
    let mut b_scale = 0u128;
    let mut b_enc = 0u128;
    let mut b_bytes = 0usize;
    let mut b_frames = 0u32;
    let mut b_sent = 0u32;
    let mut b_key = 0u32;
    let mut dirty_sum = 0f32;
    for _ in 0..rounds {
        let t = Instant::now();
        let t0 = Instant::now();
        if let Some(img) = grab(idx) {
            let cap = t0.elapsed().as_millis();
            let (w, h) = (img.width(), img.height());
            let (dw, dh) = target_size(w, h);
            let rgba = img.into_raw();
            let t1 = Instant::now();
            let rgb = encoder::scale_to_rgb(&rgba, w, h, dw, dh);
            let sc = t1.elapsed().as_millis();
            let t2 = Instant::now();
            let res = delta.encode(&rgb, dw, dh);
            let en = t2.elapsed().as_millis();
            b_cap += cap;
            b_scale += sc;
            b_enc += en;
            b_bytes += res.bytes;
            b_frames += 1;
            if res.msg.is_some() {
                b_sent += 1;
                dirty_sum += res.dirty;
            }
            if res.keyframe {
                b_key += 1;
            }
            b_ms += t.elapsed().as_millis();
        }
    }

    // ---- pass C: new path through the real pipeline (capture || encode) ----
    let stop = Arc::new(AtomicBool::new(false));
    let (raw_tx, raw_rx) = sync_channel::<(Vec<u8>, u32, u32)>(1);
    let stop_g = stop.clone();
    let grabber = std::thread::spawn(move || {
        let budget = Duration::from_millis(1000 / TARGET_FPS);
        while !stop_g.load(Ordering::Relaxed) {
            let t = Instant::now();
            if let Some(img) = grab(idx) {
                let (w, h) = (img.width(), img.height());
                if raw_tx.try_send((img.into_raw(), w, h)).is_err() {
                    // encoder busy - drop this frame
                }
            }
            let dt = t.elapsed();
            if dt < budget {
                std::thread::sleep(budget - dt);
            }
        }
    });
    let mut delta_c = Delta::new();
    let mut c_bytes = 0usize;
    let mut c_frames = 0u32;
    let t_c = Instant::now();
    while c_frames < rounds {
        match raw_rx.recv_timeout(Duration::from_millis(1500)) {
            Ok((rgba, w, h)) => {
                let (dw, dh) = target_size(w, h);
                let rgb = encoder::scale_to_rgb(&rgba, w, h, dw, dh);
                let res = delta_c.encode(&rgb, dw, dh);
                c_bytes += res.bytes;
                c_frames += 1;
            }
            Err(RecvTimeoutError::Timeout) => break,
            Err(_) => break,
        }
    }
    let c_secs = t_c.elapsed().as_secs_f32();
    stop.store(true, Ordering::Relaxed);
    let _ = grabber.join();

    let f = |x: u32| if x == 0 { 1 } else { x };
    out.push_str(&format!(
        "A full-frame (v0.2): {} Frames, {} ms/Frame => {:.1} fps, {:.1} KB/Frame\n",
        a_frames,
        a_ms / f(a_frames) as u128,
        1000.0 / (a_ms as f32 / f(a_frames) as f32).max(0.001),
        a_bytes as f32 / f(a_frames) as f32 / 1024.0
    ));
    out.push_str(&format!(
        "B delta seriell:     {} Frames, {} ms/Frame (capture {} | scale {} | encode {}) => {:.1} fps, {:.1} KB/Frame, {} gesendet, {} Keyframes, dirty {:.1}%\n",
        b_frames,
        b_ms / f(b_frames) as u128,
        b_cap / f(b_frames) as u128,
        b_scale / f(b_frames) as u128,
        b_enc / f(b_frames) as u128,
        1000.0 / (b_ms as f32 / f(b_frames) as f32).max(0.001),
        b_bytes as f32 / f(b_frames) as f32 / 1024.0,
        b_sent,
        b_key,
        if b_sent > 0 { dirty_sum / b_sent as f32 * 100.0 } else { 0.0 }
    ));
    out.push_str(&format!(
        "C delta pipeline:    {} Frames in {:.2}s => {:.1} fps, {:.1} KB/Frame\n",
        c_frames,
        c_secs,
        c_frames as f32 / c_secs.max(0.001),
        c_bytes as f32 / f(c_frames) as f32 / 1024.0
    ));
    out
}

/// Capture thread + encode thread. The grabber keeps pulling screenshots while
/// the encoder is still busy with the previous frame, so a session runs at
/// roughly max(capture, encode) instead of capture + encode.
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
    let mon_idx = monitors
        .iter()
        .position(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(0);

    let sw = monitors[mon_idx].width().unwrap_or(1920);
    let sh = monitors[mon_idx].height().unwrap_or(1080);
    *screen.lock().unwrap() = (sw, sh);
    let _ = out.send(encode(&Msg::ScreenInfo {
        width: sw,
        height: sh,
    }));
    drop(monitors);

    // FV_NODELTA / FV_NOSKIP force a full frame every time (benchmarks, debugging)
    let force_full = std::env::var("FV_NODELTA").is_ok() || std::env::var("FV_NOSKIP").is_ok();
    let frame_budget = Duration::from_millis(1000 / TARGET_FPS);

    let (raw_tx, raw_rx) = sync_channel::<(Vec<u8>, u32, u32)>(1);
    let stop_grab = stop.clone();
    let shared_grab = shared.clone();
    let grabber = std::thread::spawn(move || {
        let mut fails = 0u32;
        while !stop_grab.load(Ordering::Relaxed) {
            let t0 = Instant::now();
            match grab(mon_idx) {
                Some(img) => {
                    fails = 0;
                    let (w, h) = (img.width(), img.height());
                    // channel full = encoder still busy, skip this frame
                    let _ = raw_tx.try_send((img.into_raw(), w, h));
                }
                None => {
                    fails += 1;
                    if fails == 5 {
                        shared_grab.set_host_status("Bildschirmaufnahme schlaegt fehl");
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
            let dt = t0.elapsed();
            if dt < frame_budget {
                std::thread::sleep(frame_budget - dt);
            }
        }
    });

    let mut delta = Delta::new();
    let mut frames = 0u32;
    let mut bytes = 0usize;
    let mut window = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let (rgba, w, h) = match raw_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let (dw, dh) = target_size(w, h);
        let rgb = encoder::scale_to_rgb(&rgba, w, h, dw, dh);
        drop(rgba);
        let res = if force_full {
            delta.encode_full(&rgb, dw, dh)
        } else {
            delta.encode(&rgb, dw, dh)
        };
        if let Some(msg) = res.msg {
            frames += 1;
            bytes += res.bytes;
            if out.send(encode(&msg)).is_err() {
                break;
            }
        }

        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f32();
            {
                let mut st = shared.stats.lock().unwrap();
                st.fps = frames as f32 / secs;
                st.kbps = (bytes as f32 * 8.0 / 1000.0) / secs;
            }
            frames = 0;
            bytes = 0;
            window = Instant::now();
        }
    }

    stop.store(true, Ordering::Relaxed);
    let _ = grabber.join();
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
                let d = if down {
                    Direction::Press
                } else {
                    Direction::Release
                };
                let _ = enigo.button(b, d);
            }
            Msg::Wheel { lines } => {
                let _ = enigo.scroll(-lines, Axis::Vertical);
            }
            Msg::Key { code, named, down } => {
                let d = if down {
                    Direction::Press
                } else {
                    Direction::Release
                };
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
