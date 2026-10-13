# FreeViewer Viewer for OrientOS (plan, 12.10.2026)

Goal: Boss works on the Dell under OrientOS, reaches his Windows PC via FreeViewer (no dual boot).
Windows side = existing Rust host, unchanged. OrientOS side = new Firn viewer, installed from the OrientOS store.
Decision (project Fleitec All-in-One): FreeViewer/FreeMeet stay Rust, only the OrientOS viewer is Firn.
Status: PLAN ONLY, read from code (freeviewer main adf2b94, v0.29.4; osum main 075a95fb). Nothing built, nothing measured.

## 1. Protocol as it is today (source: src/crypto.rs, src/proto.rs, src/viewer.rs, src/net.rs, relay/relay.js)

### 1.1 Transport / relay
- WebSocket over TLS: `wss://freeviewer.fleitec.com/fv/ws` (net.rs; plain `ws://` is refused by the client).
- Relay is a dumb pipe. It only sees opaque frames.
- Relay control = JSON text frames: viewer sends `{"t":"connect","id":"<9 digits>"}`; relay answers `{"t":"paired"}`,
  `{"t":"error","msg":"offline"|"busy"|...}`, `{"t":"peer_gone"}`. Also `ping`, `bye`.
- After `paired`: binary WebSocket frames, forwarded 1:1 to the host.
- Optional direct UDP path (p2p.rs: STUN + hole punching, own HKDF key). Carries VIDEO ONLY.
  Not needed: relay path alone works (viewer simply never sends `P2pOffer`).

### 1.2 Handshake (crypto.rs header)
1. V->H `0x01 || client_pub(32)` (X25519, ephemeral)
2. H->V `0x02 || host_pub(32) || salt(16)`
3. Both: `key = HKDF-SHA256(ikm = X25519 shared, salt, info = "freeviewer-v1 session")`, 32 bytes.
4. Password mode: `pw_key = Argon2id(password, salt)` (argon2 crate 0.5.3 `Argon2::default()`, 32 B out), exact parameters
   (VERIFIED in the crate source `argon2-0.5.3/src/params.rs`, `algorithm.rs`, `version.rs`, `lib.rs`; pinned by a test in `src/vectors.rs`):
   - type **Argon2id** (`Algorithm::default()` = Argon2id), version **0x13** (19, `Version::default()` = V0x13)
   - memory **m = 19456 KiB** (`DEFAULT_M_COST = 19 * 1024`, i.e. 19 MiB = 19456 blocks of 1 KiB), iterations **t = 2**, parallelism **p = 1** (1 lane)
   - output length **32 bytes**; password = raw UTF-8 bytes; salt = the **16 raw bytes** from the host (NOT base64, NOT hex);
     no secret (pepper), no associated data, no key id
   - consequence for the viewer: ~19 MiB of working memory and 2 passes over it; same call as `hash_password_into(pw, salt, &mut out32)`.
   Test vectors: `tests/orientos-vectors.json` section `argon2` (ASCII, umlaut and long password, own salts).
   `proof = HMAC-SHA256(pw_key, "fv-auth" || client_pub || host_pub || salt)`. V->H `0x03 || proof(32)`.
5. Ask mode (no password): V->H `0x06 || device_name(utf8)`; host user clicks allow. Both show a 4-digit
   session code = `SHA256("freeviewer-v1 code" || key)[0..4] BE mod 10000` (human check against relay MITM).
6. H->V `0x04` = OK, `0x05` = wrong password.
7. Data: `0x10 || nonce(12) || AES-256-GCM(ct+tag)`. Nonce = `dir byte (host=1, viewer=2) || 3 zero bytes || counter u64 BE`,
   counter starts at 1 per direction. Receiver rejects counter <= last. No AAD.

### 1.3 Session messages (proto.rs, inside the encrypted channel)
- Framing: 1 type byte + little-endian fields; blobs = u32 len + bytes. Types = `T_*` in proto.rs.
- Host -> viewer: `ScreenInfo 0x20 (w,h)`, `Frame 0x21 (w,h,len,jpeg)`, `Tiles 0x22 (w,h,n,[x,y,w,h,len,jpeg])`,
  `Cursor 0x23`, `Monitors 0x24`, `Video 0x25 (w,h,key,data)` H.264 access unit, `Clipboard 0x37`, `Resolutions 0x3B`, `Ping/Pong 0x40/0x41`.
