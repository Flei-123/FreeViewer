//! State shared between the GUI thread and the async worker tasks.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc::UnboundedSender;

use crate::proto::{Msg, MODE_ADMIN};

pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub seq: u64,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub fps: f32,
    pub kbps: f32,
    pub latency_ms: f32,
}

pub struct Shared {
    pub relay_url: String,
    // host side
    pub my_id: Mutex<String>,
    pub password: Mutex<String>,
    pub host_status: Mutex<String>,
    pub host_peer: Mutex<String>,
    // viewer side
    pub viewer_status: Mutex<String>,
    pub frame: Mutex<Option<FrameData>>,
    pub remote_size: Mutex<(u32, u32)>,
    /// Remote pointer, normalized 0..10000 plus visibility.
    pub remote_cursor: Mutex<(i32, i32, bool)>,
    pub input_tx: Mutex<Option<UnboundedSender<Msg>>>,
    /// Clipboard text that arrived from the host and has to be written into
    /// the local clipboard by the clipboard worker thread.
    pub clip_in: Mutex<Option<String>>,
    /// How many clipboard updates the host has sent us (diagnostics/tests).
    pub clip_from_host: AtomicU32,
    pub connected: AtomicBool,
    pub connecting: AtomicBool,
    /// Active session profile (MODE_ADMIN / MODE_GAME).
    pub mode: AtomicU8,
    /// Host side: which screen the capture thread should grab.
    pub monitor: AtomicU8,
    /// Viewer side: the screens the host offers plus the active one.
    pub monitors: Mutex<Vec<crate::proto::MonitorInfo>>,
    pub active_monitor: AtomicU8,
    /// File transfers of this session (both directions).
    pub xfers: Mutex<Vec<crate::xfer::Progress>>,
    /// Transfer engine of the running session (host or viewer role).
    pub xfer: Mutex<Option<crate::xfer::Xfer>>,
    /// Where received files are written to.
    pub drop_dir: Mutex<std::path::PathBuf>,
    /// True while a direct peer to peer path carries the video.
    pub direct: AtomicBool,
    pub stats: Mutex<Stats>,
}

impl Shared {
    pub fn new(relay_url: String, password: String) -> Self {
        Self {
            relay_url,
            my_id: Mutex::new(String::new()),
            password: Mutex::new(password),
            host_status: Mutex::new("Starte...".to_string()),
            host_peer: Mutex::new("Keine aktive Sitzung".to_string()),
            viewer_status: Mutex::new(String::new()),
            frame: Mutex::new(None),
            remote_size: Mutex::new((1920, 1080)),
            remote_cursor: Mutex::new((0, 0, false)),
            input_tx: Mutex::new(None),
            clip_in: Mutex::new(None),
            clip_from_host: AtomicU32::new(0),
            connected: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            mode: AtomicU8::new(MODE_ADMIN),
            monitor: AtomicU8::new(0),
            monitors: Mutex::new(Vec::new()),
            active_monitor: AtomicU8::new(0),
            xfers: Mutex::new(Vec::new()),
            xfer: Mutex::new(None),
            drop_dir: Mutex::new(crate::xfer::default_dir()),
            direct: AtomicBool::new(false),
            stats: Mutex::new(Stats::default()),
        }
    }

    pub fn set_host_status(&self, s: impl Into<String>) {
        *self.host_status.lock().unwrap() = s.into();
    }
    pub fn set_viewer_status(&self, s: impl Into<String>) {
        *self.viewer_status.lock().unwrap() = s.into();
    }
    pub fn send_input(&self, m: Msg) {
        if let Some(tx) = self.input_tx.lock().unwrap().as_ref() {
            let _ = tx.send(m);
        }
    }
    pub fn game_mode(&self) -> bool {
        self.mode.load(Ordering::Relaxed) == crate::proto::MODE_GAME
    }
}
