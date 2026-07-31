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
    /// Target bitrate of the H.264 encoder in bit/s. Only an upper bound -
    /// the encoder runs in variable bitrate mode, a still desktop costs
    /// almost nothing.
    pub bitrate: u32,
}

pub const ADMIN: Profile = Profile {
    max_w: 1920,
    full_q: 68,
    tile_q: 78,
    fps: 30,
    bitrate: 8_000_000,
};

pub const GAME: Profile = Profile {
    max_w: 1280,
    full_q: 48,
    tile_q: 55,
    fps: 60,
    bitrate: 15_000_000,
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

    let my_name = shared.device_name.lock().unwrap().clone();
    tx.send(WsMsg::text(net::json_register(secret, &my_name)))?;

    let mut sess: Option<Session> = None;

    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    loop {
        let msg = tokio::select! {
            item = stream.next() => match item {
                Some(i) => i?,
                None => break,
            },
            _ = ticker.tick() => {
                let mut drop_session = false;
                if let Some(s) = sess.as_mut() {
                    if let Err(e) = s.poll_confirm(&tx, shared) {
                        *shared.host_peer.lock().unwrap() = format!("Sitzung beendet: {}", e);
                        drop_session = true;
                    }
                }
                if drop_session {
                    if let Some(s) = sess.take() {
                        s.stop();
                    }
                }
                continue;
            }
        };
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
    /// The viewer asked to be let in without a password; we are waiting for
    /// the person sitting in front of this machine to decide.
    WaitConfirm {
        key: [u8; 32],
        since: Instant,
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
    /// The viewer told us it can decode H.264.
    h264: Arc<AtomicBool>,
    /// Direct UDP path of this session (video only).
    p2p: Option<Arc<crate::p2p::P2p>>,
    /// Speech both ways while the session runs.
    voice: Option<crate::audio::Voice>,
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
            h264: Arc::new(AtomicBool::new(false)),
            p2p: None,
            voice: None,
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
                // Es gilt das Sitzungspasswort UND jedes feste Passwort aus
                // den Einstellungen. Der Beweis verraet nicht, welches gemeint
                // war, also bleibt nur der Reihe nach ausprobieren. Argon2
                // kostet Zeit, darum ist die Liste bei 10 Eintraegen gedeckelt.
                let session_pw = shared.password.lock().unwrap().clone();
                let mut ok = false;
                for cand in std::iter::once(session_pw)
                    .chain(crate::pwlist::candidates())
                    .take(crate::pwlist::MAX + 1)
                {
                    if cand.is_empty() {
                        continue;
                    }
                    let pw_key = crypto::password_key(&cand, salt);
                    let expected = crypto::auth_proof(&pw_key, client_pub, host_pub, salt);
                    if crypto::proof_matches(&expected, &data[1..]) {
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    tx.send(WsMsg::Binary(vec![crypto::TAG_FAIL].into()))?;
                    return Err(anyhow!("falsches Passwort"));
                }
                let key = crypto::session_key(secret, client_pub, salt);
                self.go_live(key, tx, shared)
            }
            (
                Stage::WaitProof {
                    secret,
                    client_pub,
                    salt,
                    ..
                },
                crypto::TAG_ASK,
            ) => {
                // No password - the viewer asks to be let in and the person
                // sitting in front of this machine decides. The key exchange
                // already happened, so both sides can show the same four
                // digit code and see they are talking to each other.
                let from: String = String::from_utf8_lossy(&data[1..])
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(48)
                    .collect();
                let key = crypto::session_key(secret, client_pub, salt);
                let code = crypto::session_code(&key);
                shared.knock_answer.store(0, Ordering::Relaxed);
                *shared.knock.lock().unwrap() = Some(crate::shared::Knock {
                    from: if from.trim().is_empty() {
                        "Unbekanntes Geraet".to_string()
                    } else {
                        from.trim().to_string()
                    },
                    code,
                    at: Instant::now(),
                });
                *shared.host_peer.lock().unwrap() =
                    "Verbindungsanfrage - bitte bestaetigen".to_string();
                // make sure the question is actually visible
                crate::tray::show_window();
                crate::tray::balloon(
                    "FreeViewer",
                    "Jemand moechte sich mit diesem Computer verbinden.",
                );
                self.stage = Stage::WaitConfirm {
                    key,
                    since: Instant::now(),
                };
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
                        Msg::P2pOffer { addrs, .. } => {
                            if let Some(p) = self.p2p.as_ref() {
                                p.set_remote(&addrs);
                            }
                        }
                        Msg::Caps { h264 } => {
                            let before = self.h264.swap(h264, Ordering::Relaxed);
                            if before != h264 {
                                self.force_key.store(true, Ordering::Relaxed);
                            }
                        }
                        Msg::SetMode { mode } => {
                            self.mode.store(mode, Ordering::Relaxed);
                            *shared.host_peer.lock().unwrap() = if mode == proto::MODE_GAME {
                                "Verbunden - Spielmodus (relative Maus)".to_string()
                            } else {
                                "Verbunden - Fernwartung".to_string()
                            };
                        }
                        Msg::Audio { seq, data } => {
                            if let Some(v) = self.voice.as_ref() {
                                v.feed(seq, &data);
                            }
                        }                        other => {
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

    /// Everything that has to happen once a session is allowed. Identical for
    /// the password path and for the "please confirm" path, so both really do
    /// end up with the same encryption and the same workers.
    fn go_live(
        &mut self,
        key: [u8; 32],
        tx: &mpsc::UnboundedSender<WsMsg>,
        shared: &Arc<Shared>,
    ) -> Result<()> {
        *shared.session_code.lock().unwrap() = crypto::session_code(&key);
                let cipher = Arc::new(Mutex::new(Cipher::new(&key, true)));
                tx.send(WsMsg::Binary(vec![crypto::TAG_OK].into()))?;

                // direct UDP path for the video stream (best effort)
                let p2p = match crate::p2p::P2p::new(key, true, self.stop.clone()) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        capture::log_line(&format!("p2p aus: {}", e));
                        None
                    }
                };

                // outgoing pipeline: plain proto bytes -> sealed -> websocket.
                // Video frames take the direct path whenever one is up.
                let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
                let c2 = cipher.clone();
                let tx2 = tx.clone();
                let p2p_send = p2p.clone();
                tokio::spawn(async move {
                    while let Some(plain) = out_rx.recv().await {
                        if proto::is_video(&plain) {
                            if let Some(p) = p2p_send.as_ref() {
                                if p.send_msg(&plain).await {
                                    continue;
                                }
                            }
                        }
                        let sealed = { c2.lock().unwrap().seal(&plain) };
                        if tx2.send(WsMsg::Binary(sealed.into())).is_err() {
                            break;
                        }
                    }
                });

                if let Some(p) = p2p.clone() {
                    let offer_tx = out_tx.clone();
                    let sh = shared.clone();
                    let p_punch = p.clone();
                    let p_recv = p.clone();
                    let sh_state = shared.clone();
                    tokio::spawn(async move {
                        let addrs = p.candidates().await;
                        capture::log_line(&format!("p2p eigene Kandidaten: {:?}", addrs));
                        let _ = offer_tx.send(encode(&Msg::P2pOffer { token: 0, addrs }));
                        let sh2 = sh.clone();
                        tokio::spawn(p_punch.punch_loop(move |direct, rtt| {
                            sh2.direct.store(direct, Ordering::Relaxed);
                            *sh2.host_peer.lock().unwrap() = if direct {
                                format!("Verbunden - direkter Weg ({} ms)", rtt)
                            } else {
                                "Verbunden - ueber Relay".to_string()
                            };
                        }));
                        // the host only receives punches on this socket
                        tokio::spawn(p_recv.recv_loop(
                            move |_msg| {
                                let _ = &sh_state;
                            },
                            || {},
                        ));
                    });
                }
                self.p2p = p2p;
                // voice link: speech in both directions, same encrypted channel
                {
                    let vtx = out_tx.clone();
                    let vsend: std::sync::Arc<dyn Fn(Msg) + Send + Sync> =
                        std::sync::Arc::new(move |m: Msg| {
                            let _ = vtx.send(encode(&m));
                        });
                    self.voice = Some(crate::audio::Voice::start(shared.voice.clone(), vsend));
                }

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
                let h264 = self.h264.clone();
                std::thread::spawn(move || {
                    capture_loop(stop, out_tx, screen_cap, shared2, mode, mon, fkey, h264)
                });

                self.cipher = Some(cipher);
                self.input_tx = Some(in_tx);
                self.stage = Stage::Live;
                *shared.host_peer.lock().unwrap() =
                    "Verbunden - Bildschirm wird geteilt".to_string();
                Ok(())
    }

    /// While a viewer waits at the door: has the user decided yet? Called
    /// from the host loop a few times per second.
    fn poll_confirm(
        &mut self,
        tx: &mpsc::UnboundedSender<WsMsg>,
        shared: &Arc<Shared>,
    ) -> Result<()> {
        let (key, since) = match &self.stage {
            Stage::WaitConfirm { key, since } => (*key, *since),
            _ => return Ok(()),
        };
        // FV_AUTOCONFIRM lets scripted tests answer the question. Only a
        // process started with that environment variable does it, so nobody
        // can turn it on from the outside.
        let auto = std::env::var("FV_AUTOCONFIRM").is_ok();
        let answer = if auto {
            1
        } else {
            shared.knock_answer.load(Ordering::Relaxed)
        };
        let too_late = since.elapsed() > Duration::from_secs(60);
        if answer == 1 {
            shared.knock_answer.store(0, Ordering::Relaxed);
            *shared.knock.lock().unwrap() = None;
            return self.go_live(key, tx, shared);
        }
        if answer == 2 || too_late {
            shared.knock_answer.store(0, Ordering::Relaxed);
            *shared.knock.lock().unwrap() = None;
            tx.send(WsMsg::Binary(vec![crypto::TAG_FAIL].into()))?;
            return Err(anyhow!(if too_late {
                "Anfrage wurde nicht beantwortet"
            } else {
                "Anfrage abgelehnt"
            }));
        }
        Ok(())
    }
}

/// Streaming resolution for a captured screen (downscale wide screens).
fn target_size(w: u32, h: u32, max_w: u32) -> (u32, u32) {
    let (tw, th) = if w > max_w {
        let nh = ((h as f64) * (max_w as f64) / (w as f64)).round() as u32;
        (max_w, nh.max(1))
    } else {
        (w.max(1), h.max(1))
    };
    // H.264 works on 4:2:0 chroma, so both edges have to be even. Rounding
    // down by one pixel is invisible and keeps the JPEG path happy as well.
    ((tw & !1).max(2), (th & !1).max(2))
}

/// Scaled RGB of the current frame: hardware scaler if the backend has one,
/// otherwise the CPU box filter.
fn frame_rgb(cap: &mut Box<dyn capture::Backend>, dw: u32, dh: u32) -> Vec<u8> {
    if let Some(b) = cap.scaled(dw, dh, false) {
        return b.to_vec();
    }
    let (buf, w, h, bgra) = cap.frame();
    encoder::scale_to_rgb_ex(buf, w, h, dw, dh, bgra)
}

/// NV12 of the current frame for the video encoder. The GPU does scaling and
/// colour conversion in one step; without a hardware scaler we scale on the
/// CPU and convert afterwards.
fn frame_nv12(cap: &mut Box<dyn capture::Backend>, dw: u32, dh: u32, out: &mut Vec<u8>) -> bool {
    if let Some(b) = cap.scaled(dw, dh, true) {
        out.clear();
        out.extend_from_slice(b);
        return true;
    }
    let (buf, w, h, bgra) = cap.frame();
    let rgb = encoder::scale_to_rgb_ex(buf, w, h, dw, dh, bgra);
    crate::h264::rgb_to_nv12(&rgb, dw, dh, out);
    false
}

/// `freeviewer --gputest [n]` - is the GPU scaler faster than the CPU one and
/// does it produce the same picture?
pub fn gpu_selftest(rounds: u32) -> String {
    let mut out = String::new();
    let mut cap = match capture::open(true) {
        Some(c) => c,
        None => return "kein Capture-Backend\n".to_string(),
    };
    let (sw, sh) = cap.size();
    let (dw, dh) = target_size(sw, sh, ADMIN.max_w);
    out.push_str(&format!(
        "backend {} {}x{} -> {}x{}\n",
        cap.name(),
        sw,
        sh,
        dw,
        dh
    ));

    let (mut t_gpu, mut t_cpu) = (0u128, 0u128);
    let (mut n_gpu, mut n_cpu) = (0u32, 0u32);
    let mut diff_sum = 0f64;
    let mut diff_max = 0u32;
    let mut samples = 0u64;

    for _ in 0..rounds {
        match cap.next(200) {
            Next::Frame => {}
            Next::Unchanged => continue,
            Next::Lost => break,
        }
        let t = Instant::now();
        let gpu = cap.scaled(dw, dh, false).map(|b| b.to_vec());
        let gpu_us = t.elapsed().as_micros();
        let t = Instant::now();
        let (buf, w, h, bgra) = cap.frame();
        let cpu = encoder::scale_to_rgb_ex(buf, w, h, dw, dh, bgra);
        let cpu_us = t.elapsed().as_micros();

        t_cpu += cpu_us;
        n_cpu += 1;
        if let Some(g) = gpu {
            t_gpu += gpu_us;
            n_gpu += 1;
            // Once the hardware scaler runs, the full resolution CPU buffer is
            // not refreshed any more (that is the whole point), so only the
            // first frame can be compared pixel by pixel.
            if g.len() != cpu.len() {
                out.push_str("WARN: Groessen unterschiedlich\n");
            }
            if n_gpu == 1 && g.len() == cpu.len() {
                // compare a subsample so the check itself stays cheap
                let step = (g.len() / 30_000).max(1);
                let mut i = 0;
                while i < g.len() {
                    let d = (g[i] as i32 - cpu[i] as i32).unsigned_abs();
                    diff_sum += d as f64;
                    diff_max = diff_max.max(d);
                    samples += 1;
                    i += step;
                }
            }
        }
    }

    if n_gpu == 0 {
        out.push_str("GPU-Scaler nicht verfuegbar (Fallback CPU)\n");
    } else {
        out.push_str(&format!(
            "GPU {:.2} ms/Frame ({} Frames) | CPU {:.2} ms/Frame ({} Frames) => {:.1}x\n",
            t_gpu as f32 / n_gpu as f32 / 1000.0,
            n_gpu,
            t_cpu as f32 / n_cpu.max(1) as f32 / 1000.0,
            n_cpu,
            (t_cpu as f32 / n_cpu.max(1) as f32) / (t_gpu as f32 / n_gpu as f32).max(1.0)
        ));
        out.push_str(&format!(
            "Bildabweichung zur CPU-Skalierung: Mittel {:.2}, Max {} (von 255) ueber {} Stichproben\n",
            diff_sum / samples.max(1) as f64,
            diff_max,
            samples
        ));
    }
    out
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
            let t1 = Instant::now();
            let rgb = frame_rgb(&mut cap, dw, dh);
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

/// `freeviewer --videotest [rounds]` - the honest comparison on the real
/// desktop: same captured frames through the JPEG tile encoder and through
/// hardware H.264, side by side.
pub fn video_selftest(rounds: u32) -> String {
    let mut out = String::new();
    for (name, prof) in [("Fernwartung", ADMIN), ("Spiel", GAME)] {
        let mut cap = match capture::open(true) {
            Some(c) => c,
            None => return "kein Capture-Backend\n".to_string(),
        };
        let (sw, sh) = cap.size();
        let (dw, dh) = target_size(sw, sh, prof.max_w);
        out.push_str(&format!(
            "== {} == {} {}x{} -> {}x{} @{} fps\n",
            name,
            cap.name(),
            sw,
            sh,
            dw,
            dh,
            prof.fps
        ));

        let mut enc = match crate::h264::Encoder::new(dw, dh, prof.fps as u32, prof.bitrate) {
            Ok(e) => e,
            Err(e) => {
                out.push_str(&format!("kein H.264 Encoder: {}\n", e));
                continue;
            }
        };
        out.push_str(&format!(
            "encoder {} ({})\n",
            enc.name(),
            if enc.hardware() { "GPU" } else { "CPU" }
        ));
        let mut delta = Delta::new();
        delta.set_quality(prof.full_q, prof.tile_q);

        let mut nv12 = Vec::new();
        let (mut t_cap, mut t_scale, mut t_nv, mut t_264, mut t_jpg) =
            (0u128, 0u128, 0u128, 0u128, 0u128);
        let (mut b_264, mut b_jpg) = (0usize, 0usize);
        let (mut n_264, mut n_jpg, mut frames, mut idle) = (0u32, 0u32, 0u32, 0u32);
        let mut keys = 0u32;

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
            t_cap += t.elapsed().as_micros();
            let t = Instant::now();
            let rgb = frame_rgb(&mut cap, dw, dh);
            t_scale += t.elapsed().as_micros();
            frames += 1;

            let t = Instant::now();
            let gpu_nv12 = frame_nv12(&mut cap, dw, dh, &mut nv12);
            t_nv += t.elapsed().as_micros();
            if frames == 1 {
                // verify the colour conversion the GPU did against our own
                let mut cpu = Vec::new();
                crate::h264::rgb_to_nv12(&rgb, dw, dh, &mut cpu);
                if gpu_nv12 && cpu.len() == nv12.len() {
                    let n = cpu.len();
                    let ysize = (dw * dh) as usize;
                    let mut sum = 0f64;
                    let mut max = 0u32;
                    for i in 0..n {
                        let d = (cpu[i] as i32 - nv12[i] as i32).unsigned_abs();
                        sum += d as f64;
                        max = max.max(d);
                    }
                    out.push_str(&format!(
                        "NV12 kommt von der GPU | Abweichung zur CPU-Konvertierung: Mittel {:.2}, Max {} (Y-Ebene {} Byte)\n",
                        sum / n as f64,
                        max,
                        ysize
                    ));
                } else if !gpu_nv12 {
                    out.push_str("NV12 wird auf der CPU erzeugt (kein GPU-Scaler)\n");
                }
            }
            let t = Instant::now();
            match enc.encode(&nv12) {
                Ok(cs) => {
                    t_264 += t.elapsed().as_micros();
                    for c in cs {
                        b_264 += c.data.len();
                        n_264 += 1;
                        if c.key {
                            keys += 1;
                        }
                    }
                }
                Err(e) => {
                    out.push_str(&format!("encode: {}\n", e));
                    break;
                }
            }

            let t = Instant::now();
            let res = delta.encode(&rgb, dw, dh);
            t_jpg += t.elapsed().as_micros();
            if res.msg.is_some() {
                b_jpg += res.bytes;
                n_jpg += 1;
            }
        }

        let f = frames.max(1) as f32;
        out.push_str(&format!(
            "{} Frames, {} unveraendert | capture {:.2} ms | scale {:.2} ms\n",
            frames,
            idle,
            t_cap as f32 / f / 1000.0,
            t_scale as f32 / f / 1000.0
        ));
        out.push_str(&format!(
            "H.264 : RGB->NV12 {:.2} ms + encode {:.2} ms = {:.2} ms/Frame | {} Einheiten ({} Key), {:.1} KB/Frame, {:.2} Mbit/s bei {} fps\n",
            t_nv as f32 / f / 1000.0,
            t_264 as f32 / f / 1000.0,
            (t_nv + t_264) as f32 / f / 1000.0,
            n_264,
            keys,
            b_264 as f32 / n_264.max(1) as f32 / 1024.0,
            b_264 as f32 * 8.0 / 1_000_000.0 / (n_264.max(1) as f32 / prof.fps as f32),
            prof.fps
        ));
        out.push_str(&format!(
            "JPEG  : encode {:.2} ms/Frame | {} gesendet, {:.1} KB/Frame, {:.2} Mbit/s bei {} fps\n",
            t_jpg as f32 / f / 1000.0,
            n_jpg,
            b_jpg as f32 / n_jpg.max(1) as f32 / 1024.0,
            b_jpg as f32 * 8.0 / 1_000_000.0 / (n_jpg.max(1) as f32 / prof.fps as f32),
            prof.fps
        ));
        if b_264 > 0 && b_jpg > 0 {
            out.push_str(&format!(
                "=> H.264 braucht {:.2}x der JPEG-Datenmenge und {:.2}x der Encode-Zeit\n",
                b_264 as f32 / b_jpg as f32,
                (t_nv + t_264) as f32 / t_jpg.max(1) as f32
            ));
        }
    }
    out
}

/// Capture thread + encode thread. The grabber keeps pulling frames while the
/// encoder is still busy with the previous one, so a session runs at roughly
/// max(capture, encode) instead of capture + encode.
/// Which codec carries the picture of this session.
enum Codec {
    Jpeg(Delta),
    H264 {
        enc: crate::h264::Encoder,
        nv12: Vec<u8>,
        mode: u8,
    },
}

fn capture_loop(
    stop: Arc<AtomicBool>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    screen: Arc<Mutex<ScreenRect>>,
    shared: Arc<Shared>,
    mode: Arc<AtomicU8>,
    monitor: Arc<AtomicU8>,
    force_key: Arc<AtomicBool>,
    want_h264: Arc<AtomicBool>,
) {
    // FV_NODELTA / FV_NOSKIP force a full frame every time (benchmarks)
    let force_full = std::env::var("FV_NODELTA").is_ok() || std::env::var("FV_NOSKIP").is_ok();
    let no_dxgi = std::env::var("FV_NODXGI").is_ok();

    // (pixels, width, height, true = the buffer is already NV12)
    let (raw_tx, raw_rx) = sync_channel::<(Vec<u8>, u32, u32, bool)>(1);
    let stop_grab = stop.clone();
    let shared_grab = shared.clone();
    let out_grab = out.clone();
    let mode_grab = mode.clone();
    let screen_grab = screen.clone();
    let mon_grab = monitor.clone();
    let key_grab = force_key.clone();
    let h264_grab = want_h264.clone();

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

        capture::log_line(&format!(
            "Sitzung startet: backend {} {}x{} bei {},{} (Bildschirm {})",
            cap.name(),
            sw,
            sh,
            ox,
            oy,
            cur_mon
        ));

        let mut last_cursor = (i32::MIN, i32::MIN, false);
        let mut fails = 0u32;
        // A picture has to arrive even when nothing moves. Desktop
        // Duplication only reports *changes*, and the lock screen is
        // perfectly still - without this the viewer would stare at a black
        // window until somebody wiggles the mouse.
        let mut sent_any = false;
        let mut last_push = Instant::now();
        let mut tried_fallback = false;
        // first seconds of a session are logged, that is where problems show
        let session_start = Instant::now();
        let mut grabbed = 0u64;
        let mut pushed = 0u64;
        let mut trace = Instant::now();
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
                    sent_any = true;
                    grabbed += 1;
                    last_push = Instant::now();
                    let (cw, ch) = cap.size();
                    let (dw, dh) = target_size(cw, ch, prof.max_w);
                    // With H.264 the GPU scales AND converts to NV12 in one
                    // pass, so no RGB frame is ever built on the CPU.
                    let (buf, is_nv12) = if h264_grab.load(Ordering::Relaxed) {
                        let mut b = Vec::new();
                        frame_nv12(&mut cap, dw, dh, &mut b);
                        (b, true)
                    } else {
                        (frame_rgb(&mut cap, dw, dh), false)
                    };
                    // channel full = encoder still busy, drop this frame
                    pushed += raw_tx.try_send((buf, dw, dh, is_nv12)).is_ok() as u64;
                }
                Next::Unchanged => {
                    let quiet = last_push.elapsed();
                    let have_pixels = !cap.frame().0.is_empty();
                    // A viewer that asked for a keyframe is staring at a
                    // broken picture right now - resend immediately instead of
                    // waiting for the next change on a screen that may well be
                    // standing perfectly still (lock screen, reading a page).
                    let asked = key_grab.load(Ordering::Relaxed);
                    let due = if asked {
                        Duration::from_millis(0)
                    } else {
                        Duration::from_millis(if sent_any { 1000 } else { 300 })
                    };
                    if have_pixels && quiet >= due {
                        // repeat the last picture as a keyframe
                        last_push = Instant::now();
                        sent_any = true;
                        key_grab.store(true, Ordering::Relaxed);
                        let (cw, ch) = cap.size();
                        let (dw, dh) = target_size(cw, ch, prof.max_w);
                        let (buf, is_nv12) = if h264_grab.load(Ordering::Relaxed) {
                            let mut b = Vec::new();
                            frame_nv12(&mut cap, dw, dh, &mut b);
                            (b, true)
                        } else {
                            (frame_rgb(&mut cap, dw, dh), false)
                        };
                        pushed += raw_tx.try_send((buf, dw, dh, is_nv12)).is_ok() as u64;
                    } else if !have_pixels
                        && !tried_fallback
                        && quiet > Duration::from_millis(700)
                    {
                        // The duplication API never handed us a single frame
                        // (happens on some secure desktops). The screenshot
                        // backend is slower but always delivers something.
                        tried_fallback = true;
                        capture::log_line(
                            "keine Bilder von der Duplication - wechsle auf den Screenshot-Weg",
                        );
                        if let Some(c) = capture::open_index(false, cur_mon) {
                            cap = c;
                            key_grab.store(true, Ordering::Relaxed);
                        }
                    }
                }
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

            if session_start.elapsed() < Duration::from_secs(12)
                && trace.elapsed() >= Duration::from_secs(1)
            {
                trace = Instant::now();
                capture::log_line(&format!(
                    "grabber: {} Bilder geholt, {} an den Encoder, backend {}",
                    grabbed,
                    pushed,
                    cap.name()
                ));
            }

            let dt = t0.elapsed();
            if dt < budget {
                std::thread::sleep(budget - dt);
            }
        }
    });

    let no_h264 = std::env::var("FV_NOH264").is_ok();
    let encoder_start = Instant::now();
    let mut codec = Codec::Jpeg(Delta::new());
    let mut frames = 0u32;
    let mut bytes = 0usize;
    let mut window = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        let (pixels, dw, dh, is_nv12) = match raw_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(v) => v,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let cur_mode = mode.load(Ordering::Relaxed);
        let prof = profile(cur_mode);
        let want = want_h264.load(Ordering::Relaxed) && !no_h264 && !force_full;

        // (re)build the video encoder when the viewer, the resolution or the
        // profile changed; any failure silently falls back to JPEG tiles
        let fits = match &codec {
            Codec::H264 { enc, mode: m, .. } => enc.size() == (dw, dh) && *m == cur_mode,
            Codec::Jpeg(_) => false,
        };
        if want && !fits {
            match crate::h264::Encoder::new(dw, dh, prof.fps as u32, prof.bitrate) {
                Ok(enc) => {
                    shared.set_host_status(format!(
                        "Video: H.264 {} ({}) {}x{}",
                        enc.name(),
                        if enc.hardware() { "GPU" } else { "CPU" },
                        dw,
                        dh
                    ));
                    codec = Codec::H264 {
                        enc,
                        nv12: Vec::new(),
                        mode: cur_mode,
                    };
                }
                Err(e) => {
                    capture::log_line(&format!("h264 aus, bleibe bei JPEG: {}", e));
                    want_h264.store(false, Ordering::Relaxed);
                    codec = Codec::Jpeg(Delta::new());
                }
            }
        } else if !want && matches!(codec, Codec::H264 { .. }) {
            codec = Codec::Jpeg(Delta::new());
        }

        let key_now = force_key.swap(false, Ordering::Relaxed);
        let mut failed = false;
        match &mut codec {
            Codec::Jpeg(delta) => {
                if is_nv12 {
                    // the grabber was still producing video frames when we
                    // switched back - the next frame arrives as RGB
                    continue;
                }
                let rgb = &pixels;
                delta.set_quality(prof.full_q, prof.tile_q);
                if key_now {
                    delta.reset();
                }
                let res = if force_full {
                    delta.encode_full(rgb, dw, dh)
                } else {
                    delta.encode(rgb, dw, dh)
                };
                if let Some(msg) = res.msg {
                    frames += 1;
                    bytes += res.bytes;
                    if out.send(encode(&msg)).is_err() {
                        break;
                    }
                }
            }
            Codec::H264 { enc, nv12, .. } => {
                if key_now {
                    enc.request_keyframe();
                }
                let frame: &[u8] = if is_nv12 {
                    &pixels
                } else {
                    crate::h264::rgb_to_nv12(&pixels, dw, dh, nv12);
                    nv12
                };
                match enc.encode(frame) {
                    Ok(chunks) => {
                        for c in chunks {
                            frames += 1;
                            bytes += c.data.len();
                            if out
                                .send(encode(&Msg::Video {
                                    width: dw,
                                    height: dh,
                                    key: c.key,
                                    data: c.data,
                                }))
                                .is_err()
                            {
                                failed = true;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        capture::log_line(&format!("h264 encode: {} - zurueck auf JPEG", e));
                        want_h264.store(false, Ordering::Relaxed);
                        codec = Codec::Jpeg(Delta::new());
                    }
                }
            }
        }
        if failed {
            break;
        }

        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f32();
            if encoder_start.elapsed() < Duration::from_secs(12) {
                capture::log_line(&format!(
                    "encoder: {} Pakete, {} kB, codec {}",
                    frames,
                    bytes / 1024,
                    match &codec {
                        Codec::H264 { .. } => "h264",
                        Codec::Jpeg(_) => "jpeg",
                    }
                ));
            }
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
                        if shared.clip_on.load(Ordering::Relaxed) { clip.set(&text); }
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

        if let Some(text) = clip.poll().filter(|_| shared.clip_on.load(Ordering::Relaxed)) {
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
