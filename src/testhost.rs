//! Synthetic test host (env `FV_TESTHOST=1`).
//!
//! Replaces screen capture and input injection of the host with a fixed
//! 640x480 picture (colour gradient + text) so that a viewer can be tested on
//! a machine without a display (CI server, QEMU guest talking to 10.0.2.2).
//! The whole handshake, the relay, the encryption and the message framing are
//! the real ones; only the "screen" and the "keyboard/mouse" are fake.
//!
//! What it does once a session is live:
//!   * sends `ScreenInfo`, `Monitors`, `Resolutions`, one JPEG `Frame`
//!   * every second sends a `Tiles` message with one tile (counter "T:NN")
//!   * logs every input message as one line `TESTHOST input: ...`
//!     (stdout, and appended to the file in `FV_TESTHOST_LOG` if set)
//!   * mouse button down -> a 16x16 coloured square `Tiles` at the pointer
//!   * clipboard text from the viewer is logged and echoed back as `echo:<text>`
//!   * `NeedKeyframe` (or Caps change) -> a fresh full `Frame`
//! It never sends `Video`, so a viewer sees a JPEG-only host.
//! Without `FV_TESTHOST=1` nothing here is used.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::proto::{self, encode, Msg, Tile};

pub const W: u32 = 640;
pub const H: u32 = 480;
const QUALITY: u8 = 80;
const COUNTER_X: u32 = 16;
const COUNTER_Y: u32 = 432;
const COUNTER_W: u32 = 160;
const COUNTER_H: u32 = 36;
const MARK: u32 = 16;

pub fn enabled() -> bool {
    std::env::var("FV_TESTHOST").ok().as_deref() == Some("1")
}

/// 5x7 glyph rows (bit 4 = leftmost pixel) for the few characters we draw.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        ':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        ' ' => [0; 7],
        _ => return None,
    })
}

fn put(buf: &mut [u8], x: u32, y: u32, rgb: [u8; 3]) {
    if x < W && y < H {
        let o = ((y * W + x) * 3) as usize;
        buf[o..o + 3].copy_from_slice(&rgb);
    }
}

