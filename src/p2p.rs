//! Direct peer to peer video path (UDP hole punching).
//!
//! Everything a session needs - handshake, input, files, control - keeps
//! running over the relay WebSocket: it is reliable, already encrypted and
//! tiny. Only the video stream moves to a direct UDP path when one can be
//! established, which takes the relay out of the loop for the one thing that
//! actually costs bandwidth and latency.
//!
//! How the path is found:
//!
//! 1. Both sides bind one UDP socket and collect candidate addresses: the
//!    local interface address and the address a STUN server sees (that is the
//!    public one the router mapped for exactly this socket).
//! 2. The candidates travel through the already encrypted relay channel
//!    (`P2pOffer`), so nobody in the middle can inject fake ones.
//! 3. Both sides keep sending small encrypted "punch" datagrams to every
//!    candidate of the other side. The first one that arrives opens the NAT
//!    mapping in that direction; the answer proves the way back works too.
//! 4. From then on video frames go out as encrypted fragments over UDP. If
//!    nothing arrives for a while we silently fall back to the relay.
//!
//! Losing a datagram breaks the H.264 reference chain, so an incomplete frame
//! is dropped and a fresh keyframe is requested over the reliable channel -
//! the same recovery the viewer already uses.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tokio::net::UdpSocket;

use crate::crypto::UdpCipher;

/// Datagram types (first byte of the decrypted payload).
const PUNCH: u8 = 0;
const PUNCH_ACK: u8 = 1;
const FRAG: u8 = 2;