- Viewer -> host: `Caps 0x26 {h264}`, `MouseMove 0x30` (x,y normalized 0..10000), `MouseButton 0x31` (0 L,1 R,2 M),
  `Wheel 0x32`, `Key 0x33 {code,named,down}` (named = KEY_* 1..41, else unicode scalar), `KeyVk 0x35 {vk,ext,down}`,
  `Special 0x36` (Ctrl+Alt+Del, Win, Alt+Tab ...), `Clipboard 0x37` (max 256 KiB), `SetMonitor 0x39`, `NeedKeyframe 0x62`.
- FALLBACK: a viewer that never sends `Caps{h264:true}` only gets JPEG (`Frame` + `Tiles`). => OrientOS viewer starts JPEG-only. No H.264 needed for M1-M3.
- Limits: MAX_VIDEO 8 MiB, MAX_TILES 4096, MAX_CLIP 256 KiB.

### 1.4 Minimal viewer (Firn)
- MUST: wss connect, handshake (password or ask mode), AES-256-GCM both ways, JPEG `Frame`/`Tiles` display + scaling,
  mouse (normalized), keyboard (`Key` unicode + named; `KeyVk` for shortcuts), ping/pong, `peer_gone`.
- Later: clipboard text, monitor switch, `Special` keys, remote cursor.
- Skip: P2P/UDP, audio (IMA-ADPCM 24 kHz, 0x70), file transfer (0x50-0x53), game mode, H.264.

### 1.5 Test tooling for the viewer (M0, built 13.10.2026)
- **Vectors**: `tests/orientos-vectors.json` (format in its `_doc` field). Sections: standard RFC known answers (X25519, HKDF, HMAC, AES-GCM),
  handshake (fixed secrets -> shared secret, session key, udp key, session code, frames), Argon2id + proof (3 passwords incl. umlaut),
  AES-256-GCM data frames both directions (incl. real 16x16 JPEG Frame/Tiles), `encode()` bytes of EVERY message, and byte strings that must be rejected.
  Generated and verified by `src/vectors.rs` (`cargo test vectors`); a changed wire format fails that test. Cross-checked outside Rust
  (python `cryptography` for ECDH/HKDF/AES-GCM, `argon2-cffi`/libargon2 for Argon2id) - all identical.
- **Local test host**: `tools/orientos-testhost.sh` starts a local relay (plain `ws://`, `FV_ALLOW_INSECURE_RELAY=1`) and the Rust host with
  `FV_TESTHOST=1` (synthetic 640x480 gradient + text, one JPEG `Frame`, a `Tiles` counter tile every second, a coloured tile on mouse click,
  clipboard echo `echo:<text>`, input logged as `TESTHOST input: ...`; JPEG only, never H.264). Prints ID + password. From QEMU use `ws://10.0.2.2:<port>/fv/ws`.
  `--selftest` runs the Rust viewer against it (password, wrong password, ask mode, input).
  Ask mode is auto-confirmed (`FV_AUTOCONFIRM=1`); the 4-digit code can still be compared on both sides.
- The viewer must accept plain `ws://` for this local relay (explicit opt-in); the real relay stays `wss://` only.

## 2. What OrientOS has / lacks (read from osum tree, not run)

