//! Viewer side: connect to a remote FreeViewer host through the relay.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

use crate::crypto::{self, Cipher};
use crate::net;
use crate::proto::{decode, encode, Msg};
use crate::shared::{FrameData, Shared};

pub async fn run_viewer(shared: Arc<Shared>, id: String, password: String) {
    shared.connecting.store(true, Ordering::Relaxed);
    shared.set_viewer_status(format!("Verbinde mit {} ...", id));

    let result = viewer_once(&shared, &id, &password).await;

    shared.connected.store(false, Ordering::Relaxed);
    shared.connecting.store(false, Ordering::Relaxed);
    *shared.input_tx.lock().unwrap() = None;
    *shared.frame.lock().unwrap() = None;

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
    let mut seq: u64 = 0;
    let mut ping_task: Option<tokio::task::JoinHandle<()>> = None;

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

                        // latency probe
                        let c3 = c.clone();
                        let tx3 = tx.clone();
                        ping_task = Some(tokio::spawn(async move {
                            loop {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                let ts = started.elapsed().as_millis() as u64;
                                let sealed = { c3.lock().unwrap().seal(&encode(&Msg::Ping { ts })) };
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
                            Some(Msg::Frame {
                                width,
                                height,
                                jpeg,
                            }) => {
                                if let Ok(img) = image::load_from_memory_with_format(
                                    &jpeg,
                                    image::ImageFormat::Jpeg,
                                ) {
                                    let rgba = img.to_rgba8();
                                    seq += 1;
                                    *shared.frame.lock().unwrap() = Some(FrameData {
                                        width,
                                        height,
                                        rgba: rgba.into_raw(),
                                        seq,
                                    });
                                }
                            }
                            Some(Msg::Pong { ts }) => {
                                let now = started.elapsed().as_millis() as u64;
                                let rtt = now.saturating_sub(ts) as f32;
                                shared.stats.lock().unwrap().latency_ms = rtt;
                            }
                            _ => {}
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
