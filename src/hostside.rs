//! Host side: share this machine's screen and execute remote input.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use crate::capture::{self, Next};
use crate::clip::Clip;
use crate::crypto::{self, Cipher};
use crate::encoder::{self, Delta};
use crate::input::{Injector, ScreenRect};
use crate::net;
use crate::proto::{self, decode, encode, Msg};
use crate::shared::Shared;

/// One operating point of the stream. The viewer switches between them at
/// runtime: "Fernwartung" keeps the picture sharp and the mouse absolute,
/// "Spiel" trades sharpness for frame rate and uses raw relative mouse input.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub max_w: u32,
    pub full_q: u8,
    pub tile_q: u8,
    pub fps: u64,
}

pub const ADMIN: Profile = Profile {
    max_w: 1920,
    full_q: 68,
    tile_q: 78,
    fps: 30,
};

pub const GAME: Profile = Profile {
    max_w: 1280,
    full_q: 48,
    tile_q: 55,
    fps: 60,
};

pub fn profile(mode: u8) -> Profile {
    if mode == proto::MODE_GAME {
        GAME
    } else {
        ADMIN
    }
}

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
    mode: Arc<AtomicU8>,
    monitor: Arc<AtomicU8>,
    force_key: Arc<AtomicBool>,
}

/// The screens this machine could share, in protocol form.
pub fn monitor_list(prefer_fast: bool) -> Vec<proto::MonitorInfo> {
    capture::list_monitors(prefer_fast)
        .into_iter()
        .enumerate()
        .map(|(i, m)| proto::MonitorInfo {
            name: if m.name.trim().is_empty() {
                format!("Bildschirm {}", i + 1)
            } else {
                m.name
            },
            w: m.w,
            h: m.h,
            primary: m.primary,
        })
        .collect()
}