/// Payload bytes per datagram. 1200 keeps us below the usual 1500 byte MTU
/// even with IPv4 + UDP + our own header and the AES-GCM tag.
const CHUNK: usize = 1200;
/// Incomplete pictures are thrown away after this long.
const REASSEMBLY_TIMEOUT: Duration = Duration::from_millis(400);
/// No datagram for this long means the direct path is gone.
const DEAD_AFTER: Duration = Duration::from_secs(3);
/// Public STUN servers used to learn our own outside address.
const STUN_SERVERS: [&str; 3] = [
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Everything the two ends need to know about the direct path.
pub struct P2p {
    sock: Arc<UdpSocket>,
    cipher: Arc<UdpCipher>,
    /// address of the peer once a punch got through
    peer: Mutex<Option<SocketAddr>>,
    /// candidates the peer offered
    remote: Mutex<Vec<SocketAddr>>,
    pub direct: Arc<AtomicBool>,
    last_rx: Mutex<Instant>,
    rtt_ms: AtomicU32,
    frame_id: AtomicU32,
    stop: Arc<AtomicBool>,
    /// counters for the self test / diagnostics
    pub sent_frames: AtomicU64,
    pub sent_bytes: AtomicU64,
    pub lost_frames: AtomicU64,
}

/// A picture that is still being put back together.
struct Pending {
    parts: Vec<Option<Vec<u8>>>,
    got: usize,
    started: Instant,
}

impl P2p {
    /// Binds the socket. The port is chosen by the OS and is the one the
    /// candidates below refer to. Synchronous on purpose so a session can set
    /// the direct path up without waiting for anything.
    pub fn new(key: [u8; 32], is_host: bool, stop: Arc<AtomicBool>) -> Result<Arc<Self>> {
        let raw = std::net::UdpSocket::bind("0.0.0.0:0")?;
        raw.set_nonblocking(true)?;
        let sock = UdpSocket::from_std(raw)?;
        Ok(Arc::new(Self {
            sock: Arc::new(sock),
            cipher: Arc::new(UdpCipher::new(&key, is_host)),
            peer: Mutex::new(None),
            remote: Mutex::new(Vec::new()),
            direct: Arc::new(AtomicBool::new(false)),
            last_rx: Mutex::new(Instant::now()),
            rtt_ms: AtomicU32::new(0),
            frame_id: AtomicU32::new(0),
            stop,
            sent_frames: AtomicU64::new(0),
            sent_bytes: AtomicU64::new(0),
            lost_frames: AtomicU64::new(0),
        }))
    }

    pub fn local_port(&self) -> u16 {
        self.sock.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    pub fn rtt(&self) -> u32 {
        self.rtt_ms.load(Ordering::Relaxed)
    }

    pub fn is_direct(&self) -> bool {
        self.direct.load(Ordering::Relaxed)
    }

    /// Addresses the peer should try: our local one plus whatever the STUN
    /// servers report for this very socket.
    pub async fn candidates(&self) -> Vec<String> {
        let port = self.local_port();
        let mut out: Vec<String> = Vec::new();
        if let Some(ip) = local_ip() {
            out.push(format!("{}:{}", ip, port));
        }
        for srv in STUN_SERVERS {
            match stun_reflexive(&self.sock, srv).await {
                Ok(a) => {
                    let s = a.to_string();
                    if !out.contains(&s) {
                        out.push(s);
                    }
                    break; // one public address is enough
                }
                Err(e) => crate::capture::log_line(&format!("stun {}: {}", srv, e)),
            }
        }
        out
    }

    pub fn set_remote(&self, addrs: &[String]) {
        let mut list: Vec<SocketAddr> = Vec::new();
        for a in addrs {
            if let Ok(sa) = a.parse::<SocketAddr>() {
                if sa.port() != 0 {
                    list.push(sa);
                }
            }
        }
        crate::capture::log_line(&format!("p2p Kandidaten der Gegenstelle: {:?}", list));
        *self.remote.lock().unwrap() = list;
    }

    /// Sends one proto message as encrypted fragments. Returns false when no
    /// direct path is up (caller falls back to the relay).
    pub async fn send_msg(&self, plain: &[u8]) -> bool {
        let peer = match *self.peer.lock().unwrap() {
            Some(p) => p,
            None => return false,
        };
        if !self.is_direct() {
            return false;
        }
        let id = self.frame_id.fetch_add(1, Ordering::Relaxed);
        let count = plain.len().div_ceil(CHUNK).max(1);
        if count > u16::MAX as usize {
            return false;
        }
        for (i, part) in plain.chunks(CHUNK).enumerate() {
            let mut body = Vec::with_capacity(part.len() + 9);
            body.push(FRAG);
            body.extend_from_slice(&id.to_be_bytes());
            body.extend_from_slice(&(i as u16).to_be_bytes());
            body.extend_from_slice(&(count as u16).to_be_bytes());
            body.extend_from_slice(part);
            let sealed = self.cipher.seal(&body);
            self.sent_bytes
                .fetch_add(sealed.len() as u64, Ordering::Relaxed);
            if self.sock.send_to(&sealed, peer).await.is_err() {
                return false;
            }
        }
        self.sent_frames.fetch_add(1, Ordering::Relaxed);
        true
    }

    async fn send_to(&self, addr: SocketAddr, kind: u8, stamp: u64) {
        let mut body = Vec::with_capacity(9);
        body.push(kind);
        body.extend_from_slice(&stamp.to_be_bytes());
        let sealed = self.cipher.seal(&body);
        let _ = self.sock.send_to(&sealed, addr).await;
    }

    /// Keeps knocking until a path is open, then keeps the NAT mapping alive
    /// and notices when the path dies.
    pub async fn punch_loop(self: Arc<Self>, on_state: impl Fn(bool, u32) + Send + 'static) {
        let started = Instant::now();
        let mut announced = false;
        while !self.stop.load(Ordering::Relaxed) {
            let direct = self.is_direct();
            let targets: Vec<SocketAddr> = if direct {
                self.peer.lock().unwrap().iter().copied().collect()
            } else {
                self.remote.lock().unwrap().clone()
            };
            let stamp = started.elapsed().as_millis() as u64;
            for a in targets {
                self.send_to(a, PUNCH, stamp).await;
            }
            if direct {
                let quiet = self.last_rx.lock().unwrap().elapsed();
                if quiet > DEAD_AFTER {
                    self.direct.store(false, Ordering::Relaxed);
                    *self.peer.lock().unwrap() = None;
                    announced = false;
                    on_state(false, 0);
                    crate::capture::log_line("p2p: direkte Verbindung verloren");
                }
            }
            if direct && !announced {
                announced = true;
                on_state(true, self.rtt());
            }
            tokio::time::sleep(Duration::from_millis(if direct { 1000 } else { 250 })).await;
        }
    }

    /// Receives datagrams, answers punches and hands finished messages on.
    pub async fn recv_loop(self: Arc<Self>, sink: impl Fn(Vec<u8>) + Send + 'static) {
        let mut buf = vec![0u8; 65536];
        let mut pending: HashMap<u32, Pending> = HashMap::new();
        let started = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            let (n, from) = match tokio::time::timeout(
                Duration::from_millis(500),
                self.sock.recv_from(&mut buf),
            )
            .await
            {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => continue,
                Err(_) => {
                    drop_stale(&mut pending, &self);
                    continue;
                }
            };
            let plain = match self.cipher.open(&buf[..n]) {
                Some(p) => p,
                None => continue, // not ours - ignore silently
            };
            if plain.is_empty() {
                continue;
            }
            *self.last_rx.lock().unwrap() = Instant::now();
            match plain[0] {
                PUNCH => {
                    let stamp = be64(&plain[1..]);
                    self.send_to(from, PUNCH_ACK, stamp).await;
                    // hearing from the peer is enough to know where to send
                    let mut p = self.peer.lock().unwrap();
                    if p.is_none() {
                        crate::capture::log_line(&format!("p2p: Gegenstelle antwortet von {}", from));
                        *p = Some(from);
                    }
                }
                PUNCH_ACK => {
                    let stamp = be64(&plain[1..]);
                    let now = started.elapsed().as_millis() as u64;
                    self.rtt_ms
                        .store(now.saturating_sub(stamp) as u32, Ordering::Relaxed);
                    let mut p = self.peer.lock().unwrap();
                    if p.is_none() {
                        *p = Some(from);
                    }
                    drop(p);
                    if !self.direct.swap(true, Ordering::Relaxed) {
                        crate::capture::log_line(&format!(
                            "p2p: direkter Weg steht ueber {} ({} ms)",
                            from,
                            self.rtt()
                        ));
                    }
                }
                FRAG => {
                    if plain.len() < 9 {
                        continue;
                    }
                    let id = u32::from_be_bytes([plain[1], plain[2], plain[3], plain[4]]);
                    let idx = u16::from_be_bytes([plain[5], plain[6]]) as usize;
                    let count = u16::from_be_bytes([plain[7], plain[8]]) as usize;
                    if count == 0 || idx >= count || count > 4096 {
                        continue;
                    }
                    let e = pending.entry(id).or_insert_with(|| Pending {
                        parts: vec![None; count],
                        got: 0,
                        started: Instant::now(),
                    });
                    if e.parts.len() != count {
                        continue;
                    }
                    if e.parts[idx].is_none() {
                        e.parts[idx] = Some(plain[9..].to_vec());
                        e.got += 1;
                    }
                    if e.got == count {
                        let mut msg = Vec::new();
                        for p in e.parts.iter().flatten() {
                            msg.extend_from_slice(p);
                        }
                        pending.remove(&id);
                        sink(msg);
                    }
                    drop_stale(&mut pending, &self);
                }
                _ => {}
            }
        }
    }
}

fn drop_stale(pending: &mut HashMap<u32, Pending>, p2p: &Arc<P2p>) {
    let before = pending.len();
    pending.retain(|_, v| v.started.elapsed() < REASSEMBLY_TIMEOUT);
    let lost = before - pending.len();
    if lost > 0 {
        p2p.lost_frames.fetch_add(lost as u64, Ordering::Relaxed);
    }
}

fn be64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    let n = b.len().min(8);
    a[..n].copy_from_slice(&b[..n]);
    u64::from_be_bytes(a)
}

