//! File transfer. Symmetric: viewer and host use the very same code, so both
//! ends can send and receive.
//!
//! The pieces travel as ordinary `Msg::File*` messages inside the encrypted
//! session, i.e. the relay never sees a byte of the file either. A simple
//! window (unacknowledged bytes) keeps a big file from starving the video
//! stream and from filling the receiver's memory.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::proto::{Msg, CHUNK};
use crate::shared::Shared;

/// At most this many bytes may be unacknowledged before the sender waits.
const WINDOW: u64 = 4 * 1024 * 1024;
/// The receiver confirms after this many written bytes.
const ACK_EVERY: u64 = 512 * 1024;
/// If nothing is acknowledged for this long the transfer is given up.
const STALL: Duration = Duration::from_secs(90);

/// What the GUI shows for one running/finished transfer.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub id: u32,
    pub name: String,
    pub size: u64,
    pub done: u64,
    /// true = we are receiving it, false = we are sending it
    pub incoming: bool,
    pub finished: bool,
    pub error: String,
}

impl Progress {
    pub fn percent(&self) -> f32 {
        if self.size == 0 {
            return if self.finished { 1.0 } else { 0.0 };
        }
        (self.done as f32 / self.size as f32).clamp(0.0, 1.0)
    }
}

/// Where incoming files are written to: <Downloads>\FreeViewer.
pub fn default_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
        .map(PathBuf::from);
    let dir = match home {
        Some(h) => h.join("Downloads").join("FreeViewer"),
        None => crate::ident::config_dir().join("files"),
    };
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Only the bare file name survives - a peer must never be able to write
/// "..\\..\\Windows\\System32\\evil.dll".
pub fn safe_name(raw: &str) -> String {
    let base = raw
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("datei");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "datei".to_string()
    } else {
        cleaned
    }
}

/// Never overwrite: "bild.png" -> "bild (2).png".
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 2..10_000 {
        let cand = dir.join(format!("{} ({}){}", stem, n, ext));
        if !cand.exists() {
            return cand;
        }
    }
    dir.join(format!("{}.{}", name, std::process::id()))
}

struct Incoming {
    file: File,
    path: PathBuf,
    got: u64,
    acked: u64,
}

/// One file transfer engine per session.
pub struct Xfer {
    shared: Arc<Shared>,
    send: Arc<dyn Fn(Msg) + Send + Sync>,
    stop: Arc<AtomicBool>,
    incoming: HashMap<u32, Incoming>,
    acks: HashMap<u32, Arc<AtomicU64>>,
    next_id: u32,
}

impl Xfer {
    pub fn new(shared: Arc<Shared>, send: Arc<dyn Fn(Msg) + Send + Sync>) -> Self {
        Self {
            shared,
            send,
            stop: Arc::new(AtomicBool::new(false)),
            incoming: HashMap::new(),
            acks: HashMap::new(),
            next_id: 1,
        }
    }

    fn set_progress(shared: &Arc<Shared>, p: Progress) {
        let mut list = shared.xfers.lock().unwrap();
        match list.iter_mut().find(|x| x.id == p.id && x.incoming == p.incoming) {
            Some(slot) => *slot = p,
            None => {
                if list.len() > 40 {
                    list.remove(0);
                }
                list.push(p);
            }
        }
    }

