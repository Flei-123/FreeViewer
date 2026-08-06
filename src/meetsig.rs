//! Natives Meeting - Stufe 1: die Signalisierung ohne Browser.
//!
//! Bisher lief das Meeting im Client in einem nackten Browserfenster. Der
//! Browser hat dabei ALLES gemacht: Verhandlung mit dem Medienserver, Ton,
//! Bild, Echo-Unterdrueckung, Kacheln. Fuer die native Fassung bauen wir das
//! Stueck fuer Stueck selbst nach. Dieses Modul ist der erste Stein: die
//! Steuerleitung zum Medienserver (WebSocket, JSON) - also alles, was KEIN
//! Ton und kein Bild ist: beitreten, Teilnehmerliste, Chat, Handzeichen,
//! Stummschalten, Warteraum, Gastgeber-Eingriffe.
//!
//! Bewusst nur mit dem, was FreeViewer sowieso schon dabei hat
//! (tokio + tokio-tungstenite + serde_json) - kein neuer Fremdcode.
//!
//! Das Protokoll ist dasselbe, das die Browserseite spricht (Server:
//! freemeet-sfu/src/proto.rs). Jede Nachricht traegt `v` (Protokollversion);
//! unbekannte Nachrichten werden ruhig verworfen statt die Sitzung zu killen.

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// Protokollversion, die dieser Client spricht.
pub const PROTO_VERSION: u16 = 1;

/// Ein Teilnehmer, so wie ihn der Server beschreibt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Teilnehmer {
    pub id: u64,
    pub name: String,
    pub ton_aus: bool,
    pub bild_aus: bool,
    pub hand: bool,
    /// Nicht leer = dieser Teilnehmer erlaubt Fernsteuerung ueber FreeViewer.
    pub fvid: String,
}