/// The address of the interface that would be used to reach the internet.
/// No packet is actually sent - connect() on UDP only sets the default peer.
pub fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

/// Minimal STUN client (RFC 5389): one binding request, read the
/// XOR-MAPPED-ADDRESS out of the answer. That is the address the outside
/// world sees for *this* socket, which is exactly what hole punching needs.
pub async fn stun_reflexive(sock: &UdpSocket, server: &str) -> Result<SocketAddr> {
    const MAGIC: u32 = 0x2112_A442;
    let mut req = [0u8; 20];
    req[0..2].copy_from_slice(&0x0001u16.to_be_bytes()); // binding request
    req[2..4].copy_from_slice(&0u16.to_be_bytes()); // no attributes
    req[4..8].copy_from_slice(&MAGIC.to_be_bytes());
    let tid = crate::crypto::random_bytes(12);
    req[8..20].copy_from_slice(&tid);

    let addr = tokio::net::lookup_host(server)
        .await?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow!("keine IPv4 fuer {}", server))?;
    sock.send_to(&req, addr).await?;

    let mut buf = [0u8; 1024];
    let deadline = Instant::now() + Duration::from_millis(1200);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(anyhow!("keine Antwort"));
        }
        let (n, from) = tokio::time::timeout(left, sock.recv_from(&mut buf)).await??;
        if from != addr || n < 20 {
            continue;
        }
        let b = &buf[..n];
        if b[0..2] != 0x0101u16.to_be_bytes() || b[8..20] != tid[..] {
            continue;
        }
        let mut p = 20usize;
        while p + 4 <= n {
            let atype = u16::from_be_bytes([b[p], b[p + 1]]);
            let alen = u16::from_be_bytes([b[p + 2], b[p + 3]]) as usize;
            let val = p + 4;
            if val + alen > n {
                break;
            }
            // 0x0020 = XOR-MAPPED-ADDRESS, 0x0001 = MAPPED-ADDRESS (old)
            if (atype == 0x0020 || atype == 0x0001) && alen >= 8 && b[val + 1] == 0x01 {
                let raw_port = u16::from_be_bytes([b[val + 2], b[val + 3]]);
                let raw_ip = u32::from_be_bytes([b[val + 4], b[val + 5], b[val + 6], b[val + 7]]);
                let (port, ip) = if atype == 0x0020 {
                    (raw_port ^ (MAGIC >> 16) as u16, raw_ip ^ MAGIC)
                } else {
                    (raw_port, raw_ip)
                };
                return Ok(SocketAddr::from((std::net::Ipv4Addr::from(ip), port)));
            }
            p = val + alen.div_ceil(4) * 4; // attributes are padded to 4 bytes
        }
        return Err(anyhow!("keine Adresse in der STUN-Antwort"));
    }
}

