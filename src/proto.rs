//! FreeViewer session protocol.
//!
//! Every message below travels INSIDE the AES-256-GCM channel that host and
//! viewer establish directly with each other. The relay only ever sees opaque
//! ciphertext.
//!
//! Mouse coordinates are normalized to 0..10000 (per-ten-thousand of the
//! remote screen) so that scaling/downsampling of the video stream never
//! affects pointer accuracy.

#[derive(Debug, Clone)]
pub enum Msg {
    /// Real (unscaled) size of the shared screen.
    ScreenInfo { width: u32, height: u32 },
    /// One JPEG encoded frame (may be downscaled).
    Frame { width: u32, height: u32, jpeg: Vec<u8> },
    /// Normalized 0..10000 pointer position.
    MouseMove { x: i32, y: i32 },
    /// button: 0 = left, 1 = right, 2 = middle
    MouseButton { button: u8, down: bool },
    /// positive = scroll up
    Wheel { lines: i32 },
    /// named = true  -> code is one of the KEY_* constants below
    /// named = false -> code is a unicode scalar value
    Key { code: u32, named: bool, down: bool },
    Ping { ts: u64 },
    Pong { ts: u64 },
}

pub const KEY_BACKSPACE: u32 = 1;
pub const KEY_ENTER: u32 = 2;
pub const KEY_TAB: u32 = 3;
pub const KEY_ESCAPE: u32 = 4;
pub const KEY_LEFT: u32 = 5;
pub const KEY_RIGHT: u32 = 6;
pub const KEY_UP: u32 = 7;
pub const KEY_DOWN: u32 = 8;
pub const KEY_DELETE: u32 = 9;
pub const KEY_HOME: u32 = 10;
pub const KEY_END: u32 = 11;
pub const KEY_PAGEUP: u32 = 12;
pub const KEY_PAGEDOWN: u32 = 13;
pub const KEY_INSERT: u32 = 14;
pub const KEY_SPACE: u32 = 15;
pub const KEY_SHIFT: u32 = 20;
pub const KEY_CTRL: u32 = 21;
pub const KEY_ALT: u32 = 22;
pub const KEY_META: u32 = 23;
pub const KEY_F1: u32 = 30; // F1..F12 => 30..41

const T_SCREEN: u8 = 0x20;
const T_FRAME: u8 = 0x21;
const T_MOVE: u8 = 0x30;
const T_BUTTON: u8 = 0x31;
const T_WHEEL: u8 = 0x32;
const T_KEY: u8 = 0x33;
const T_PING: u8 = 0x40;
const T_PONG: u8 = 0x41;

fn pu32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn pi32(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn pu64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

pub fn encode(m: &Msg) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::with_capacity(32);
    match m {
        Msg::ScreenInfo { width, height } => {
            v.push(T_SCREEN);
            pu32(&mut v, *width);
            pu32(&mut v, *height);
        }
        Msg::Frame {
            width,
            height,
            jpeg,
        } => {
            v.reserve(jpeg.len() + 16);
            v.push(T_FRAME);
            pu32(&mut v, *width);
            pu32(&mut v, *height);
            pu32(&mut v, jpeg.len() as u32);
            v.extend_from_slice(jpeg);
        }
        Msg::MouseMove { x, y } => {
            v.push(T_MOVE);
            pi32(&mut v, *x);
            pi32(&mut v, *y);
        }
        Msg::MouseButton { button, down } => {
            v.push(T_BUTTON);
            v.push(*button);
            v.push(if *down { 1 } else { 0 });
        }
        Msg::Wheel { lines } => {
            v.push(T_WHEEL);
            pi32(&mut v, *lines);
        }
        Msg::Key { code, named, down } => {
            v.push(T_KEY);
            pu32(&mut v, *code);
            v.push(if *named { 1 } else { 0 });
            v.push(if *down { 1 } else { 0 });
        }
        Msg::Ping { ts } => {
            v.push(T_PING);
            pu64(&mut v, *ts);
        }
        Msg::Pong { ts } => {
            v.push(T_PONG);
            pu64(&mut v, *ts);
        }
    }
    v
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Option<u8> {
        let x = *self.b.get(self.p)?;
        self.p += 1;
        Some(x)
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.p..self.p + 4)?;
        self.p += 4;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|x| x as i32)
    }
    fn u64(&mut self) -> Option<u64> {
        let s = self.b.get(self.p..self.p + 8)?;
        self.p += 8;
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        Some(u64::from_le_bytes(a))
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
}

pub fn decode(b: &[u8]) -> Option<Msg> {
    let mut r = Rd { b, p: 0 };
    let tag = r.u8()?;
    match tag {
        T_SCREEN => Some(Msg::ScreenInfo {
            width: r.u32()?,
            height: r.u32()?,
        }),
        T_FRAME => {
            let width = r.u32()?;
            let height = r.u32()?;
            let n = r.u32()? as usize;
            let data = r.take(n)?;
            Some(Msg::Frame {
                width,
                height,
                jpeg: data.to_vec(),
            })
        }
        T_MOVE => Some(Msg::MouseMove {
            x: r.i32()?,
            y: r.i32()?,
        }),
        T_BUTTON => Some(Msg::MouseButton {
            button: r.u8()?,
            down: r.u8()? != 0,
        }),
        T_WHEEL => Some(Msg::Wheel { lines: r.i32()? }),
        T_KEY => Some(Msg::Key {
            code: r.u32()?,
            named: r.u8()? != 0,
            down: r.u8()? != 0,
        }),
        T_PING => Some(Msg::Ping { ts: r.u64()? }),
        T_PONG => Some(Msg::Pong { ts: r.u64()? }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let msgs = vec![
            Msg::ScreenInfo {
                width: 2560,
                height: 1440,
            },
            Msg::Frame {
                width: 320,
                height: 200,
                jpeg: vec![1, 2, 3, 4, 5],
            },
            Msg::MouseMove { x: -7, y: 9999 },
            Msg::MouseButton {
                button: 2,
                down: true,
            },
            Msg::Wheel { lines: -3 },
            Msg::Key {
                code: 65,
                named: false,
                down: true,
            },
            Msg::Ping { ts: 1234567890 },
        ];
        for m in msgs {
            let enc = encode(&m);
            let dec = decode(&enc).expect("decode");
            assert_eq!(format!("{:?}", m), format!("{:?}", dec));
        }
        assert!(decode(&[]).is_none());
        assert!(decode(&[0x21, 1, 2]).is_none());
    }
}