| Need | OrientOS state | Gap |
|---|---|---|
| TLS 1.3 client | `vendor/firn/lib/tls/`; `kernel/app/fetch.fi`; OTA tested vs local TLS 1.3 server with real chain (docs/OTA.md) | Never tested vs public server; OTA test had no DNS (IPv4 literal). Need DNS for freeviewer.fleitec.com + Let's Encrypt root in `/etc/ssl/roots.pem`. |
| WebSocket | none found | New `lib/net/ws.fi` (handshake, masked client frames, ping/close). |
| X25519, HKDF, SHA-256, HMAC | `lib/crypto/x25519.fi`, `hkdf.fi`, `sha256.fi` | Cross-check with Rust test vectors. |
| Argon2id | `lib/crypto/argon2.fi` | Must match Rust argon2 0.5 default params bit-exact; memory ~19 MiB in user space. Vector test. |
| AES-256-GCM | `vendor/firn/lib/std/crypto/gcm.fi` (chacha.fi header: needs AES-NI) | Verify AES-256 + AES-NI on Dell, else software path. `lib/crypto/aes.fi` has CCM/CMAC only. |
| JPEG decoder | `kernel/user/jpeg.fi` (640 lines), `vendor/firn/lib/paint/jpeg.fi` | Check baseline/progressive + subsampling of Rust `image` output; speed at 1080p unknown. |
| H.264 decoder | `kernel/user/h264.fi` (3260 lines), bit-exact vs ffmpeg; 640x480 25.2 fps, 720p 8.4 fps (docs/RUNDE-H264T.md) | Too slow for 1080p; host profile vs supported subset unknown. Later. |
| Sockets | `SYS_SOCKET/CONNECT/SENDTO/RECVFROM` in kernel/sys; DHCP client; e1000 tested; WLAN driver (USB chips) | Dell: LAN fine; WLAN uncertain (OrientOS KB: Roadmap). Check non-blocking recv + timers for ping loop. UDP not needed. |
| GUI | fUi / `wlib` (surface, input events) | Raw pixel canvas with mouse/key capture; keycode -> unicode/VK map (de layout, AltGr, dead keys). |
| Clipboard | `wlib.clip_put/clip_take` (+history, CLIP-2 drop target) | OK for text. |
| Store/package | `.opk` (pkg/opk.py, pakete/bauen.sh, /bin/opk), `assets/apps/*.osp`, store UI `bstore.fi`, OTA | Manifest v2 / SDK: ACCEL.md D-01 lists it as missing, OrientOS KB (12.10.) mentions tools/sdk/osdk.py; not found in the checkout I read -> confirm. Updates must be SIGNED (ed25519.fi exists). |

## 3. Milestones (ALL EFFORT = ESTIMATES, unit = focused worker days)
- M0 Prereqs (2-4 d): DNS + TLS from OrientOS to the relay; Rust-generated test vectors (HKDF, Argon2id, AES-GCM, proof) in `tests/orientos-vectors.json`.
- M1 Still picture (5-8 d): ws.fi, password handshake, one JPEG `Frame` shown in a fUi window. Gate: Rust headless host on server, viewer in QEMU.
- M2 Live video + mouse (5-8 d): `Tiles` compositing, scaling, mouse, ping, reconnect. Measure fps/latency on the Dell.
- M3 Keyboard + clipboard (4-6 d): keymap -> `Key`/`KeyVk`, `Special`, clipboard text, Ask mode with visible session code.
- M4 Store package + auto-update (4-7 d): .opk + manifest v2, store entry, SIGNED updates, saved IDs (no plain password storage).
- M5 optional later: H.264, P2P, audio.
- Total estimate ~20-33 d; usable (M1-M3) ~14-22 d.
- Windows host: NO change needed for M1-M3.

## 4. BLOCKER check: nonce bug from audit 29.09. (roadmap r1)
- Bug = FreeMeet E2E only (`src/meete2e.rs`): all participants share ONE key; nonce = track (1 audio, 2 video) + counter from 0 (`nonce[0]=spur`, `nonce[8..12]=zaehler`). Two senders on the same key repeat nonces -> AES-GCM broken.
- FreeViewer remote sessions are NOT affected (code check):
  - key fresh per session (ephemeral X25519 + random salt + HKDF), exactly 2 parties;
  - nonce = direction byte (host 1 / viewer 2) + 64-bit counter -> unique per key; receiver enforces strictly increasing counter;
  - UDP path uses a separate HKDF key (`udp_key`).
- => Firn viewer may be built against the current session protocol. Must NOT implement FreeMeet.
- Other FreeViewer findings relevant for the viewer (not nonce): r5 no mutual authentication (host never proves the password; in Ask mode only the 4-digit code protects against relay MITM -> show it prominently); r4 no brute-force limit; r2 unsigned Windows updates (do not copy for the OrientOS package); r6/r14 relay leaks/spoofing. None blocks M1-M3.
- Boss decision on the FreeMeet fix stays in project_needs; it does not gate this work.

## 5. Risks
- Argon2id bit-exactness + memory; AES-NI; JPEG speed; Dell network; DNS/TLS vs public server never tested.
- Handshake has no version byte: Rust-side protocol changes must stay backward compatible. Test-vector file = contract.
