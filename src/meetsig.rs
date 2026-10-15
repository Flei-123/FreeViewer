//! Native meeting - stage 1: signalling without a browser.
//!
//! Until now the meeting in the client ran in a bare browser window. The
//! browser did EVERYTHING there: negotiation with the media server, audio,
//! video, echo suppression, tiles. For the native version we rebuild this
//! piece by piece ourselves. This module is the first stone: the
//! control line to the media server (WebSocket, JSON) - i.e. everything that is NOT
//! audio and not video: join, participant list, chat, raised hand,
//! mute, waiting room, host interventions.
//!
//! Deliberately only with what FreeViewer already has on board
//! (tokio + tokio-tungstenite + serde_json) - no new third-party code.
//!
//! The protocol is the same one the browser side speaks (server:
//! freemeet-sfu/src/proto.rs). Every message carries `v` (protocol version);
//! unknown messages are quietly discarded instead of killing the session.

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// Protocol version this client speaks.
pub const PROTO_VERSION: u16 = 1;

/// A participant, as the server describes it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Participant {
    pub id: u64,
    pub name: String,
    pub ton_aus: bool,
    pub bild_aus: bool,
    pub hand: bool,
    /// Not empty = this participant allows remote control via FreeViewer.
    pub fvid: String,
}

impl Participant {
    fn aus_json(v: &serde_json::Value) -> Participant {
        Participant {
            id: v.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            name: v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            ton_aus: v
                .get("audio_muted")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            bild_aus: v
                .get("video_muted")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            hand: v.get("hand").and_then(|x| x.as_bool()).unwrap_or(false),
            fvid: v
                .get("fvid")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        }
    }
}

