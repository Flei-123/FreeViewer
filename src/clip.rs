//! Clipboard synchronisation (text) for both sides of a session.
//!
//! Both ends poll their own clipboard. When it changed, the new text is sent
//! to the peer, which writes it into its own clipboard. `last` remembers the
//! text we saw or wrote ourselves, so the two machines cannot ping-pong the
//! same string forever.

use std::time::{Duration, Instant};

pub struct Clip {
    cb: Option<arboard::Clipboard>,
    last: String,
    next_poll: Instant,
}

impl Clip {
    pub fn new() -> Self {
        Self {
            cb: arboard::Clipboard::new().ok(),
            last: String::new(),
            next_poll: Instant::now(),
        }
    }

    pub fn available(&self) -> bool {
        self.cb.is_some()
    }

    /// Returns the local clipboard text when it changed since the last call.
    /// Rate limited internally, so it can be called from a hot loop.
    pub fn poll(&mut self) -> Option<String> {
        if Instant::now() < self.next_poll {
            return None;
        }
        self.next_poll = Instant::now() + Duration::from_millis(600);
        let cb = self.cb.as_mut()?;
        // an empty or non-text clipboard is not an error worth logging
        let text = cb.get_text().ok()?;
        if text.is_empty() || text == self.last {
            return None;
        }
        if text.len() > crate::proto::MAX_CLIP {
            return None;
        }
        self.last = text.clone();
        Some(text)
    }

    /// Writes text coming from the peer into the local clipboard. Returns
    /// false when nothing had to be done (same text) or when it failed.
    pub fn set(&mut self, text: &str) -> bool {
        if text == self.last {
            return false;
        }
        self.last = text.to_string();
        match self.cb.as_mut() {
            Some(cb) => match cb.set_text(text.to_string()) {
                Ok(()) => true,
                Err(e) => {
                    crate::dbg_line(&format!("Zwischenablage schreiben fehlgeschlagen: {:?}", e));
                    false
                }
            },
            None => false,
        }
    }
}