    /// Starts sending `path` to the other side (own thread, non blocking).
    pub fn send_path(&mut self, path: PathBuf) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "datei".to_string());
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                Self::set_progress(
                    &self.shared,
                    Progress {
                        id,
                        name,
                        size: 0,
                        done: 0,
                        incoming: false,
                        finished: true,
                        error: format!("{}", e),
                    },
                );
                return;
            }
        };
        if meta.is_dir() {
            Self::set_progress(
                &self.shared,
                Progress {
                    id,
                    name,
                    size: 0,
                    done: 0,
                    incoming: false,
                    finished: true,
                    error: "Ordner werden (noch) nicht uebertragen".to_string(),
                },
            );
            return;
        }
        let size = meta.len();
        let acked = Arc::new(AtomicU64::new(0));
        self.acks.insert(id, acked.clone());

        Self::set_progress(
            &self.shared,
            Progress {
                id,
                name: name.clone(),
                size,
                done: 0,
                incoming: false,
                finished: false,
                error: String::new(),
            },
        );

        let send = self.send.clone();
        let stop = self.stop.clone();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let mut fail = |msg: String| {
                Self::set_progress(
                    &shared,
                    Progress {
                        id,
                        name: name.clone(),
                        size,
                        done: 0,
                        incoming: false,
                        finished: true,
                        error: msg.clone(),
                    },
                );
                send(Msg::FileEnd {
                    id,
                    ok: false,
                    msg,
                });
            };

            let mut f = match File::open(&path) {
                Ok(f) => f,
                Err(e) => return fail(format!("{}", e)),
            };
            send(Msg::FileOffer {
                id,
                name: name.clone(),
                size,
            });

            let mut buf = vec![0u8; CHUNK];
            let mut off = 0u64;
            let mut seen_ack = 0u64;
            let mut since = Instant::now();
            loop {
                if stop.load(Ordering::Relaxed) {
                    return fail("Sitzung beendet".to_string());
                }
                // flow control: never push more than WINDOW unacknowledged bytes
                while off.saturating_sub(acked.load(Ordering::Relaxed)) >= WINDOW {
                    if stop.load(Ordering::Relaxed) {
                        return fail("Sitzung beendet".to_string());
                    }
                    let now = acked.load(Ordering::Relaxed);
                    if now != seen_ack {
                        seen_ack = now;
                        since = Instant::now();
                    } else if since.elapsed() > STALL {
                        return fail("Gegenstelle antwortet nicht".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(3));
                }

                let n = match f.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => return fail(format!("{}", e)),
                };
                send(Msg::FileChunk {
                    id,
                    off,
                    data: buf[..n].to_vec(),
                });
                off += n as u64;
                Self::set_progress(
                    &shared,
                    Progress {
                        id,
                        name: name.clone(),
                        size,
                        done: off,
                        incoming: false,
                        finished: false,
                        error: String::new(),
                    },
                );
                // be nice to the video stream
                std::thread::sleep(Duration::from_millis(1));
            }

            send(Msg::FileEnd {
                id,
                ok: true,
                msg: String::new(),
            });

            // "done" means the other side has written every byte, not just
            // that we pushed them into the socket
            let mut waited = Instant::now();
            let mut seen = acked.load(Ordering::Relaxed);
            while acked.load(Ordering::Relaxed) < off {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let now = acked.load(Ordering::Relaxed);
                if now != seen {
                    seen = now;
                    waited = Instant::now();
                } else if waited.elapsed() > STALL {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let confirmed = acked.load(Ordering::Relaxed);
            Self::set_progress(
                &shared,
                Progress {
                    id,
                    name,
                    size,
                    done: confirmed.min(off),
                    incoming: false,
                    finished: true,
                    error: if confirmed >= off {
                        String::new()
                    } else {
                        format!("nur {} von {} Bytes bestaetigt", confirmed, off)
                    },
                },
            );
        });
    }

    /// Feed every `Msg::File*` of the session in here.
    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::FileOffer { id, name, size } => {
                let dir = self.shared.drop_dir.lock().unwrap().clone();
                let _ = fs::create_dir_all(&dir);
                let clean = safe_name(&name);
                let path = unique_path(&dir, &clean);
                match File::create(&path) {
                    Ok(file) => {
                        self.incoming.insert(
                            id,
                            Incoming {
                                file,
                                path: path.clone(),
                                got: 0,
                                acked: 0,
                            },
                        );
                        Self::set_progress(
                            &self.shared,
                            Progress {
                                id,
                                name: path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or(clean),
                                size,
                                done: 0,
                                incoming: true,
                                finished: false,
                                error: String::new(),
                            },
                        );
                    }
                    Err(e) => {
                        (self.send)(Msg::FileEnd {
                            id,
                            ok: false,
                            msg: format!("{}", e),
                        });
                        Self::set_progress(
                            &self.shared,
                            Progress {
                                id,
                                name: clean,
                                size,
                                done: 0,
                                incoming: true,
                                finished: true,
                                error: format!("{}", e),
                            },
                        );
                    }
                }
            }
            Msg::FileChunk { id, off, data } => {
                let mut broken: Option<String> = None;
                if let Some(inc) = self.incoming.get_mut(&id) {
                    if off != inc.got {
                        if inc.file.seek(SeekFrom::Start(off)).is_err() {
                            broken = Some("Schreibfehler".to_string());
                        }
                    }
                    if broken.is_none() {
                        match inc.file.write_all(&data) {
                            Ok(()) => {
                                inc.got = off + data.len() as u64;
                                if inc.got - inc.acked >= ACK_EVERY {
                                    inc.acked = inc.got;
                                    (self.send)(Msg::FileAck { id, got: inc.got });
                                }
                            }
                            Err(e) => broken = Some(format!("{}", e)),
                        }
                    }
                }
                if let Some(err) = broken {
                    self.incoming.remove(&id);
                    (self.send)(Msg::FileEnd {
                        id,
                        ok: false,
                        msg: err.clone(),
                    });
                    let mut list = self.shared.xfers.lock().unwrap();
                    if let Some(p) = list.iter_mut().find(|x| x.id == id && x.incoming) {
                        p.finished = true;
                        p.error = err;
                    }
                    return;
                }
                let done = self.incoming.get(&id).map(|i| i.got);
                if let Some(done) = done {
                    let mut list = self.shared.xfers.lock().unwrap();
                    if let Some(p) = list.iter_mut().find(|x| x.id == id && x.incoming) {
                        p.done = done;
                    }
                }
            }
            Msg::FileEnd { id, ok, msg } => {
                if let Some(mut inc) = self.incoming.remove(&id) {
                    let _ = inc.file.flush();
                    (self.send)(Msg::FileAck { id, got: inc.got });
                    let mut list = self.shared.xfers.lock().unwrap();
                    if let Some(p) = list.iter_mut().find(|x| x.id == id && x.incoming) {
                        p.finished = true;
                        p.done = inc.got;
                        if !ok {
                            p.error = if msg.is_empty() {
                                "abgebrochen".to_string()
                            } else {
                                msg
                            };
                        }
                    }
                    if !ok {
                        let _ = fs::remove_file(&inc.path);
                    }
                } else {
                    // the other side reports a problem with something we send
                    let mut list = self.shared.xfers.lock().unwrap();
                    if let Some(p) = list.iter_mut().find(|x| x.id == id && !x.incoming) {
                        p.finished = true;
                        if !ok {
                            p.error = msg;
                        }
                    }
                }
            }
            Msg::FileAck { id, got } => {
                if let Some(a) = self.acks.get(&id) {
                    a.fetch_max(got, Ordering::Relaxed);
                }
                let mut list = self.shared.xfers.lock().unwrap();
                if let Some(p) = list.iter_mut().find(|x| x.id == id && !x.incoming) {
                    if p.size > 0 && got >= p.size {
                        p.finished = true;
                        p.done = p.size;
                    }
                }
            }
            _ => {}
        }
    }

    /// Cancels everything that is still running (session is going away).
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let ids: Vec<u32> = self.incoming.keys().copied().collect();
        for (_, inc) in self.incoming.drain() {
            drop(inc.file);
            // do not pretend the fragment is a complete file
            let mut fragment = inc.path.clone().into_os_string();
            fragment.push(".unvollstaendig");
            let _ = fs::rename(&inc.path, PathBuf::from(fragment));
        }
        let mut list = self.shared.xfers.lock().unwrap();
        for p in list.iter_mut() {
            if p.incoming && !p.finished && ids.contains(&p.id) {
                p.finished = true;
                p.error = "Sitzung beendet".to_string();
            }
        }
    }
}

impl Drop for Xfer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// True for every message that belongs to a file transfer.
pub fn is_file_msg(m: &Msg) -> bool {
    matches!(
        m,
        Msg::FileOffer { .. } | Msg::FileChunk { .. } | Msg::FileEnd { .. } | Msg::FileAck { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_drop_folder() {
        assert_eq!(safe_name("..\\..\\Windows\\System32\\evil.dll"), "evil.dll");
        assert_eq!(safe_name("/etc/passwd"), "passwd");
        assert_eq!(safe_name("a:b?c*.txt"), "a_b_c_.txt");
        assert_eq!(safe_name("   "), "datei");
        assert_eq!(safe_name(".."), "datei");
        assert_eq!(safe_name("urlaub.zip"), "urlaub.zip");
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("fvtest{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let first = unique_path(&dir, "x.txt");
        fs::write(&first, b"1").unwrap();
        let second = unique_path(&dir, "x.txt");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("x (2).txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn percent_is_clamped() {
        let p = Progress {
            id: 1,
            name: "a".into(),
            size: 200,
            done: 50,
            incoming: true,
            finished: false,
            error: String::new(),
        };
        assert!((p.percent() - 0.25).abs() < 0.001);
    }
}