/// What is reported to the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Ereignis {
    /// The server has let us in.
    Willkommen {
        ich: u64,
        raum: String,
        titel: String,
        gastgeber: u64,
        leute: Vec<Participant>,
        server: String,
    },
    Dazu(Participant),
    Weg(u64),
    Chat {
        von: u64,
        text: String,
        privat: bool,
    },
    Tippt {
        peer: u64,
        an: bool,
    },
    Hand {
        peer: u64,
        an: bool,
    },
    Muted {
        peer: u64,
        art: String,
        an: bool,
    },
    /// The host has muted US - the track must really be off.
    ForceMuted {
        art: String,
    },
    Gastgeber(u64),
    Sprecher(Option<u64>),
    Fernsteuerung {
        peer: u64,
        fvid: String,
    },
    /// Someone asks ME for control of my shared screen.
    /// Arrives only at the sharer; the server checks from the tracks
    /// who is really sharing.
    ControlRequested {
        peer: u64,
        name: String,
    },
    /// Answer to MY request. `von` = who decided.
    /// With an approval `fvid` is the FreeViewer number of the sharer -
    /// without it the approval would be inconsequential, because the client would not know
    /// where to connect. Empty for a refusal.
    ControlAnswered {
        von: u64,
        gewaehrt: bool,
        fvid: String,
    },
    /// A release was withdrawn - either by the sharer or
    /// because they stopped sharing.
    ControlGiveBack {
        peer: u64,
    },
    /// Where is the mouse pointer of whoever is currently sharing (0..1 in THEIR
    /// image)? This lets a viewer zoom exactly there.
    Zeiger {
        peer: u64,
        x: f32,
        y: f32,
    },
    /// A viewer asks for a section of MY screen.
    Bereichswunsch {
        peer: u64,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// A sender reports which section it is currently sending.
    Bereich {
        peer: u64,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// We are in the waiting room.
    Wartet {
        titel: String,
        text: String,
    },
    /// Someone is waiting at the door (host only).
    WarteDazu {
        peer: u64,
        name: String,
    },
    WarteWeg {
        peer: u64,
    },
    WaitState {
        an: bool,
    },
    Abgewiesen(String),
    Rausgeworfen(String),
    Beendet(String),
    /// The server says which m-line belongs to whom - without it the
    /// native client does not know whose voice is arriving.
    Spur {
        mid: String,
        peer: u64,
        art: String,
        display: bool,
    },
    /// SDP offer/answer from the server. Only used from stage 2,
    /// but already passed on cleanly now instead of being discarded.
    Sdp {
        art: String,
        sdp: String,
    },
    FriendError {
        code: String,
        text: String,
    },
    Getrennt(String),
}

/// What the UI can tell the session.
#[derive(Debug, Clone)]
enum Befehl {
    Chat(String),
    Tippt(bool),
    Hand(bool),
    Muted { art: String, an: bool },
    Warteraum { aktion: String, peer: Option<u64> },
    Gastgeber { aktion: String, peer: Option<u64> },
    Display(bool),
    /// Fernsteuerung anbieten/zurueckziehen. Leere fvid = zurueckgezogen.
    Fernsteuerung { an: bool, fvid: String },
    /// Raw JSON - there so that stage 2 (offer/answer) does not have to rebuild anything.
    Roh(serde_json::Value),
    Verlassen,
}

/// A running meeting session. Is ended cleanly when dropped.
pub struct Session {
    befehle: UnboundedSender<Befehl>,
    ereignisse: Receiver<Ereignis>,
    /// Last seen state - so the UI does not have to do its own bookkeeping.
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub ich: u64,
    pub raum: String,
    pub titel: String,
    pub gastgeber: u64,
    pub leute: Vec<Participant>,
    pub wartende: Vec<(u64, String)>,
    pub warteraum_an: bool,
    pub im_warteraum: bool,
    pub verbunden: bool,
    pub last_error: String,
}

impl Session {
    /// Neue Ereignisse abholen (blockiert nie).
    pub fn abholen(&self) -> Vec<Ereignis> {
        let mut aus = Vec::new();
        while let Ok(e) = self.ereignisse.try_recv() {
            aus.push(e);
        }
        aus
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|z| z.clone()).unwrap_or_default()
    }

    pub fn chat(&self, text: &str) {
        let _ = self.befehle.send(Befehl::Chat(text.to_string()));
    }
    pub fn tippt(&self, an: bool) {
        let _ = self.befehle.send(Befehl::Tippt(an));
    }
    pub fn hand(&self, an: bool) {
        let _ = self.befehle.send(Befehl::Hand(an));
    }
    /// kind: "audio" or "video"; an = switched off (as in the protocol).
    pub fn is_muted(&self, art: &str, an: bool) {
        let _ = self.befehle.send(Befehl::Muted {
            art: art.to_string(),
            an,
        });
    }
    pub fn display(&self, an: bool) {
        let _ = self.befehle.send(Befehl::Display(an));
    }
    /// Report the own pointer position in the shared image (0..1). Only useful
    /// while sharing is really going on.
    pub fn zeiger(&self, x: f32, y: f32) {
        let _ = self
            .befehle
            .send(Befehl::Roh(serde_json::json!({"t":"cursor","x":x,"y":y})));
    }

    /// As viewer: ask `an_peer` for a section of the screen.
    pub fn bereichswunsch(&self, an_peer: u64, x: f32, y: f32, w: f32, h: f32) {
        let _ = self.befehle.send(Befehl::Roh(
            serde_json::json!({"t":"region-req","to":an_peer,"x":x,"y":y,"w":w,"h":h}),
        ));
    }

    /// Als Sender: bekanntgeben, welchen Ausschnitt ich gerade schicke.
    pub fn bereich(&self, x: f32, y: f32, w: f32, h: f32) {
        let _ = self.befehle.send(Befehl::Roh(
            serde_json::json!({"t":"region-on","x":x,"y":y,"w":w,"h":h}),
        ));
    }

    /// As viewer: ask the sharer for control. The server
    /// finds out for itself who is sharing - we do not have to (and
    /// cannot) claim that.
    pub fn request_control(&self) {
        let _ = self
            .befehle
            .send(Befehl::Roh(serde_json::json!({"t":"control-request"})));
    }

    /// As sharer: answer a request. `gewaehrt=false` refuses.
    /// `fvid` is the OWN FreeViewer number; the server passes it on only
    /// to the approved person, not into the room.
    pub fn answer_control(&self, peer: u64, gewaehrt: bool, fvid: &str) {
        let _ = self.befehle.send(Befehl::Roh(serde_json::json!({
            "t": "control-answer",
            "peer": peer,
            "grant": gewaehrt,
            "fvid": if gewaehrt { fvid } else { "" },
        })));
    }

    /// As sharer: withdraw a granted release. `None` = all.
    pub fn revoke_control(&self, peer: Option<u64>) {
        let mut v = serde_json::json!({"t":"control-revoke"});
        if let (Some(p), Some(o)) = (peer, v.as_object_mut()) {
            o.insert("peer".into(), serde_json::json!(p));
        }
        let _ = self.befehle.send(Befehl::Roh(v));
    }

    /// Offer remote control via FreeViewer (`an`) or withdraw it.
    /// `fvid` is the own FreeViewer number; irrelevant when withdrawing.
    pub fn fernsteuerung(&self, an: bool, fvid: &str) {
        let _ = self.befehle.send(Befehl::Fernsteuerung {
            an,
            fvid: fvid.to_string(),
        });
    }
    /// aktion: "admit" | "admit-all" | "deny" | "on" | "off"
    pub fn warteraum(&self, aktion: &str, peer: Option<u64>) {
        let _ = self.befehle.send(Befehl::Warteraum {
            aktion: aktion.to_string(),
            peer,
        });
    }
    /// aktion: "mute-all" | "mute" | "kick" | "end"
    pub fn gastgeber_aktion(&self, aktion: &str, peer: Option<u64>) {
        let _ = self.befehle.send(Befehl::Gastgeber {
            aktion: aktion.to_string(),
            peer,
        });
    }
    pub fn roh(&self, v: serde_json::Value) {
        let _ = self.befehle.send(Befehl::Roh(v));
    }
    pub fn verlassen(&self) {
        let _ = self.befehle.send(Befehl::Verlassen);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.befehle.send(Befehl::Verlassen);
    }
}