/// `freeviewer --p2ptest` - can this machine see its own public address and
/// how long does that take?
pub fn selftest() -> String {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return format!("keine Laufzeitumgebung: {}\n", e),
    };
    rt.block_on(async {
        let mut out = String::new();
        match local_ip() {
            Some(ip) => out.push_str(&format!("lokale Adresse: {}\n", ip)),
            None => out.push_str("lokale Adresse: unbekannt\n"),
        }
        let sock = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => return out + &format!("kein UDP-Socket: {}\n", e),
        };
        out.push_str(&format!(
            "UDP-Port: {}\n",
            sock.local_addr().map(|a| a.port()).unwrap_or(0)
        ));
        let mut any = false;
        for srv in STUN_SERVERS {
            let t = Instant::now();
            match stun_reflexive(&sock, srv).await {
                Ok(a) => {
                    any = true;
                    out.push_str(&format!(
                        "{:<28} -> oeffentlich {} ({} ms)\n",
                        srv,
                        a,
                        t.elapsed().as_millis()
                    ));
                }
                Err(e) => out.push_str(&format!("{:<28} -> {}\n", srv, e)),
            }
        }
        // A second request from the same socket: if the outside port stays the
        // same the NAT is "cone" like and hole punching works. If it changes
        // for every destination it is a symmetric NAT and only the relay helps.
        if any {
            let mut seen: Vec<SocketAddr> = Vec::new();
            for srv in STUN_SERVERS {
                if let Ok(a) = stun_reflexive(&sock, srv).await {
                    if !seen.contains(&a) {
                        seen.push(a);
                    }
                }
            }
            if seen.len() == 1 {
                out.push_str("NAT-Verhalten: gleiche Aussenadresse fuer alle Ziele - Hole Punching moeglich\n");
            } else {
                out.push_str(&format!(
                    "NAT-Verhalten: symmetrisch ({:?}) - direkter Weg unwahrscheinlich, Relay bleibt\n",
                    seen
                ));
            }
        } else {
            out.push_str("kein STUN erreichbar - direkter Weg nicht ermittelbar\n");
        }
        out
    })
}
