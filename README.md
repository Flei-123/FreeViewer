# FreeViewer

**Open source remote desktop in Rust - a free TeamViewer alternative.**
Connect with a 9 digit ID and a session password, through a relay, end-to-end encrypted.
No account, no subscription, no telemetry.

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)
[![Rust](https://img.shields.io/badge/rust-1.88+-orange.svg)](https://www.rust-lang.org)

> Status: **v0.2 - working prototype.** Screen streaming, remote mouse and
> keyboard, relay and crypto are implemented and tested end to end over the
> internet. File transfer, clipboard sync and multi monitor are not done yet.

## What works today

- **9 digit ID + session password** (TeamViewer style). The ID is stable per
  machine, the password is regenerated on every start (or set it yourself).
- **Relay based connections** - works behind NAT/CGNAT/firewalls, no port
  forwarding, no VPN. A single outgoing `wss://` connection is enough.
- **End-to-end encryption** - X25519 key exchange + AES-256-GCM. The relay only
  routes opaque ciphertext, it never sees the password or the screen.
- **Password authentication with Argon2id**, bound to the ephemeral session
  keys (a captured proof cannot be replayed against another session).
- **Screen streaming** - JPEG frames, downscaled to max 1600 px width,
  15 fps target, unchanged frames are skipped (idle screen = almost no traffic).
- **Remote input** - absolute mouse positioning, all three buttons, scroll
  wheel, keyboard including modifiers (Shift/Ctrl/Alt) and F1-F12.
- **Live stats** - resolution, fps, kbit/s and round trip time in the session bar.

## Quick start

```bash
git clone https://github.com/Flei-123/FreeViewer.git
cd FreeViewer
cargo run --release
```

Both computers run the same binary. The left card shows *your* ID and password,
the right card connects to someone else's.

Environment variables:

| Variable      | Meaning                                    | Default                            |
| ------------- | ------------------------------------------ | ---------------------------------- |
| `FV_RELAY`    | relay websocket URL                        | `wss://jarvis.fleitec.com/fv/ws`   |
| `FV_PASSWORD` | fixed session password (unattended access) | random on every start              |

Extra command line modes (handy for servers and testing):

```bash
freeviewer --headless                       # host only, no window, prints the ID
freeviewer --connect <id> <password> [n]    # viewer only, pulls n frames, prints stats
```

## Architecture

```
+---------------+        wss (TLS)        +--------------+        wss (TLS)        +---------------+
|   Viewer      | <---------------------> |    Relay     | <---------------------> |     Host      |
| egui window   |   AES-256-GCM payload   | dumb pipe,   |   AES-256-GCM payload   | xcap capture  |
| input capture |   (relay cannot read)   | id directory |                         | enigo input   |
+---------------+                         +--------------+                         +---------------+
```

- `src/main.rs` - egui GUI, input capture, session view
- `src/hostside.rs` - screen capture thread, input execution thread, host session
- `src/viewer.rs` - viewer session, frame decode
- `src/crypto.rs` - X25519 + HKDF + Argon2id + AES-256-GCM channel
- `src/proto.rs` - compact binary message format
- `src/net.rs` - relay websocket transport
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
- [ ] file transfer (drag & drop)
- [ ] clipboard sync
- [ ] multi monitor selection
- [ ] delta/tile encoding instead of full JPEG frames, hardware encode
- [ ] direct P2P (UDP hole punching) with relay fallback
- [ ] unattended access as a service, Linux/macOS builds
- [ ] session recording, chat

## License

GPL-3.0 - see [LICENSE](LICENSE).
