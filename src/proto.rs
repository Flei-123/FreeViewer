//! FreeViewer session protocol.
//!
//! Every message below travels INSIDE the AES-256-GCM channel that host and
//! viewer establish directly with each other. The relay only ever sees opaque
//! ciphertext.
//!
//! Mouse coordinates are normalized to 0..10000 (per-ten-thousand of the
//! remote screen) so that scaling/downsampling of the video stream never
//! affects pointer accuracy. In game mode the viewer additionally sends raw
//! *relative* motion, which is what 3D games read through raw input.

/// One monitor the host is able to share.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub primary: bool,
}

/// One changed rectangle of the current frame (delta update).
#[derive(Debug, Clone)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub jpeg: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Real (unscaled) size of the shared screen.
    ScreenInfo { width: u32, height: u32 },
    /// One JPEG encoded full frame / keyframe (may be downscaled).
    Frame {
        width: u32,
        height: u32,
        jpeg: Vec<u8>,
    },
    /// Only the parts of the frame that changed since the previous one.
    /// `width`/`height` describe the full frame the tiles belong to, so the
    /// viewer can detect a stale canvas and wait for the next keyframe.
    Tiles {
        width: u32,
        height: u32,
        tiles: Vec<Tile>,
    },
    /// Where the remote mouse pointer is (normalized), so the viewer can draw
    /// it - the duplication API never renders the cursor into the frame.
    Cursor { x: i32, y: i32, visible: bool },
    /// Normalized 0..10000 pointer position.
    MouseMove { x: i32, y: i32 },
    /// Raw relative mouse motion in remote pixels (game mode).
    MouseDelta { dx: i32, dy: i32 },
    /// button: 0 = left, 1 = right, 2 = middle, 3 = back, 4 = forward
    MouseButton { button: u8, down: bool },
    /// positive = scroll up
    Wheel { lines: i32 },
    /// named = true  -> code is one of the KEY_* constants below
    /// named = false -> code is a unicode scalar value
    Key { code: u32, named: bool, down: bool },
    /// Raw Windows virtual key code including the extended-key flag. This is
    /// what the low level keyboard hook of the viewer produces, so every
    /// shortcut (Win, Alt+Tab, Alt+F4, AltGr, ...) survives the trip.
    KeyVk { vk: u16, ext: bool, down: bool },
    /// Key combinations that cannot simply be injected (secure attention
    /// sequence and friends).
    Special { code: u8 },
    /// Clipboard text of the sender changed.
    Clipboard { text: String },
    /// Viewer asks the host to switch its capture/input profile.
    SetMode { mode: u8 },
    /// Host announces every screen it could share plus the active one.
    Monitors { active: u8, list: Vec<MonitorInfo> },
    /// Viewer picks the screen to capture (index into the list above).
    SetMonitor { index: u8 },
    /// Start of a file transfer. Works in both directions.
    FileOffer { id: u32, name: String, size: u64 },
    /// One piece of the file at byte offset `off`.
    FileChunk { id: u32, off: u64, data: Vec<u8> },
    /// Transfer finished (`ok`) or was aborted (reason in `msg`).
    FileEnd { id: u32, ok: bool, msg: String },
    /// Receiver confirms how many bytes it has written (flow control).
    FileAck { id: u32, got: u64 },
    /// Viewer/host offers a UDP endpoint for a direct peer to peer path.
    P2pOffer { token: u64, addrs: Vec<String> },
    /// The direct path is up (or down again) - purely informational.
    P2pState { direct: bool, rtt_ms: u32 },
    /// The video path lost data, please send a full frame.
    NeedKeyframe,
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

/// Remote maintenance: sharp picture, absolute mouse, host cursor drawn.
pub const MODE_ADMIN: u8 = 0;
/// Gaming/3D: relative mouse, full keyboard grab, more fps, smaller picture.
pub const MODE_GAME: u8 = 1;