/// Derive the signalling address from the Meet address:
/// https://meet.fleitec.com  ->  wss://meet.fleitec.com/ws
pub fn ws_adresse(basis: &str) -> String {
    let b = basis.trim_end_matches('/');
    if let Some(rest) = b.strip_prefix("https://") {
        format!("wss://{}/ws", rest)
    } else if let Some(rest) = b.strip_prefix("http://") {
        format!("ws://{}/ws", rest)
    } else {
        format!("wss://{}/ws", b)
    }
}

fn umschlag(mut v: serde_json::Value) -> String {
    if let Some(o) = v.as_object_mut() {
        o.insert("v".into(), json!(PROTO_VERSION));
    }
    v.to_string()
}

/// Join a meeting. Keeps running in the background until `verlassen()`
/// is called or the session is dropped.
/// `e2e` tells the server whether WE can do end-to-end. That has to be in the
/// join message already: the server negotiates from it whether the room
/// is encrypted, and rejects whoever cannot keep up.
#[allow(clippy::too_many_arguments)]
pub fn beitreten(
    basis: &str,
    raum: &str,
    pass: &str,
    name: &str,
    fvid: &str,
    e2e: bool,
) -> Result<Session> {
    let url = ws_adresse(basis);
    let (bef_tx, bef_rx) = unbounded_channel::<Befehl>();
    let (ev_tx, ev_rx) = channel::<Ereignis>();
    let state = Arc::new(Mutex::new(State {
        raum: raum.to_string(),
        ..Default::default()
    }));

    let z2 = state.clone();
    let (raum, pass, name, fvid) = (
        raum.to_string(),
        pass.to_string(),
        name.to_string(),
        fvid.to_string(),
    );

    // Own thread with its own small runtime: the UI should not notice
    // anything of the network work, and we do not want to depend on
    // a runtime already running somewhere else.
    std::thread::Builder::new()
        .name("meetsig".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = ev_tx.send(Ereignis::Getrennt(format!("Laufzeit: {}", e)));
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(e) = lauf(&url, &raum, &pass, &name, &fvid, e2e, bef_rx, &ev_tx, &z2).await
                {
                    if let Ok(mut z) = z2.lock() {
                        z.verbunden = false;
                        z.last_error = e.to_string();
                    }
                    let _ = ev_tx.send(Ereignis::Getrennt(e.to_string()));
                } else {
                    if let Ok(mut z) = z2.lock() {
                        z.verbunden = false;
                    }
                    let _ = ev_tx.send(Ereignis::Getrennt(String::new()));
                }
            });
        })
        .map_err(|e| anyhow!("Faden laesst sich nicht starten: {}", e))?;

    Ok(Session {
        befehle: bef_tx,
        ereignisse: ev_rx,
        state,
    })
}

