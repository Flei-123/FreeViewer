//! State shared between the GUI thread and the async worker tasks.

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tokio::sync::mpsc::UnboundedSender;

use crate::proto::Msg;

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
    pub input_tx: Mutex<Option<UnboundedSender<Msg>>>,
    pub connected: AtomicBool,
    pub connecting: AtomicBool,
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
            input_tx: Mutex::new(None),
            connected: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
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
}