pub const SPECIAL_CAD: u8 = 1; // Ctrl+Alt+Del
pub const SPECIAL_TASKMGR: u8 = 2; // Ctrl+Shift+Esc
pub const SPECIAL_WIN: u8 = 3; // Windows key tap
pub const SPECIAL_ALTTAB: u8 = 4; // Alt+Tab
pub const SPECIAL_LOCK: u8 = 5; // Win+L
pub const SPECIAL_RELEASE: u8 = 6; // let go of everything the viewer still holds

const T_SCREEN: u8 = 0x20;
const T_FRAME: u8 = 0x21;
const T_TILES: u8 = 0x22;
const T_CURSOR: u8 = 0x23;
const T_MOVE: u8 = 0x30;
const T_BUTTON: u8 = 0x31;
const T_WHEEL: u8 = 0x32;
const T_KEY: u8 = 0x33;
const T_DELTA: u8 = 0x34;
const T_KEYVK: u8 = 0x35;
const T_SPECIAL: u8 = 0x36;
const T_CLIP: u8 = 0x37;
const T_MODE: u8 = 0x38;
const T_MONS: u8 = 0x24;
const T_SETMON: u8 = 0x39;
const T_FOFFER: u8 = 0x50;
const T_FCHUNK: u8 = 0x51;
const T_FEND: u8 = 0x52;
const T_FACK: u8 = 0x53;
const T_P2P: u8 = 0x60;
const T_P2PST: u8 = 0x61;
const T_NEEDKEY: u8 = 0x62;
const T_PING: u8 = 0x40;
const T_PONG: u8 = 0x41;