impl Teilnehmer {
    fn aus_json(v: &serde_json::Value) -> Teilnehmer {
        Teilnehmer {
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

/// Was der Oberflaeche gemeldet wird.
#[derive(Debug, Clone, PartialEq)]
pub enum Ereignis {
    /// Der Server hat uns hereingelassen.
    Willkommen {
        ich: u64,
        raum: String,
        titel: String,
        gastgeber: u64,
        leute: Vec<Teilnehmer>,
        server: String,
    },
    Dazu(Teilnehmer),
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
    Stumm {
        peer: u64,
        art: String,
        an: bool,
    },
    /// Der Gastgeber hat UNS stummgeschaltet - die Spur muss wirklich aus.
    ZwangStumm {
        art: String,
    },
    Gastgeber(u64),
    Sprecher(Option<u64>),
    Fernsteuerung {
        peer: u64,
        fvid: String,
    },
    /// Wo steht der Mauszeiger dessen, der gerade teilt (0..1 in SEINEM
    /// Bild)? Damit kann ein Zuschauer genau dorthin zoomen.
    Zeiger {
        peer: u64,
        x: f32,
        y: f32,
    },
    /// Wir sitzen im Warteraum.
    Wartet {
        titel: String,
        text: String,
    },
    /// Jemand wartet vor der Tuer (nur beim Gastgeber).
    WarteDazu {
        peer: u64,
        name: String,
    },
    WarteWeg {
        peer: u64,
    },
    WarteZustand {
        an: bool,
    },
    Abgewiesen(String),
    Rausgeworfen(String),
    Beendet(String),
    /// Der Server sagt, welche m-line zu wem gehoert - ohne das weiss der
    /// native Client nicht, wessen Stimme da gerade ankommt.
    Spur {
        mid: String,
        peer: u64,
        art: String,
        bildschirm: bool,
    },
    /// SDP-Angebot/-Antwort des Servers. Kommt erst in Stufe 2 zum Einsatz,
    /// wird aber schon jetzt sauber durchgereicht statt verworfen.
    Sdp {
        art: String,
        sdp: String,
    },
    Fehler {
        code: String,
        text: String,
    },
    Getrennt(String),
}

/// Was die Oberflaeche der Sitzung sagen kann.
#[derive(Debug, Clone)]
enum Befehl {
    Chat(String),
    Tippt(bool),
    Hand(bool),
    Stumm { art: String, an: bool },
    Warteraum { aktion: String, peer: Option<u64> },
    Gastgeber { aktion: String, peer: Option<u64> },
    Bildschirm(bool),
    /// Fernsteuerung anbieten/zurueckziehen. Leere fvid = zurueckgezogen.
    Fernsteuerung { an: bool, fvid: String },
    /// Rohes JSON - dafuer da, dass Stufe 2 (Angebot/Antwort) nichts umbauen muss.
    Roh(serde_json::Value),
    Verlassen,
}

/// Eine laufende Meeting-Sitzung. Wird beim Fallenlassen sauber beendet.
pub struct Sitzung {
    befehle: UnboundedSender<Befehl>,
    ereignisse: Receiver<Ereignis>,
    /// Zuletzt gesehener Zustand - damit die Oberflaeche nicht selbst buchfuehren muss.
    zustand: Arc<Mutex<Zustand>>,
}

#[derive(Debug, Clone, Default)]
pub struct Zustand {
    pub ich: u64,
    pub raum: String,
    pub titel: String,
    pub gastgeber: u64,
    pub leute: Vec<Teilnehmer>,
    pub wartende: Vec<(u64, String)>,
    pub warteraum_an: bool,
    pub im_warteraum: bool,
    pub verbunden: bool,
    pub letzter_fehler: String,
}

impl Sitzung {
    /// Neue Ereignisse abholen (blockiert nie).
    pub fn abholen(&self) -> Vec<Ereignis> {
        let mut aus = Vec::new();
        while let Ok(e) = self.ereignisse.try_recv() {
            aus.push(e);
        }
        aus
    }

    pub fn zustand(&self) -> Zustand {
        self.zustand.lock().map(|z| z.clone()).unwrap_or_default()
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
    /// art: "audio" oder "video"; an = ausgeschaltet (so wie im Protokoll).
    pub fn stumm(&self, art: &str, an: bool) {
        let _ = self.befehle.send(Befehl::Stumm {
            art: art.to_string(),
            an,
        });
    }
    pub fn bildschirm(&self, an: bool) {
        let _ = self.befehle.send(Befehl::Bildschirm(an));
    }
    /// Eigene Zeigerposition im geteilten Bild melden (0..1). Nur sinnvoll,
    /// solange wirklich geteilt wird.
    pub fn zeiger(&self, x: f32, y: f32) {
        let _ = self
            .befehle
            .send(Befehl::Roh(serde_json::json!({"t":"cursor","x":x,"y":y})));
    }

    /// Fernsteuerung ueber FreeViewer anbieten (`an`) oder zuruecknehmen.
    /// `fvid` ist die eigene FreeViewer-Nummer; beim Zuruecknehmen egal.
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

impl Drop for Sitzung {
    fn drop(&mut self) {
        let _ = self.befehle.send(Befehl::Verlassen);
    }
}

/// Adresse der Signalisierung aus der Meet-Adresse ableiten:
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

/// Einem Meeting beitreten. Laeuft im Hintergrund weiter, bis `verlassen()`
/// gerufen wird oder die Sitzung fallengelassen wird.
pub fn beitreten(basis: &str, raum: &str, pass: &str, name: &str, fvid: &str) -> Result<Sitzung> {
    let url = ws_adresse(basis);
    let (bef_tx, bef_rx) = unbounded_channel::<Befehl>();
    let (ev_tx, ev_rx) = channel::<Ereignis>();
    let zustand = Arc::new(Mutex::new(Zustand {
        raum: raum.to_string(),
        ..Default::default()
    }));

    let z2 = zustand.clone();
    let (raum, pass, name, fvid) = (
        raum.to_string(),
        pass.to_string(),
        name.to_string(),
        fvid.to_string(),
    );

    // Eigener Faden mit eigener kleiner Laufzeit: die Oberflaeche soll von
    // der Netzarbeit nichts merken, und wir wollen nicht davon abhaengen,
    // dass irgendwo sonst schon eine Laufzeit laeuft.
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
                if let Err(e) = lauf(&url, &raum, &pass, &name, &fvid, bef_rx, &ev_tx, &z2).await {
                    if let Ok(mut z) = z2.lock() {
                        z.verbunden = false;
                        z.letzter_fehler = e.to_string();
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

    Ok(Sitzung {
        befehle: bef_tx,
        ereignisse: ev_rx,
        zustand,
    })
}

#[allow(clippy::too_many_arguments)]
async fn lauf(
    url: &str,
    raum: &str,
    pass: &str,
    name: &str,
    fvid: &str,
    mut befehle: UnboundedReceiver<Befehl>,
    ev: &Sender<Ereignis>,
    zustand: &Arc<Mutex<Zustand>>,
) -> Result<()> {
    let (mut ws, _) = verbinden(url).await?;
    if let Ok(mut z) = zustand.lock() {
        z.verbunden = true;
    }

    // Beitritt. Die Faehigkeiten sind ehrlich: heute kann der native Client
    // noch KEIN Simulcast und keine E2E-Schicht. Ton/Bild kommen in Stufe 2,
    // dann wandert "opus"/"h264" hier hinein.
    let join = umschlag(json!({
        "t": "join",
        "room": raum,
        "name": name,
        "pass": pass,
        "fvid": fvid,
        "caps": { "simulcast": false, "codecs": [], "e2e": false }
    }));
    ws.send(Message::Text(join.into())).await?;

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
    ticker.tick().await; // der erste Schlag kommt sofort

    loop {
        tokio::select! {
            // ---- vom Server ------------------------------------------------
            nachricht = ws.next() => {
                let Some(nachricht) = nachricht else { break };
                match nachricht {
                    Ok(Message::Text(t)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                            if verarbeiten(&v, ev, zustand) {
                                // Sitzung ist zu Ende (Kick/Ende/Abweisung)
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
            // ---- von der Oberflaeche --------------------------------------
            b = befehle.recv() => {
                let Some(b) = b else { break };
                let raus = match b {
                    Befehl::Chat(text) => Some(json!({"t":"chat","text":text})),
                    Befehl::Tippt(an) => Some(json!({"t":"typing","on":an})),
                    Befehl::Hand(an) => Some(json!({"t":"hand","on":an})),
                    Befehl::Stumm { art, an } => Some(json!({"t":"mute","kind":art,"on":an})),
                    Befehl::Bildschirm(an) => Some(json!({"t":"screen","on":an})),
                    Befehl::Fernsteuerung { an, fvid } => {
                        // Nur Ziffern raus - die Nummer kommt aus der eigenen
                        // Einstellung, aber der Server erwartet sie sauber.
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

/// Eine Servernachricht verarbeiten. Rueckgabe `true` = Sitzung ist vorbei.
fn verarbeiten(
    v: &serde_json::Value,
    ev: &Sender<Ereignis>,
    zustand: &Arc<Mutex<Zustand>>,
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
            let leute: Vec<Teilnehmer> = v
                .get("peers")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().map(Teilnehmer::aus_json).collect())
                .unwrap_or_default();
            if let Ok(mut z) = zustand.lock() {
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
                .map(Teilnehmer::aus_json)
                .unwrap_or_default();
            if let Ok(mut z) = zustand.lock() {
                z.leute.retain(|x| x.id != p.id);
                z.leute.push(p.clone());
            }
            let _ = ev.send(Ereignis::Dazu(p));
        }
        "peer-leave" => {
            let id = u64f("id");
            if let Ok(mut z) = zustand.lock() {
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
            // Werte ausserhalb des Bildes waeren eine Falschmeldung - lieber
            // gar nichts sagen, als auf eine erfundene Stelle zu zoomen.
            if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) {
                let _ = ev.send(Ereignis::Zeiger {
                    peer: u64f("peer"),
                    x,
                    y,
                });
            }
        }
        "hand" => {
            let (peer, an) = (u64f("peer"), boolf("on"));
            if let Ok(mut z) = zustand.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    p.hand = an;
                }
            }
            let _ = ev.send(Ereignis::Hand { peer, an });
        }
        "mute" => {
            let (peer, art, an) = (u64f("peer"), strf("kind"), boolf("on"));
            if let Ok(mut z) = zustand.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    if art == "audio" {
                        p.ton_aus = an;
                    } else {
                        p.bild_aus = an;
                    }
                }
            }
            let _ = ev.send(Ereignis::Stumm { peer, art, an });
        }
        "force-mute" => {
            let _ = ev.send(Ereignis::ZwangStumm { art: strf("kind") });
        }
        "host" => {
            let peer = u64f("peer");
            if let Ok(mut z) = zustand.lock() {
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
            if let Ok(mut z) = zustand.lock() {
                if let Some(p) = z.leute.iter_mut().find(|x| x.id == peer) {
                    p.fvid = fvid.clone();
                }
            }
            let _ = ev.send(Ereignis::Fernsteuerung { peer, fvid });
        }
        "waiting" => {
            if let Ok(mut z) = zustand.lock() {
                z.im_warteraum = true;
            }
            let _ = ev.send(Ereignis::Wartet {
                titel: strf("titel"),
                text: strf("msg"),
            });
        }
        "lobby-add" => {
            let (peer, name) = (u64f("peer"), strf("name"));
            if let Ok(mut z) = zustand.lock() {
                z.wartende.retain(|(id, _)| *id != peer);
                z.wartende.push((peer, name.clone()));
            }
            let _ = ev.send(Ereignis::WarteDazu { peer, name });
        }
        "lobby-del" => {
            let peer = u64f("peer");
            if let Ok(mut z) = zustand.lock() {
                z.wartende.retain(|(id, _)| *id != peer);
            }
            let _ = ev.send(Ereignis::WarteWeg { peer });
        }
        "lobby-state" => {
            let an = boolf("on");
            if let Ok(mut z) = zustand.lock() {
                z.warteraum_an = an;
            }
            let _ = ev.send(Ereignis::WarteZustand { an });
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
                bildschirm: boolf("screen"),
            });
        }
        "offer" | "answer" => {
            let _ = ev.send(Ereignis::Sdp {
                art: t.to_string(),
                sdp: strf("sdp"),
            });
        }
        "error" => {
            if let Ok(mut z) = zustand.lock() {
                z.letzter_fehler = format!("{}: {}", strf("code"), strf("msg"));
            }
            let _ = ev.send(Ereignis::Fehler {
                code: strf("code"),
                text: strf("msg"),
            });
        }
        // pong, quality, transport, screen-off: fuer Stufe 1 ohne Belang.
        _ => {}
    }
    false
}


/// Verbindung aufbauen - und zwar zu JEDER Adresse, die der Name liefert.
///
/// Warum nicht einfach connect_async? Im Heimnetz zeigt meet.fleitec.com auf
/// ZWEI Adressen: die oeffentliche und eine interne (Split-DNS). Die interne
/// beantwortet aber keinen Port 443. Wer nur die erste Adresse probiert,
/// haengt 20 Sekunden im Zeitueberschreitung-Fehler 10060 fest - genau das
/// ist auf FLEI-ONE passiert. Also: alle Adressen durchprobieren, oeffentliche
/// zuerst, je 4 Sekunden Geduld.
async fn verbinden(
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
    // Oeffentliche Adressen zuerst - interne sind im Heimnetz oft Sackgassen.
    adressen.sort_by_key(|a| match a.ip() {
        std::net::IpAddr::V4(v) => u8::from(v.is_private() || v.is_loopback()),
        std::net::IpAddr::V6(_) => 1,
    });

    let mut letzter = String::new();
    // Zwei Anlaeufe je Adresse mit grosszuegiger Frist: gemessen an Justins
    // Rechner braucht der erste Verbindungsaufbau ueber die eigene
    // oeffentliche Adresse (NAT-Rueckschleife durch die Fritzbox) manchmal
    // laenger als vier Sekunden. Mit einem einzigen kurzen Versuch stand
    // dort sporadisch "keine Antwort", obwohl das Netz in Ordnung war.
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
    fn jede_nachricht_traegt_die_protokollversion() {
        let s = umschlag(json!({"t":"hand","on":true}));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["t"], "hand");
        assert_eq!(v["on"], true);
    }

    #[test]
    fn willkommen_fuellt_den_zustand() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(Zustand::default()));
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
    fn teilnehmer_kommen_und_gehen() {
        let (tx, rx) = channel();
        let z = Arc::new(Mutex::new(Zustand::default()));
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
        let z = Arc::new(Mutex::new(Zustand::default()));
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
    fn stumm_und_hand_aendern_den_teilnehmer() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(Zustand::default()));
        verarbeiten(&json!({"t":"peer-join","peer":{"id":4,"name":"Eva"}}), &tx, &z);
        verarbeiten(&json!({"t":"mute","peer":4,"kind":"audio","on":true}), &tx, &z);
        verarbeiten(&json!({"t":"hand","peer":4,"on":true}), &tx, &z);
        let zu = z.lock().unwrap().clone();
        assert!(zu.leute[0].ton_aus);
        assert!(zu.leute[0].hand);
    }

    #[test]
    fn rauswurf_beendet_die_sitzung() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(Zustand::default()));
        assert!(verarbeiten(&json!({"t":"kicked","msg":"tschuess"}), &tx, &z));
        assert!(verarbeiten(&json!({"t":"ended","msg":"vorbei"}), &tx, &z));
        assert!(verarbeiten(&json!({"t":"denied","msg":"nein"}), &tx, &z));
        assert!(!verarbeiten(&json!({"t":"speaker","peer":2}), &tx, &z));
    }

    #[test]
    fn unbekannte_nachricht_stoert_nicht() {
        let (tx, _rx) = channel();
        let z = Arc::new(Mutex::new(Zustand::default()));
        assert!(!verarbeiten(&json!({"t":"gibt-es-nicht","x":1}), &tx, &z));
    }
}
