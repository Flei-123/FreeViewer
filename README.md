# FreeViewer

**Open source remote desktop in Rust - a free TeamViewer alternative with a Parsec style game mode.**
Connect with a 9 digit ID and a session password, through a relay, end-to-end encrypted.
No account, no subscription, no telemetry.

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.88+-orange.svg)](https://www.rust-lang.org)

> Status: **v0.4 - working prototype.** Screen streaming (DXGI desktop
> duplication), remote mouse and keyboard including raw relative motion,
> clipboard sync, relay and crypto are implemented and measured end to end over
> the internet. File transfer and multi monitor are not done yet.

## What works today

- **9 digit ID + session password** (TeamViewer style). The ID is stable per
  machine, the password is regenerated on every start (or set it yourself).
- **Relay based connections** - works behind NAT/CGNAT/firewalls, no port
  forwarding, no VPN. A single outgoing `wss://` connection is enough.
- **End-to-end encryption** - X25519 key exchange + AES-256-GCM. The relay only
  routes opaque ciphertext, it never sees the password or the screen.
- **Password authentication with Argon2id**, bound to the ephemeral session
  keys (a captured proof cannot be replayed against another session).
- **DXGI Desktop Duplication capture** (Windows 8+) with an automatic `xcap`
  screenshot fallback: the compositor hands over the finished desktop frame,
  blocks until something actually changed and reports the dirty rectangles -
  9 ms per frame instead of 45 ms.
- **Tile delta encoding** - only changed 64x64 tiles are merged into rectangles
  and re-encoded; keyframes on session start, resolution change or when more
  than 60 % of the screen moved. An idle desktop costs almost no traffic.
- **Two session profiles, switchable during the session:**
  - *Fernwartung* (remote maintenance): sharp picture (max 1920 px, q68/78),
    absolute mouse, the remote cursor is drawn by the viewer, 30 fps target.
  - *Spiel* (game): raw **relative** mouse motion for in-game cameras, full
    keyboard grab, smaller picture (max 1280 px) at 60 fps target.
- **Remote input** - absolute positioning, relative raw motion, five mouse
  buttons, scroll wheel and the complete keyboard through `SendInput` with real
  virtual key codes and extended-key flags.
- **Key combinations** - a low level keyboard hook (`WH_KEYBOARD_LL`) grabs
  Win, Alt+Tab, Alt+F4, AltGr ... locally and sends them to the remote machine.
  A menu sends the ones Windows never lets an application produce:
  Ctrl+Alt+Del (via `sas.dll`), Ctrl+Shift+Esc, Windows key, Alt+Tab, Win+L.
- **Right Ctrl always stays local** and releases the grab. Letting go also tells
  the host to release every key and button, so a session can never leave "W"
  or Ctrl stuck on the remote machine.
- **Clipboard sync in both directions** (text, max 256 KB) - copy on one side,
  paste on the other; echo suppressed so the two machines cannot ping-pong.
- **Live stats** - resolution, fps, kbit/s and round trip time in the session bar.

## Quick start

```bash
git clone https://github.com/Flei-123/FreeViewer.git
cd FreeViewer
cargo run --release
```

Both computers run the same binary. The left card shows *your* ID and password,
the right card connects to someone else's.

In a session: switch **Fernwartung / Spiel** in the top bar, use **Tasten
senden** for Ctrl+Alt+Del and friends. In game mode, click into the picture to
grab mouse and keyboard, press **right Ctrl** to get them back.

Environment variables:

| Variable      | Meaning                                    | Default                            |
| ------------- | ------------------------------------------ | ---------------------------------- |
| `FV_RELAY`    | relay websocket URL                        | `wss://jarvis.fleitec.com/fv/ws`   |
| `FV_PASSWORD` | fixed session password (unattended access) | random on every start              |
| `FV_NODXGI`   | force the xcap screenshot backend          | unset (DXGI preferred)             |
| `FV_NODELTA`  | send full frames instead of tiles          | unset (delta on)                   |

Extra command line modes (handy for servers and testing):

```bash
freeviewer --headless                        # host only, no window, prints the ID
freeviewer --connect <id> <password> [n]     # viewer only, pulls n frames, prints stats
freeviewer --inputtest <id> <password>       # scripted mouse/keyboard/clipboard test
freeviewer --deltatest [n]                   # benchmark: capture, scale, encode per profile
freeviewer --captest [n]                     # DXGI vs xcap capture timings
```

## Architecture

```
+---------------+        wss (TLS)        +--------------+        wss (TLS)        +---------------+
|   Viewer      | <---------------------> |    Relay     | <---------------------> |     Host      |
| egui window   |   AES-256-GCM payload   | dumb pipe,   |   AES-256-GCM payload   | DXGI capture  |
| raw input hook|   (relay cannot read)   | id directory |                         | SendInput     |
+---------------+                         +--------------+                         +---------------+
```

- `src/main.rs` - egui GUI, session view, input forwarding, mode switch
- `src/capture.rs` - DXGI desktop duplication backend + xcap fallback
- `src/encoder.rs` - downscale, tile delta detection, JPEG encode, tile blit
- `src/hostside.rs` - capture thread, input thread, host session, profiles
- `src/input.rs` - host side injection (SendInput, virtual keys, SAS)
- `src/vinput.rs` - viewer side raw capture (pointer lock + keyboard hook)
- `src/clip.rs` - clipboard polling/writing for both ends
- `src/viewer.rs` - viewer session, frame/tile decode into a persistent canvas
- `src/crypto.rs` - X25519 + HKDF + Argon2id + AES-256-GCM channel
- `src/proto.rs` - compact binary message format
- `src/selftest.rs` - scripted input test driven over a real session
- `relay/relay.js` - the relay (Node.js, ~200 lines, zero knowledge)

### Handshake

```
viewer -> host : 0x01 || client_pub(32)
host -> viewer : 0x02 || host_pub(32) || salt(16)
viewer -> host : 0x03 || HMAC(Argon2id(password, salt), "fv-auth"||pubs||salt)
host -> viewer : 0x04 ok  |  0x05 wrong password
both           : 0x10 || nonce(12) || AES-256-GCM(payload)
```

Session key = HKDF-SHA256(X25519 shared secret, salt). Nonces are direction
tagged and strictly increasing, so replay and reflection are rejected.

## Performance

Measured on a Ryzen 7 3800X with a 3440x1440 screen, relay in a datacenter
(~50 ms RTT). Reproduce with `freeviewer --captest` and `freeviewer --deltatest 120`.

| Capture backend                | per frame |
| ------------------------------ | --------- |
| DXGI desktop duplication (v0.4)| 9.1 ms    |
| xcap screenshot (v0.3)         | 44.7 ms   |

| Profile               | stream size | capture | scale   | encode | possible fps | per frame |
| --------------------- | ----------- | ------- | ------- | ------ | ------------ | --------- |
| Fernwartung           | 1920x804    | 5.1 ms  | 17.7 ms | 4.2 ms | 37 fps       | 31.6 KB   |
| Spiel                 | 1280x536    | 5.2 ms  | 10.8 ms | 1.9 ms | 56 fps       | 9.2 KB    |

End to end through the relay, moving desktop: 43-45 fps at ~2.1 Mbit/s, 48-70 ms
round trip. Scaling is now the most expensive step - the next win is scaling on
the GPU and a hardware H.264 encoder instead of JPEG tiles.

### How the input side is verified

`freeviewer --inputtest <id> <password>` drives a real encrypted session and
scripts every input path; `tools/input_probe.ps1` samples the *host* machine
from outside (cursor position, NumLock, Ctrl, clipboard) every 200 ms. The two
logs are lined up afterwards, so "it works" is a measurement, not a claim:

| Step sent by the viewer      | measured on the host                       |
| ---------------------------- | ------------------------------------------ |
| absolute move 5000/5000      | cursor at the exact center of the screen   |
| absolute move 2500/2500      | cursor at the quarter point                |
| 30x relative +10/0           | cursor moves right (pointer ballistics)    |
| NumLock tap, twice           | NumLock state flips and flips back         |
| Ctrl down (held)             | `GetAsyncKeyState(VK_CONTROL)` set         |
| release-all special          | Ctrl released again                        |
| clipboard "FV-CLIP-..."      | host clipboard contains the text           |

## Run your own relay

The relay is a small Node.js service - anyone can host their own and point
clients at it with `FV_RELAY`.

```bash
node relay/relay.js          # listens on :7180, websocket path /fv/ws
node relay/test_relay.js     # end to end smoke test
```

It keeps only `sha256(host_secret) -> id` on disk so IDs stay stable. It has no
idea what the sessions contain.

## Roadmap

- [x] relay + end-to-end crypto + ID/password login
- [x] screen streaming, remote mouse/keyboard
- [x] delta/tile encoding instead of full JPEG frames (v0.3)
- [x] DXGI desktop duplication capture (v0.4)
- [x] game mode: raw relative mouse, full keyboard grab, key combos (v0.4)
- [x] clipboard sync (v0.4)
- [ ] file transfer (drag & drop)
- [ ] multi monitor selection
- [ ] hardware video encode (H.264/AV1), GPU scaling
- [ ] direct P2P (UDP hole punching) with relay fallback
- [ ] unattended access as a service, Linux/macOS builds
- [ ] session recording, chat

## License

GPL-3.0 - see [LICENSE](LICENSE).
