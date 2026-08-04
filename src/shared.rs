//! State shared between the GUI thread and the async worker tasks.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
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

/// Somebody is knocking: a viewer without a password wants in and the person
/// sitting here has to allow it.
#[derive(Clone, Debug)]
pub struct Knock {
    /// what the other side calls itself
    pub from: String,
    /// four digits both sides can compare
    pub code: String,
    pub at: std::time::Instant,
}

pub struct Shared {
    /// Was der ferne Bildschirm an Aufloesungen wirklich kann (kommt vom Host).
    pub remote_resolutions: Mutex<Vec<(u32, u32)>>,
    pub relay_url: String,
    // host side
    pub my_id: Mutex<String>,
    pub password: Mutex<String>,
    pub host_status: Mutex<String>,
    pub host_peer: Mutex<String>,
    /// Name of this machine in other people's lists.
    pub device_name: Mutex<String>,
    /// A viewer is waiting to be let in (host side).
    pub knock: Mutex<Option<Knock>>,
    /// 0 = still deciding, 1 = allowed, 2 = refused.
    pub knock_answer: AtomicU8,
    /// Session code of the running session, shown on both sides.
    pub session_code: Mutex<String>,
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
    /// Pictures the H.264 worker has decoded (feeds the fps counter).
    pub video_frames: AtomicU32,
    /// Bytes that arrived on the direct UDP path (feeds the bitrate counter).
    pub video_bytes: AtomicU64,
    /// Pictures that took the direct UDP path instead of the relay.
    pub udp_frames: AtomicU64,
    /// Monotonic picture counter. Both the JPEG canvas and the video worker
    /// draw their sequence numbers from here, so the GUI always sees a single
    /// increasing series no matter which codec is active.
    pub frame_seq: AtomicU64,
    pub connected: AtomicBool,
    pub connecting: AtomicBool,
    /// Active session profile (MODE_ADMIN / MODE_GAME).
    pub mode: AtomicU8,
    /// The host key (right Ctrl) was pressed: 1 = release the input,
    /// 2 = leave the session entirely. The GUI picks this up and resets it.
    pub escape: AtomicU8,
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
    /// A newer build waiting on the relay.
    pub update: Mutex<Option<crate::update::Release>>,
    pub update_status: Mutex<String>,
    pub auto_update: AtomicBool,
    /// Microphone/speaker of the running session (voice link).
    pub voice: std::sync::Arc<crate::audio::VoiceState>,    /// Zwischenablage in beide Richtungen abgleichen?
    pub clip_on: AtomicBool,
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
            device_name: Mutex::new(crate::presence::device_name()),
            knock: Mutex::new(None),
            knock_answer: AtomicU8::new(0),
            session_code: Mutex::new(String::new()),
            viewer_status: Mutex::new(String::new()),
            frame: Mutex::new(None),
            remote_size: Mutex::new((1920, 1080)),
            remote_resolutions: Mutex::new(Vec::new()),
            remote_cursor: Mutex::new((0, 0, false)),
            input_tx: Mutex::new(None),
            clip_in: Mutex::new(None),
            clip_from_host: AtomicU32::new(0),
            video_frames: AtomicU32::new(0),
            video_bytes: AtomicU64::new(0),
            udp_frames: AtomicU64::new(0),
            frame_seq: AtomicU64::new(0),
            connected: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            mode: AtomicU8::new(MODE_ADMIN),
            escape: AtomicU8::new(0),
            monitor: AtomicU8::new(0),
            monitors: Mutex::new(Vec::new()),
            active_monitor: AtomicU8::new(0),
            xfers: Mutex::new(Vec::new()),
            xfer: Mutex::new(None),
            drop_dir: Mutex::new(crate::xfer::default_dir()),
            direct: AtomicBool::new(false),
            update: Mutex::new(None),
            update_status: Mutex::new(String::new()),
            auto_update: AtomicBool::new(crate::ident::auto_update_enabled()),
            voice: std::sync::Arc::new(crate::audio::VoiceState::default()),            clip_on: AtomicBool::new(crate::ident::clipboard_enabled()),
            stats: Mutex::new(Stats::default()),
        }
    }

    pub fn set_host_status(&self, s: impl Into<String>) {
        *self.host_status.lock().unwrap() = s.into();
    }
    pub fn set_update_status(&self, s: impl Into<String>) {
        *self.update_status.lock().unwrap() = s.into();
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

impl Shared {
    /// Next sequence number for a freshly decoded picture.
    pub fn next_frame_seq(&self) -> u64 {
        self.frame_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }
}