/// Draws `text` at (x0,y0), every font pixel becomes a `scale` x `scale` block.
fn text(buf: &mut [u8], x0: u32, y0: u32, scale: u32, s: &str, rgb: [u8; 3]) {
    let mut cx = x0;
    for c in s.chars() {
        if let Some(rows) = glyph(c) {
            for (ry, row) in rows.iter().enumerate() {
                for col in 0..5u32 {
                    if row & (1 << (4 - col)) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                put(buf, cx + col * scale + dx, y0 + ry as u32 * scale + dy, rgb);
                            }
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

/// The whole fake screen as RGB. `counter` is shown in the bottom-left box.
pub fn scene(counter: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (W * H * 3) as usize];
    for y in 0..H {
        for x in 0..W {
            put(
                &mut buf,
                x,
                y,
                [(x * 255 / (W - 1)) as u8, (y * 255 / (H - 1)) as u8, 128],
            );
        }
    }
    text(&mut buf, 42, 62, 4, "FREEVIEWER", [0, 0, 0]);
    text(&mut buf, 40, 60, 4, "FREEVIEWER", [255, 255, 255]);
    text(&mut buf, 42, 122, 4, "TESTHOST", [0, 0, 0]);
    text(&mut buf, 40, 120, 4, "TESTHOST", [255, 255, 255]);
    text(&mut buf, 40, 200, 3, "640X480 JPEG", [255, 255, 255]);
    // counter box: black background so the tile always looks the same
    for y in COUNTER_Y..COUNTER_Y + COUNTER_H {
        for x in COUNTER_X..COUNTER_X + COUNTER_W {
            put(&mut buf, x, y, [0, 0, 0]);
        }
    }
    text(&mut buf, COUNTER_X + 6, COUNTER_Y + 4, 4, &format!("T:{}", counter % 100), [255, 255, 0]);
    buf
}

fn crop(rgb: &[u8], x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for row in y..y + h {
        let o = ((row * W + x) * 3) as usize;
        out.extend_from_slice(&rgb[o..o + (w * 3) as usize]);
    }
    out
}

fn frame_msg(counter: u32) -> Option<Msg> {
    let rgb = scene(counter);
    let jpeg = crate::encoder::jpeg_rgb(&rgb, W, H, QUALITY)?;
    Some(Msg::Frame {
        width: W,
        height: H,
        jpeg,
    })
}

fn counter_tile(counter: u32) -> Option<Msg> {
    let rgb = crop(&scene(counter), COUNTER_X, COUNTER_Y, COUNTER_W, COUNTER_H);
    let jpeg = crate::encoder::jpeg_rgb(&rgb, COUNTER_W, COUNTER_H, QUALITY)?;
    Some(Msg::Tiles {
        width: W,
        height: H,
        tiles: vec![Tile {
            x: COUNTER_X,
            y: COUNTER_Y,
            w: COUNTER_W,
            h: COUNTER_H,
            jpeg,
        }],
    })
}

fn mark_tile(px: u32, py: u32, button: u8) -> Option<Msg> {
    let x = px.saturating_sub(MARK / 2).min(W - MARK);
    let y = py.saturating_sub(MARK / 2).min(H - MARK);
    let color: [u8; 3] = match button {
        0 => [255, 0, 0],
        1 => [0, 255, 0],
        _ => [0, 0, 255],
    };
    let mut rgb = Vec::with_capacity((MARK * MARK * 3) as usize);
    for _ in 0..MARK * MARK {
        rgb.extend_from_slice(&color);
    }
    let jpeg = crate::encoder::jpeg_rgb(&rgb, MARK, MARK, QUALITY)?;
    Some(Msg::Tiles {
        width: W,
        height: H,
        tiles: vec![Tile {
            x,
            y,
            w: MARK,
            h: MARK,
            jpeg,
        }],
    })
}

fn log_line(line: &str) {
    println!("TESTHOST input: {}", line);
    if let Ok(path) = std::env::var("FV_TESTHOST_LOG") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{}", line);
        }
    }
}

fn send(out: &mpsc::UnboundedSender<Vec<u8>>, m: Option<Msg>) -> bool {
    match m {
        Some(m) => out.send(encode(&m)).is_ok(),
        None => true,
    }
}

/// Runs the fake screen for one session on its own thread until `stop` is set.
pub fn spawn(
    stop: Arc<AtomicBool>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    input: std::sync::mpsc::Receiver<Msg>,
    force_key: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut counter = 0u32;
        let (mut px, mut py) = (W / 2, H / 2);
        let _ = send(&out, Some(Msg::ScreenInfo { width: W, height: H }));
        let _ = send(
            &out,
            Some(Msg::Monitors {
                active: 0,
                list: vec![proto::MonitorInfo {
                    name: "TESTHOST".to_string(),
                    w: W,
                    h: H,
                    primary: true,
                }],
            }),
        );
        let _ = send(&out, Some(Msg::Resolutions { list: vec![(W, H)] }));
        let _ = send(&out, frame_msg(counter));
        println!("TESTHOST session live: {}x{} JPEG", W, H);
        let mut last_tick = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            match input.recv_timeout(Duration::from_millis(50)) {
                Ok(m) => match m {
                    Msg::MouseMove { x, y } => {
                        log_line(&format!("MouseMove x={} y={}", x, y));
                        px = (x.clamp(0, 10000) as u32 * (W - 1)) / 10000;
                        py = (y.clamp(0, 10000) as u32 * (H - 1)) / 10000;
                        let _ = send(
                            &out,
                            Some(Msg::Cursor {
                                x,
                                y,
                                visible: true,
                            }),
                        );
                    }
                    Msg::MouseDelta { dx, dy } => log_line(&format!("MouseDelta dx={} dy={}", dx, dy)),
                    Msg::MouseButton { button, down } => {
                        log_line(&format!("MouseButton button={} down={}", button, down));
                        if down && !send(&out, mark_tile(px, py, button)) {
                            break;
                        }
                    }
                    Msg::Wheel { lines } => log_line(&format!("Wheel lines={}", lines)),
                    Msg::Key { code, named, down } => {
                        log_line(&format!("Key code={} named={} down={}", code, named, down))
                    }
                    Msg::KeyVk { vk, ext, down } => {
                        log_line(&format!("KeyVk vk={} ext={} down={}", vk, ext, down))
                    }
                    Msg::Special { code } => log_line(&format!("Special code={}", code)),
                    Msg::Clipboard { text } => {
                        log_line(&format!("Clipboard len={} text={}", text.len(), text));
                        let echo = Msg::Clipboard {
                            text: format!("echo:{}", text),
                        };
                        if !send(&out, Some(echo)) {
                            break;
                        }
                    }
                    other => log_line(&format!("other {:?}", other)),
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if force_key.swap(false, Ordering::Relaxed) && !send(&out, frame_msg(counter)) {
                break;
            }
            if last_tick.elapsed() >= Duration::from_secs(1) {
                last_tick = Instant::now();
                counter += 1;
                if !send(&out, counter_tile(counter)) {
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_is_deterministic_and_encodes() {
        assert_eq!(scene(3), scene(3));
        assert_ne!(scene(3), scene(4));
        match frame_msg(0) {
            Some(Msg::Frame { width, height, jpeg }) => {
                assert_eq!((width, height), (W, H));
                assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
            }
            _ => panic!("frame"),
        }
        assert!(matches!(counter_tile(1), Some(Msg::Tiles { .. })));
        assert!(matches!(mark_tile(0, 0, 0), Some(Msg::Tiles { .. })));
        assert!(matches!(mark_tile(639, 479, 2), Some(Msg::Tiles { .. })));
    }
}