impl Session {
    fn new() -> Self {
        Self {
            stage: Stage::WaitHello,
            cipher: None,
            stop: Arc::new(AtomicBool::new(false)),
            input_tx: None,
            mode: Arc::new(AtomicU8::new(proto::MODE_ADMIN)),
            monitor: Arc::new(AtomicU8::new(0)),
            force_key: Arc::new(AtomicBool::new(false)),
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

                let screen = Arc::new(Mutex::new(ScreenRect::default()));
                let (in_tx, in_rx) = std::sync::mpsc::channel::<Msg>();

                // input worker (SendInput must live on one dedicated thread)
                let screen_in = screen.clone();
                let out_in = out_tx.clone();
                let stop_in = self.stop.clone();
                let shared_in = shared.clone();
                std::thread::spawn(move || {
                    input_loop(in_rx, screen_in, out_in, stop_in, shared_in)
                });

                // capture worker
                let stop = self.stop.clone();
                let shared2 = shared.clone();
                let screen_cap = screen.clone();
                let mode = self.mode.clone();
                let mon = self.monitor.clone();
                let fkey = self.force_key.clone();
                std::thread::spawn(move || {
                    capture_loop(stop, out_tx, screen_cap, shared2, mode, mon, fkey)
                });

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
                        Msg::SetMonitor { index } => {
                            self.monitor.store(index, Ordering::Relaxed);
                        }
                        Msg::NeedKeyframe => {
                            self.force_key.store(true, Ordering::Relaxed);
                        }
                        Msg::SetMode { mode } => {
                            self.mode.store(mode, Ordering::Relaxed);
                            *shared.host_peer.lock().unwrap() = if mode == proto::MODE_GAME {
                                "Verbunden - Spielmodus (relative Maus)".to_string()
                            } else {
                                "Verbunden - Fernwartung".to_string()
                            };
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

/// Streaming resolution for a captured screen (downscale wide screens).
fn target_size(w: u32, h: u32, max_w: u32) -> (u32, u32) {
    if w > max_w {
        let nh = ((h as f64) * (max_w as f64) / (w as f64)).round() as u32;
        (max_w, nh.max(1))
    } else {
        (w.max(1), h.max(1))
    }
}

/// `freeviewer --captest` - which backend do we get and how fast is it?
pub fn capture_selftest(rounds: u32) -> String {
    let mut out = String::new();
    out.push_str("== DXGI Desktop Duplication ==\n");
    out.push_str(&capture::bench(rounds, true));
    out.push_str("== xcap (Screenshot-Fallback) ==\n");
    out.push_str(&capture::bench(rounds, false));
    out
}

/// `freeviewer --deltatest [n]` - end to end encoder benchmark on the real
/// screen: capture, scale, delta encode, once per configured profile.
pub fn delta_selftest(rounds: u32) -> String {
    let mut out = String::new();
    for (name, prof) in [("Fernwartung", ADMIN), ("Spiel", GAME)] {
        let mut cap = match capture::open(true) {
            Some(c) => c,
            None => return "kein Capture-Backend\n".to_string(),
        };
        let (sw, sh) = cap.size();
        let (dw, dh) = target_size(sw, sh, prof.max_w);
        let mut delta = Delta::new();
        delta.set_quality(prof.full_q, prof.tile_q);

        let (mut n_cap, mut n_scale, mut n_enc) = (0u128, 0u128, 0u128);
        let mut frames = 0u32;
        let mut idle = 0u32;
        let mut bytes = 0usize;
        let mut sent = 0u32;
        let mut keys = 0u32;
        let t0 = Instant::now();
        for _ in 0..rounds {
            let t = Instant::now();
            match cap.next(200) {
                Next::Frame => {}
                Next::Unchanged => {
                    idle += 1;
                    continue;
                }
                Next::Lost => break,
            }
            n_cap += t.elapsed().as_micros();
            let (buf, w, h, bgra) = cap.frame();
            let t1 = Instant::now();
            let rgb = encoder::scale_to_rgb_ex(buf, w, h, dw, dh, bgra);
            n_scale += t1.elapsed().as_micros();
            let t2 = Instant::now();
            let res = delta.encode(&rgb, dw, dh);
            n_enc += t2.elapsed().as_micros();
            frames += 1;
            bytes += res.bytes;
            if res.msg.is_some() {
                sent += 1;
            }
            if res.keyframe {
                keys += 1;
            }
        }
        let secs = t0.elapsed().as_secs_f32().max(0.001);
        let f = frames.max(1) as f32;
        out.push_str(&format!(
            "{:<12} {} ({}x{} -> {}x{}) | {} Frames, {} unveraendert in {:.2}s\n",
            name,
            cap.name(),
            sw,
            sh,
            dw,
            dh,
            frames,
            idle,
            secs
        ));
        out.push_str(&format!(
            "             capture {:.1} ms | scale {:.1} ms | encode {:.1} ms => {:.1} fps moeglich, {} gesendet ({} Keyframes), {:.1} KB/Frame\n",
            n_cap as f32 / f / 1000.0,
            n_scale as f32 / f / 1000.0,
            n_enc as f32 / f / 1000.0,
            1000.0 / ((n_cap + n_scale + n_enc) as f32 / f / 1000.0).max(0.001),
            sent,
            keys,
            bytes as f32 / sent.max(1) as f32 / 1024.0
        ));
    }
    out
}

/// Capture thread + encode thread. The grabber keeps pulling frames while the
/// encoder is still busy with the previous one, so a session runs at roughly
/// max(capture, encode) instead of capture + encode.
fn capture_loop(
    stop: Arc<AtomicBool>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    screen: Arc<Mutex<ScreenRect>>,
    shared: Arc<Shared>,
    mode: Arc<AtomicU8>,
    monitor: Arc<AtomicU8>,
    force_key: Arc<AtomicBool>,
) {
    // FV_NODELTA / FV_NOSKIP force a full frame every time (benchmarks)
    let force_full = std::env::var("FV_NODELTA").is_ok() || std::env::var("FV_NOSKIP").is_ok();
    let no_dxgi = std::env::var("FV_NODXGI").is_ok();

    let (raw_tx, raw_rx) = sync_channel::<(Vec<u8>, u32, u32)>(1);
    let stop_grab = stop.clone();
    let shared_grab = shared.clone();
    let out_grab = out.clone();
    let mode_grab = mode.clone();
    let screen_grab = screen.clone();
    let mon_grab = monitor.clone();
    let key_grab = force_key.clone();

    let grabber = std::thread::spawn(move || {
        let mut cur_mon = mon_grab.load(Ordering::Relaxed) as usize;
        let mut cap = match capture::open_index(!no_dxgi, cur_mon) {
            Some(c) => c,
            None => {
                shared_grab.set_host_status("Kein Bildschirm gefunden");
                return;
            }
        };
        let (mut sw, mut sh) = cap.size();
        let (mut ox, mut oy) = cap.origin();
        *screen_grab.lock().unwrap() = ScreenRect {
            x: ox,
            y: oy,
            w: sw,
            h: sh,
        };
        let list = monitor_list(!no_dxgi);
        shared_grab.set_host_status(format!(
            "Aufnahme: {} {}x{} (Bildschirm {}/{})",
            cap.name(),
            sw,
            sh,
            cur_mon + 1,
            list.len().max(1)
        ));
        let _ = out_grab.send(encode(&Msg::ScreenInfo {
            width: sw,
            height: sh,
        }));
        let _ = out_grab.send(encode(&Msg::Monitors {
            active: cur_mon as u8,
            list,
        }));

        let mut last_cursor = (i32::MIN, i32::MIN, false);
        let mut fails = 0u32;
        while !stop_grab.load(Ordering::Relaxed) {
            // the viewer can switch screens in the middle of a session
            let want = mon_grab.load(Ordering::Relaxed) as usize;
            if want != cur_mon {
                match capture::open_index(!no_dxgi, want) {
                    Some(c) => {
                        cap = c;
                        cur_mon = want;
                        let list = monitor_list(!no_dxgi);
                        shared_grab.set_host_status(format!(
                            "Aufnahme: {} {}x{} (Bildschirm {}/{})",
                            cap.name(),
                            cap.size().0,
                            cap.size().1,
                            cur_mon + 1,
                            list.len().max(1)
                        ));
                        let _ = out_grab.send(encode(&Msg::Monitors {
                            active: cur_mon as u8,
                            list,
                        }));
                    }
                    None => {
                        mon_grab.store(cur_mon as u8, Ordering::Relaxed);
                    }
                }
            }
            // resolution change / screen switch -> tell the viewer, force a keyframe
            let (nw, nh) = cap.size();
            let (nx, ny) = cap.origin();
            if (nw, nh, nx, ny) != (sw, sh, ox, oy) {
                sw = nw;
                sh = nh;
                ox = nx;
                oy = ny;
                *screen_grab.lock().unwrap() = ScreenRect {
                    x: ox,
                    y: oy,
                    w: sw,
                    h: sh,
                };
                key_grab.store(true, Ordering::Relaxed);
                let _ = out_grab.send(encode(&Msg::ScreenInfo {
                    width: sw,
                    height: sh,
                }));
            }
            let prof = profile(mode_grab.load(Ordering::Relaxed));
            let budget = Duration::from_millis(1000 / prof.fps.max(1));
            let t0 = Instant::now();
            match cap.next(budget.as_millis() as u32) {
                Next::Frame => {
                    fails = 0;
                    let (buf, w, h, bgra) = cap.frame();
                    let (dw, dh) = target_size(w, h, prof.max_w);
                    let rgb = encoder::scale_to_rgb_ex(buf, w, h, dw, dh, bgra);
                    // channel full = encoder still busy, drop this frame
                    let _ = raw_tx.try_send((rgb, dw, dh));
                }
                Next::Unchanged => {}
                Next::Lost => {
                    fails += 1;
                    if fails == 5 {
                        shared_grab.set_host_status("Bildschirmaufnahme schlaegt fehl");
                    }
                    std::thread::sleep(Duration::from_millis(250));
                    if fails > 20 {
                        break;
                    }
                }
            }

            // the duplication API does not paint the cursor into the frame,
            // so the viewer gets its position and draws it itself
            let (cx, cy, vis) = cap.cursor();
            if (cx, cy, vis) != last_cursor && (sw > 0 && sh > 0) {
                last_cursor = (cx, cy, vis);
                let nx = ((cx - ox) as i64 * 10000 / sw.max(1) as i64) as i32;
                let ny = ((cy - oy) as i64 * 10000 / sh.max(1) as i64) as i32;
                let _ = out_grab.send(encode(&Msg::Cursor {
                    x: nx,
                    y: ny,
                    visible: vis,
                }));
            }

            let dt = t0.elapsed();
            if dt < budget {
                std::thread::sleep(budget - dt);
            }
        }
    });

    let mut delta = Delta::new();
    let mut frames = 0u32;
    let mut bytes = 0usize;
    let mut window = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let (rgb, dw, dh) = match raw_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let prof = profile(mode.load(Ordering::Relaxed));
        delta.set_quality(prof.full_q, prof.tile_q);
        if force_key.swap(false, Ordering::Relaxed) {
            delta.reset();
        }
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

fn input_loop(
    rx: std::sync::mpsc::Receiver<Msg>,
    screen: Arc<Mutex<ScreenRect>>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
) {
    let mut inj = Injector::new();
    let mut clip = Clip::new();

    // file transfers of this session use the same encrypted channel
    let send_msg: Arc<dyn Fn(Msg) + Send + Sync> = {
        let out2 = out.clone();
        Arc::new(move |m: Msg| {
            let _ = out2.send(encode(&m));
        })
    };
    shared.xfers.lock().unwrap().clear();
    *shared.xfer.lock().unwrap() = Some(crate::xfer::Xfer::new(shared.clone(), send_msg));

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(m) => {
                let rect = *screen.lock().unwrap();
                match m {
                    Msg::MouseMove { x, y } => inj.mouse_abs(x, y, rect),
                    Msg::MouseDelta { dx, dy } => inj.mouse_delta(dx, dy),
                    Msg::MouseButton { button, down } => inj.button(button, down),
                    Msg::Wheel { lines } => inj.wheel(lines),
                    Msg::KeyVk { vk, ext, down } => inj.key_vk(vk, ext, down),
                    Msg::Key { code, named, down } => {
                        match if named {
                            crate::input::named_to_vk(code)
                        } else {
                            None
                        } {
                            Some((vk, ext)) => inj.key_vk(vk, ext, down),
                            None => inj.key_portable(code, named, down),
                        }
                    }
                    Msg::Special { code } => {
                        let what = inj.special(code);
                        shared.set_host_status(format!("Sondertaste: {}", what));
                    }
                    Msg::Clipboard { text } => {
                        clip.set(&text);
                    }
                    other if crate::xfer::is_file_msg(&other) => {
                        if let Some(x) = shared.xfer.lock().unwrap().as_mut() {
                            x.on_msg(other);
                        }
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if let Some(text) = clip.poll() {
            if out.send(encode(&Msg::Clipboard { text })).is_err() {
                break;
            }
        }
    }

    if let Some(mut x) = shared.xfer.lock().unwrap().take() {
        x.shutdown();
    }
    // never leave keys stuck on the host when a session dies
    inj.release_all();
}