/// Hard limit so a corrupt/hostile message cannot make us allocate wildly.
const MAX_TILES: usize = 4096;
/// Clipboard transfers are capped so a peer cannot exhaust our memory.
pub const MAX_CLIP: usize = 256 * 1024;
/// Upper bound for one file chunk on the wire.
pub const MAX_CHUNK: usize = 512 * 1024;
/// Bytes per chunk we actually send.
pub const CHUNK: usize = 64 * 1024;
/// A file name never needs more than this.
pub const MAX_NAME: usize = 512;
const MAX_MONITORS: usize = 32;
const MAX_ADDRS: usize = 8;

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
        Msg::Tiles {
            width,
            height,
            tiles,
        } => {
            let total: usize = tiles.iter().map(|t| t.jpeg.len() + 20).sum();
            v.reserve(total + 16);
            v.push(T_TILES);
            pu32(&mut v, *width);
            pu32(&mut v, *height);
            pu32(&mut v, tiles.len() as u32);
            for t in tiles {
                pu32(&mut v, t.x);
                pu32(&mut v, t.y);
                pu32(&mut v, t.w);
                pu32(&mut v, t.h);
                pu32(&mut v, t.jpeg.len() as u32);
                v.extend_from_slice(&t.jpeg);
            }
        }
        Msg::Cursor { x, y, visible } => {
            v.push(T_CURSOR);
            pi32(&mut v, *x);
            pi32(&mut v, *y);
            v.push(if *visible { 1 } else { 0 });
        }
        Msg::MouseMove { x, y } => {
            v.push(T_MOVE);
            pi32(&mut v, *x);
            pi32(&mut v, *y);
        }
        Msg::MouseDelta { dx, dy } => {
            v.push(T_DELTA);
            pi32(&mut v, *dx);
            pi32(&mut v, *dy);
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
        Msg::KeyVk { vk, ext, down } => {
            v.push(T_KEYVK);
            v.extend_from_slice(&vk.to_le_bytes());
            v.push(if *ext { 1 } else { 0 });
            v.push(if *down { 1 } else { 0 });
        }
        Msg::Special { code } => {
            v.push(T_SPECIAL);
            v.push(*code);
        }
        Msg::Clipboard { text } => {
            let b = text.as_bytes();
            let n = b.len().min(MAX_CLIP);
            v.reserve(n + 8);
            v.push(T_CLIP);
            pu32(&mut v, n as u32);
            v.extend_from_slice(&b[..n]);
        }
        Msg::SetMode { mode } => {
            v.push(T_MODE);
            v.push(*mode);
        }
        Msg::Monitors { active, list } => {
            v.push(T_MONS);
            v.push(*active);
            pu32(&mut v, list.len().min(MAX_MONITORS) as u32);
            for m in list.iter().take(MAX_MONITORS) {
                let nb = m.name.as_bytes();
                let n = nb.len().min(MAX_NAME);
                pu32(&mut v, n as u32);
                v.extend_from_slice(&nb[..n]);
                pu32(&mut v, m.w);
                pu32(&mut v, m.h);
                v.push(if m.primary { 1 } else { 0 });
            }
        }
        Msg::SetMonitor { index } => {
            v.push(T_SETMON);
            v.push(*index);
        }
        Msg::FileOffer { id, name, size } => {
            let nb = name.as_bytes();
            let n = nb.len().min(MAX_NAME);
            v.push(T_FOFFER);
            pu32(&mut v, *id);
            pu64(&mut v, *size);
            pu32(&mut v, n as u32);
            v.extend_from_slice(&nb[..n]);
        }
        Msg::FileChunk { id, off, data } => {
            let n = data.len().min(MAX_CHUNK);
            v.reserve(n + 20);
            v.push(T_FCHUNK);
            pu32(&mut v, *id);
            pu64(&mut v, *off);
            pu32(&mut v, n as u32);
            v.extend_from_slice(&data[..n]);
        }
        Msg::FileEnd { id, ok, msg } => {
            let mb = msg.as_bytes();
            let n = mb.len().min(MAX_NAME);
            v.push(T_FEND);
            pu32(&mut v, *id);
            v.push(if *ok { 1 } else { 0 });
            pu32(&mut v, n as u32);
            v.extend_from_slice(&mb[..n]);
        }
        Msg::FileAck { id, got } => {
            v.push(T_FACK);
            pu32(&mut v, *id);
            pu64(&mut v, *got);
        }
        Msg::P2pOffer { token, addrs } => {
            v.push(T_P2P);
            pu64(&mut v, *token);
            pu32(&mut v, addrs.len().min(MAX_ADDRS) as u32);
            for a in addrs.iter().take(MAX_ADDRS) {
                let ab = a.as_bytes();
                let n = ab.len().min(64);
                pu32(&mut v, n as u32);
                v.extend_from_slice(&ab[..n]);
            }
        }
        Msg::P2pState { direct, rtt_ms } => {
            v.push(T_P2PST);
            v.push(if *direct { 1 } else { 0 });
            pu32(&mut v, *rtt_ms);
        }
        Msg::NeedKeyframe => {
            v.push(T_NEEDKEY);
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
    fn u16(&mut self) -> Option<u16> {
        let s = self.b.get(self.p..self.p + 2)?;
        self.p += 2;
        Some(u16::from_le_bytes([s[0], s[1]]))
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
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
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
        T_TILES => {
            let width = r.u32()?;
            let height = r.u32()?;
            let count = r.u32()? as usize;
            if count > MAX_TILES {
                return None;
            }
            let mut tiles = Vec::with_capacity(count.min(256));
            for _ in 0..count {
                let x = r.u32()?;
                let y = r.u32()?;
                let w = r.u32()?;
                let h = r.u32()?;
                let n = r.u32()? as usize;
                let data = r.take(n)?;
                tiles.push(Tile {
                    x,
                    y,
                    w,
                    h,
                    jpeg: data.to_vec(),
                });
            }
            Some(Msg::Tiles {
                width,
                height,
                tiles,
            })
        }
        T_CURSOR => Some(Msg::Cursor {
            x: r.i32()?,
            y: r.i32()?,
            visible: r.u8()? != 0,
        }),
        T_MOVE => Some(Msg::MouseMove {
            x: r.i32()?,
            y: r.i32()?,
        }),
        T_DELTA => Some(Msg::MouseDelta {
            dx: r.i32()?,
            dy: r.i32()?,
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
        T_KEYVK => Some(Msg::KeyVk {
            vk: r.u16()?,
            ext: r.u8()? != 0,
            down: r.u8()? != 0,
        }),
        T_SPECIAL => Some(Msg::Special { code: r.u8()? }),
        T_CLIP => {
            let n = r.u32()? as usize;
            if n > MAX_CLIP {
                return None;
            }
            let data = r.take(n)?;
            Some(Msg::Clipboard {
                text: String::from_utf8_lossy(data).into_owned(),
            })
        }
        T_MODE => Some(Msg::SetMode { mode: r.u8()? }),
        T_MONS => {
            let active = r.u8()?;
            let count = r.u32()? as usize;
            if count > MAX_MONITORS {
                return None;
            }
            let mut list = Vec::with_capacity(count);
            for _ in 0..count {
                let n = r.u32()? as usize;
                if n > MAX_NAME {
                    return None;
                }
                let name = String::from_utf8_lossy(r.take(n)?).into_owned();
                let w = r.u32()?;
                let h = r.u32()?;
                let primary = r.u8()? != 0;
                list.push(MonitorInfo { name, w, h, primary });
            }
            Some(Msg::Monitors { active, list })
        }
        T_SETMON => Some(Msg::SetMonitor { index: r.u8()? }),
        T_FOFFER => {
            let id = r.u32()?;
            let size = r.u64()?;
            let n = r.u32()? as usize;
            if n > MAX_NAME {
                return None;
            }
            let name = String::from_utf8_lossy(r.take(n)?).into_owned();
            Some(Msg::FileOffer { id, name, size })
        }
        T_FCHUNK => {
            let id = r.u32()?;
            let off = r.u64()?;
            let n = r.u32()? as usize;
            if n > MAX_CHUNK {
                return None;
            }
            let data = r.take(n)?.to_vec();
            Some(Msg::FileChunk { id, off, data })
        }
        T_FEND => {
            let id = r.u32()?;
            let ok = r.u8()? != 0;
            let n = r.u32()? as usize;
            if n > MAX_NAME {
                return None;
            }
            let msg = String::from_utf8_lossy(r.take(n)?).into_owned();
            Some(Msg::FileEnd { id, ok, msg })
        }
        T_FACK => Some(Msg::FileAck {
            id: r.u32()?,
            got: r.u64()?,
        }),
        T_P2P => {
            let token = r.u64()?;
            let count = r.u32()? as usize;
            if count > MAX_ADDRS {
                return None;
            }
            let mut addrs = Vec::with_capacity(count);
            for _ in 0..count {
                let n = r.u32()? as usize;
                if n > 64 {
                    return None;
                }
                addrs.push(String::from_utf8_lossy(r.take(n)?).into_owned());
            }
            Some(Msg::P2pOffer { token, addrs })
        }
        T_P2PST => Some(Msg::P2pState {
            direct: r.u8()? != 0,
            rtt_ms: r.u32()?,
        }),
        T_NEEDKEY => Some(Msg::NeedKeyframe),
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
            Msg::Tiles {
                width: 1600,
                height: 670,
                tiles: vec![
                    Tile {
                        x: 0,
                        y: 64,
                        w: 64,
                        h: 128,
                        jpeg: vec![9, 8, 7],
                    },
                    Tile {
                        x: 1536,
                        y: 640,
                        w: 64,
                        h: 30,
                        jpeg: vec![1],
                    },
                ],
            },
            Msg::Cursor {
                x: 5000,
                y: 1,
                visible: true,
            },
            Msg::MouseMove { x: -7, y: 9999 },
            Msg::MouseDelta { dx: -12, dy: 40 },
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
            Msg::KeyVk {
                vk: 0x5B,
                ext: true,
                down: true,
            },
            Msg::Special { code: SPECIAL_CAD },
            Msg::Clipboard {
                text: "hallo welt \u{00e4}\u{00f6}\u{00fc}".to_string(),
            },
            Msg::SetMode { mode: MODE_GAME },
            Msg::Monitors {
                active: 1,
                list: vec![
                    MonitorInfo {
                        name: "\\\\.\\DISPLAY1".to_string(),
                        w: 2560,
                        h: 1440,
                        primary: true,
                    },
                    MonitorInfo {
                        name: "Parsec Virtual Display".to_string(),
                        w: 1920,
                        h: 1080,
                        primary: false,
                    },
                ],
            },
            Msg::SetMonitor { index: 2 },
            Msg::FileOffer {
                id: 7,
                name: "urlaub \u{00fc}ber alles.zip".to_string(),
                size: 4_294_967_400,
            },
            Msg::FileChunk {
                id: 7,
                off: 65536,
                data: vec![3, 1, 4, 1, 5],
            },
            Msg::FileEnd {
                id: 7,
                ok: false,
                msg: "abgebrochen".to_string(),
            },
            Msg::FileAck { id: 7, got: 131072 },
            Msg::P2pOffer {
                token: 0xdead_beef_1234,
                addrs: vec!["192.168.1.51:41234".to_string(), "84.115.1.2:41234".to_string()],
            },
            Msg::P2pState {
                direct: true,
                rtt_ms: 7,
            },
            Msg::NeedKeyframe,
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

    #[test]
    fn truncated_tiles_do_not_panic() {
        let full = encode(&Msg::Tiles {
            width: 100,
            height: 100,
            tiles: vec![Tile {
                x: 0,
                y: 0,
                w: 64,
                h: 64,
                jpeg: vec![1, 2, 3, 4],
            }],
        });
        for cut in 0..full.len() {
            let _ = decode(&full[..cut]);
        }
        // absurd tile count is rejected instead of allocating
        let mut evil = vec![T_TILES];
        evil.extend_from_slice(&100u32.to_le_bytes());
        evil.extend_from_slice(&100u32.to_le_bytes());
        evil.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&evil).is_none());
    }

    #[test]
    fn oversized_file_pieces_are_rejected() {
        let mut evil = vec![T_FCHUNK];
        evil.extend_from_slice(&1u32.to_le_bytes());
        evil.extend_from_slice(&0u64.to_le_bytes());
        evil.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&evil).is_none());

        let mut evil = vec![T_FOFFER];
        evil.extend_from_slice(&1u32.to_le_bytes());
        evil.extend_from_slice(&0u64.to_le_bytes());
        evil.extend_from_slice(&(MAX_NAME as u32 + 1).to_le_bytes());
        assert!(decode(&evil).is_none());

        let mut evil = vec![T_MONS, 0];
        evil.extend_from_slice(&(MAX_MONITORS as u32 + 1).to_le_bytes());
        assert!(decode(&evil).is_none());

        // truncation anywhere must not panic
        let full = encode(&Msg::Monitors {
            active: 0,
            list: vec![MonitorInfo {
                name: "x".to_string(),
                w: 1,
                h: 2,
                primary: true,
            }],
        });
        for cut in 0..full.len() {
            let _ = decode(&full[..cut]);
        }
    }

    #[test]
    fn clipboard_is_capped() {
        let mut evil = vec![T_CLIP];
        evil.extend_from_slice(&(u32::MAX).to_le_bytes());
        assert!(decode(&evil).is_none());
        // a too long text is truncated instead of being sent whole
        let big = "x".repeat(MAX_CLIP + 100);
        let enc = encode(&Msg::Clipboard { text: big });
        match decode(&enc) {
            Some(Msg::Clipboard { text }) => assert_eq!(text.len(), MAX_CLIP),
            _ => panic!("clipboard roundtrip"),
        }
    }
}