#[allow(clippy::too_many_arguments)]
async fn lauf(
    url: &str,
    raum: &str,
    pass: &str,
    name: &str,
    fvid: &str,
    e2e: bool,
    mut befehle: UnboundedReceiver<Befehl>,
    ev: &Sender<Ereignis>,
    state: &Arc<Mutex<State>>,
) -> Result<()> {
    let (mut ws, _) = connect_now(url).await?;
    if let Ok(mut z) = state.lock() {
        z.verbunden = true;
    }

    // Join. The capabilities are honest: the native client cannot do
    // simulcast yet. It reports end-to-end exactly when a key from
    // the link is present - the server decides from that whether
    // the room runs encrypted. If a hard "false" stood here, the
    // own client would be locked out of its own encrypted meeting.
    let join = umschlag(json!({
        "t": "join",
        "room": raum,
        "name": name,
        "pass": pass,
        "fvid": fvid,
        // e2e_ver: frame format we speak (r61). A server with FM_E2E_MIN_VER
        // refuses E2E joins that do not announce it. Old servers ignore it.
        "caps": { "simulcast": false, "codecs": [], "e2e": e2e,
                  "e2e_ver": if e2e { crate::meete2e::FORMAT_VERSION } else { 0 } }
    }));
    ws.send(Message::Text(join.into())).await?;

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
    ticker.tick().await; // der erste Schlag kommt sofort

    loop {
        tokio::select! {
            // ---- vom Server ------------------------------------------------
            message = ws.next() => {
                let Some(message) = message else { break };
                match message {
                    Ok(Message::Text(t)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                            if verarbeiten(&v, ev, state) {
                                // Session is over (kick/end/rejection)
                                let _ = ws.close(None).await;
                                return Ok(());
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(_) => {}
                    Err(e) => return Err(anyhow!("Verbindung: {}", e)),
                }
            }
            // ---- from the UI --------------------------------------
            b = befehle.recv() => {
                let Some(b) = b else { break };
                let raus = match b {
                    Befehl::Chat(text) => Some(json!({"t":"chat","text":text})),
                    Befehl::Tippt(an) => Some(json!({"t":"typing","on":an})),
                    Befehl::Hand(an) => Some(json!({"t":"hand","on":an})),
                    Befehl::Muted { art, an } => Some(json!({"t":"mute","kind":art,"on":an})),
                    Befehl::Display(an) => Some(json!({"t":"screen","on":an})),
                    Befehl::Fernsteuerung { an, fvid } => {
                        // Only digits out - the number comes from the own
                        // setting, but the server expects it clean.
                        let nr: String = fvid.chars().filter(|c| c.is_ascii_digit()).collect();
                        Some(json!({"t":"remote","on":an,"fvid": if an { nr } else { String::new() }}))
                    }
                    Befehl::Warteraum { aktion, peer } => {
                        Some(json!({"t":"lobby","action":aktion,"peer":peer}))
                    }
                    Befehl::Gastgeber { aktion, peer } => {
                        Some(json!({"t":"host-action","action":aktion,"peer":peer}))
                    }
                    Befehl::Roh(v) => Some(v),
                    Befehl::Verlassen => {
                        let _ = ws.send(Message::Text(umschlag(json!({"t":"leave"})).into())).await;
                        let _ = ws.close(None).await;
                        return Ok(());
                    }
                };
                if let Some(v) = raus {
                    ws.send(Message::Text(umschlag(v).into())).await?;
                }
            }
            // ---- am Leben halten -------------------------------------------
            _ = ticker.tick() => {
                ws.send(Message::Text(umschlag(json!({"t":"ping","nonce":1})).into())).await?;
            }
        }
    }
    Ok(())
}

/// Process a server message. Return `true` = session is over.
fn verarbeiten(
    v: &serde_json::Value,
    ev: &Sender<Ereignis>,
    state: &Arc<Mutex<State>>,
) -> bool {
    let t = v.get("t").and_then(|x| x.as_str()).unwrap_or("");
    let u64f = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let strf = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let boolf = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);

    match t {
        "welcome" => {
            let leute: Vec<Participant> = v
                .get("peers")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().map(Participant::aus_json).collect())
                .unwrap_or_default();
            if let Ok(mut z) = state.lock() {
                z.ich = u64f("you");
                z.raum = strf("room");
                z.titel = strf("titel");
                z.gastgeber = u64f("host");
                z.leute = leute.clone();
                z.im_warteraum = false;
            }
            let _ = ev.send(Ereignis::Willkommen {
                ich: u64f("you"),
                raum: strf("room"),
                titel: strf("titel"),
                gastgeber: u64f("host"),
                leute,
                server: strf("server_version"),
            });
        }
        "peer-join" => {
            let p = v
                .get("peer")
                .map(Participant::aus_json)
                .unwrap_or_default();
            if let Ok(mut z) = state.lock() {
                z.leute.retain(|x| x.id != p.id);
                z.leute.push(p.clone());
            }
            let _ = ev.send(Ereignis::Dazu(p));
        }
        "peer-leave" => {
            let id = u64f("id");
            if let Ok(mut z) = state.lock() {
                z.leute.retain(|x| x.id != id);
            }
            let _ = ev.send(Ereignis::Weg(id));
        }
        "chat" => {
            let _ = ev.send(Ereignis::Chat {
                von: u64f("from"),
                text: strf("text"),
                privat: boolf("private"),
            });
        }
        "typing" => {
            let _ = ev.send(Ereignis::Tippt {
                peer: u64f("peer"),
                an: boolf("on"),
            });
        }
        "cursor" => {
            let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(-1.0) as f32;
            let (x, y) = (f("x"), f("y"));
            // Values outside the image would be a false report - better to say
            // nothing at all than to zoom to an invented spot.
            if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) {
                let _ = ev.send(Ereignis::Zeiger {
                    peer: u64f("peer"),
                    x,
                    y,
                });
            }
        }
        "region-req" | "region-on" => {
            let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(-1.0) as f32;
            let (x, y, w, h) = (f("x"), f("y"), f("w"), f("h"));
            // Unsinnige Rechtecke gaeben einen falschen Ausschnitt - lieber
            // gar nichts melden als etwas Erfundenes.
            let gut = (0.0..=1.0).contains(&x)
                && (0.0..=1.0).contains(&y)
                && w > 0.0
                && h > 0.0
                && x + w <= 1.001
                && y + h <= 1.001;
            if gut {
                let peer = u64f("peer");
                let _ = ev.send(if t == "region-req" {
                    Ereignis::Bereichswunsch { peer, x, y, w, h }
                } else {
                    Ereignis::Bereich { peer, x, y, w, h }
                });
            }
        }
        "hand" => {
            let (peer, an) = (u64f("peer"), boolf("on"));
            if let Ok(mut z) = state.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    p.hand = an;
                }
            }
            let _ = ev.send(Ereignis::Hand { peer, an });
        }
        "mute" => {
            let (peer, art, an) = (u64f("peer"), strf("kind"), boolf("on"));
            if let Ok(mut z) = state.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    if art == "audio" {
                        p.ton_aus = an;
                    } else {
                        p.bild_aus = an;
                    }
                }
            }
            let _ = ev.send(Ereignis::Muted { peer, art, an });
        }
        "force-mute" => {
            let _ = ev.send(Ereignis::ForceMuted { art: strf("kind") });
        }
        "host" => {
            let peer = u64f("peer");
            if let Ok(mut z) = state.lock() {
                z.gastgeber = peer;
            }
            let _ = ev.send(Ereignis::Gastgeber(peer));
        }
        "speaker" => {
            let p = v.get("peer").and_then(|x| x.as_u64());
            let _ = ev.send(Ereignis::Sprecher(p));
        }
        "remote" => {
            let (peer, fvid) = (u64f("peer"), strf("fvid"));
            if let Ok(mut z) = state.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    p.fvid = fvid.clone();
                }
            }
            let _ = ev.send(Ereignis::Fernsteuerung { peer, fvid });
        }
        "control-requested" => {
            let _ = ev.send(Ereignis::ControlRequested {
                peer: u64f("peer"),
                name: strf("name"),
            });
        }
        "control-answered" => {
            let _ = ev.send(Ereignis::ControlAnswered {
                von: u64f("by"),
                gewaehrt: boolf("grant"),
                fvid: strf("fvid"),
            });
        }
        "control-revoked" => {
            let _ = ev.send(Ereignis::ControlGiveBack { peer: u64f("peer") });
        }
        "waiting" => {
            if let Ok(mut z) = state.lock() {
                z.im_warteraum = true;
            }
            let _ = ev.send(Ereignis::Wartet {
                titel: strf("titel"),
                text: strf("msg"),
            });
        }
        "lobby-add" => {
            let (peer, name) = (u64f("peer"), strf("name"));
            if let Ok(mut z) = state.lock() {
                z.wartende.retain(|(id, _)| *id != peer);
                z.wartende.push((peer, name.clone()));
            }
            let _ = ev.send(Ereignis::WarteDazu { peer, name });
        }
        "lobby-del" => {
            let peer = u64f("peer");
            if let Ok(mut z) = state.lock() {
                z.wartende.retain(|(id, _)| *id != peer);
            }
            let _ = ev.send(Ereignis::WarteWeg { peer });
        }
        "lobby-state" => {
            let an = boolf("on");
            if let Ok(mut z) = state.lock() {
                z.warteraum_an = an;
            }
            let _ = ev.send(Ereignis::WaitState { an });
        }
        "denied" => {
            let _ = ev.send(Ereignis::Abgewiesen(strf("msg")));
            return true;
        }
        "kicked" => {
            let _ = ev.send(Ereignis::Rausgeworfen(strf("msg")));
            return true;
        }
        "ended" => {
            let _ = ev.send(Ereignis::Beendet(strf("msg")));
            return true;
        }
        "track" => {
            let _ = ev.send(Ereignis::Spur {
                mid: strf("mid"),
                peer: u64f("peer"),
                art: strf("kind"),
                display: boolf("screen"),
            });
        }
        "offer" | "answer" => {
            let _ = ev.send(Ereignis::Sdp {
                art: t.to_string(),
                sdp: strf("sdp"),
            });
        }
        "error" => {
            if let Ok(mut z) = state.lock() {
                z.last_error = format!("{}: {}", strf("code"), strf("msg"));
            }
            let _ = ev.send(Ereignis::FriendError {
                code: strf("code"),
                text: strf("msg"),
            });
        }
        // pong, quality, transport, screen-off: irrelevant for stage 1.
        _ => {}
    }
    false
}


/// Establish the connection - to EVERY address the name resolves to.
///
/// Why not simply connect_async? On the home network meet.fleitec.com points to
/// TWO addresses: the public one and an internal one (split DNS). The internal
/// one does not answer port 443, though. Whoever tries only the first address
/// hangs for 20 seconds in the timeout error 10060 - exactly that
/// happened on FLEI-ONE. So: try all addresses, public ones
/// first, 4 seconds of patience each.
async fn connect_now(
    url: &str,
) -> Result<(
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
)> {
    let ohne = url
        .strip_prefix("wss://")
        .or_else(|| url.strip_prefix("ws://"))
        .ok_or_else(|| anyhow!("Adresse ohne Schema: {}", url))?;
    let sicher = url.starts_with("wss://");
    let host_teil = ohne.split('/').next().unwrap_or(ohne);
    let (host, port) = match host_teil.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
            (h.to_string(), p.parse::<u16>().unwrap_or(443))
        }
        _ => (host_teil.to_string(), if sicher { 443 } else { 80 }),
    };

    let mut adressen: Vec<std::net::SocketAddr> =
        tokio::net::lookup_host((host.as_str(), port)).await?.collect();
    if adressen.is_empty() {
        return Err(anyhow!("Name {} liefert keine Adresse", host));
    }
    // Public addresses first - internal ones are often dead ends on the home network.
    adressen.sort_by_key(|a| match a.ip() {
        std::net::IpAddr::V4(v) => u8::from(v.is_private() || v.is_loopback()),
        std::net::IpAddr::V6(_) => 1,
    });

    let mut letzter = String::new();
    // Two attempts per address with a generous deadline: measured on Justin's
    // machine, the first connection setup via the own
    // public address (NAT loopback through the Fritzbox) sometimes takes
    // longer than four seconds. With a single short attempt
    // "no answer" sporadically appeared there, although the network was fine.
    for runde in 0..2u8 {
        for a in adressen.iter().copied() {
            let versuch = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::net::TcpStream::connect(a),
            )
            .await;
            let strom = match versuch {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    letzter = format!("{} : {}", a, e);
                    continue;
                }
                Err(_) => {
                    letzter = format!("{} : keine Antwort", a);
                    continue;
                }
            };
            let _ = strom.set_nodelay(true);
            match tokio_tungstenite::client_async_tls(url, strom).await {
                Ok(p) => return Ok(p),
                Err(e) => {
                    letzter = format!("{} : {}", a, e);
                    continue;
                }
            }
        }
        if runde == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        }
    }
    Err(anyhow!("keine Verbindung zu {} ({})", host, letzter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adresse_wird_richtig_gebaut() {
        assert_eq!(
            ws_adresse("https://meet.fleitec.com"),
            "wss://meet.fleitec.com/ws"
        );
        assert_eq!(ws_adresse("https://meet.fleitec.com/"), "wss://meet.fleitec.com/ws");
        assert_eq!(ws_adresse("http://192.168.1.61:7200"), "ws://192.168.1.61:7200/ws");
        assert_eq!(ws_adresse("meet.fleitec.com"), "wss://meet.fleitec.com/ws");
    }

    #[test]
    fn every_message_carries_the_protocol_version() {
        let s = umschlag(json!({"t":"hand","on":true}));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["t"], "hand");
        assert_eq!(v["on"], true);
    }

    #[test]
    fn steuerungsanfrage_kommt_als_ereignis_an() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        let v = json!({"v":1,"t":"control-requested","peer":9,"name":"Ben"});
        assert!(!verarbeiten(&v, &tx, &z));
        match rx.try_recv().unwrap() {
            Ereignis::ControlRequested { peer, name } => {
                assert_eq!(peer, 9);
                assert_eq!(name, "Ben");
            }
            x => panic!("falsches Ereignis: {:?}", x),
        }
    }

    #[test]
    fn zusage_bringt_die_nummer_mit() {
        // Without the number an approval would be inconsequential - the client would not know
        // where to connect. Exactly that must arrive.
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        let v = json!({"v":1,"t":"control-answered","by":4,"grant":true,"fvid":"497628420"});
        assert!(!verarbeiten(&v, &tx, &z));
        match rx.try_recv().unwrap() {
            Ereignis::ControlAnswered { von, gewaehrt, fvid } => {
                assert_eq!(von, 4);
                assert!(gewaehrt);
                assert_eq!(fvid, "497628420");
            }
            x => panic!("falsches Ereignis: {:?}", x),
        }
    }

    #[test]
    fn absage_kommt_ohne_nummer_an() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        let v = json!({"v":1,"t":"control-answered","by":4,"grant":false});
        assert!(!verarbeiten(&v, &tx, &z));
        match rx.try_recv().unwrap() {
            Ereignis::ControlAnswered { gewaehrt, fvid, .. } => {
                assert!(!gewaehrt);
                assert!(fvid.is_empty(), "bei einer Absage darf keine Nummer kommen");
            }
            x => panic!("falsches Ereignis: {:?}", x),
        }
    }

    #[test]
    fn ruecknahme_kommt_an() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        assert!(!verarbeiten(&json!({"v":1,"t":"control-revoked","peer":3}), &tx, &z));
        assert_eq!(rx.try_recv().unwrap(), Ereignis::ControlGiveBack { peer: 3 });
    }

    #[test]
    fn welcome_fills_the_state() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        let v = json!({
            "v":1,"t":"welcome","you":7,"room":"111-222-333","titel":"Test",
            "host":7,"server_version":"1.0",
            "peers":[{"id":9,"name":"Ben","audio_muted":true}],
            "caps":{},"e2e_active":false
        });
        assert!(!verarbeiten(&v, &tx, &z));
        let zu = z.lock().unwrap().clone();
        assert_eq!(zu.ich, 7);
        assert_eq!(zu.gastgeber, 7);
        assert_eq!(zu.leute.len(), 1);
        assert_eq!(zu.leute[0].name, "Ben");
        assert!(zu.leute[0].ton_aus);
        match rx.try_recv().unwrap() {
            Ereignis::Willkommen { ich, leute, .. } => {
                assert_eq!(ich, 7);
                assert_eq!(leute.len(), 1);
            }
            other => panic!("falsches Ereignis: {:?}", other),
        }
    }

    #[test]
    fn participants_come_and_go() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        verarbeiten(
            &json!({"t":"peer-join","peer":{"id":3,"name":"Cara"}}),
            &tx,
            &z,
        );
        assert_eq!(z.lock().unwrap().leute.len(), 1);
        verarbeiten(&json!({"t":"peer-leave","id":3}), &tx, &z);
        assert_eq!(z.lock().unwrap().leute.len(), 0);
        assert!(matches!(rx.try_recv().unwrap(), Ereignis::Dazu(_)));
        assert!(matches!(rx.try_recv().unwrap(), Ereignis::Weg(3)));
    }

    #[test]
    fn warteraum_wird_mitgefuehrt() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        verarbeiten(&json!({"t":"lobby-add","peer":5,"name":"Dora"}), &tx, &z);
        verarbeiten(&json!({"t":"lobby-state","on":true}), &tx, &z);
        {
            let zu = z.lock().unwrap();
            assert_eq!(zu.wartende.len(), 1);
            assert!(zu.warteraum_an);
        }
        verarbeiten(&json!({"t":"lobby-del","peer":5}), &tx, &z);
        assert_eq!(z.lock().unwrap().wartende.len(), 0);
    }

    #[test]
    fn muted_and_hand_change_the_participant() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        verarbeiten(&json!({"t":"peer-join","peer":{"id":4,"name":"Eva"}}), &tx, &z);
        verarbeiten(&json!({"t":"mute","peer":4,"kind":"audio","on":true}), &tx, &z);
        verarbeiten(&json!({"t":"hand","peer":4,"on":true}), &tx, &z);
        let zu = z.lock().unwrap().clone();
        assert!(zu.leute[0].ton_aus);
        assert!(zu.leute[0].hand);
    }

    #[test]
    fn kick_ends_the_session() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        assert!(verarbeiten(&json!({"t":"kicked","msg":"tschuess"}), &tx, &z));
        assert!(verarbeiten(&json!({"t":"ended","msg":"vorbei"}), &tx, &z));
        assert!(verarbeiten(&json!({"t":"denied","msg":"nein"}), &tx, &z));
        assert!(!verarbeiten(&json!({"t":"speaker","peer":2}), &tx, &z));
    }

    #[test]
    fn unknown_message_does_not_disturb() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(State::default()));
        assert!(!verarbeiten(&json!({"t":"gibt-es-nicht","x":1}), &tx, &z));
    }
}
