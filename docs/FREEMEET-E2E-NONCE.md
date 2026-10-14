# FreeMeet E2E: nonce reuse fix (roadmap r1) - format v2

Status 13.10.2026: code + tests done, **NOT deployed** (needs the boss's OK, see "Deploy").

## The bug (v1)

* One AES-256-GCM key per track kind (`ton`, `bild`) for the WHOLE room.
* Nonce = `kind byte | counter`, counter started at 0 in every sender.
* So these produced the same (key, nonce): two participants; camera and
  screen share of one participant (both `bild`, both counter 0); a rejoin
  (counter back to 0); the NAL index was only 8 bit (frame with > 256 NAL units
  repeated).
* Effect: ciphertext XOR ciphertext = plaintext XOR plaintext, and the GHASH
  key leaks, so frames can be forged. The media server (honest-but-curious)
  could read/forge media although E2E is on.
* Only FreeMeet (native `src/meete2e.rs` and browser `e2e.js`). The
  FreeViewer remote session has its own crypto and is not affected.

## Format v2

Root secret (32 byte, from the link `#k=`) is unchanged, so links stay valid.

| item | v2 |
|---|---|
| sender salt | 8 random bytes per outgoing STREAM (mic, camera, every screen share, every join/rejoin); new salt before a 32-bit counter would wrap |
| stream key | `HKDF-SHA256(ikm=root, salt="freemeet-e2e-v2", info=kind ‖ 0x00 ‖ salt)`, kind = `ton` / `bild`, 32 byte |
| nonce (12 byte) | `kind(1: ton=1, bild=2) ‖ nal index(2, BE) ‖ salt[0..5] ‖ counter(4, BE)` |
| audio frame | `0xE3 ‖ flags(1, bit0 = key frame) ‖ salt(8) ‖ counter(4, BE) ‖ AES-GCM(ct ‖ tag16)`; AAD = the 14 header bytes |
| video, per encrypted NAL | blob = `ver ‖ salt(8) ‖ counter(4) ‖ nal index(2) ‖ ct ‖ tag16`, then emulation-prevention escaping; AAD = NAL header byte ‖ first 15 blob bytes |
| video blob `ver` (r82) | `0xE3`, except slice NAL units (type 1/5) with slice `pps_id` 1..6: `0xD3 0xDB 0xC9 0xCB 0xCD 0xCF` (index = pps_id, table `VIDEO_KENNUNG`). Why: a WebRTC receiver reads `first_mb_in_slice, slice_type, pps_id` (Exp-Golomb) from the first bits of every slice and drops frames whose pps_id has no PPS; the byte is built to read `0, 0, pps_id`. Firefox gives simulcast layers different SPS/PPS ids, so with a constant `0xE3` (reads pps_id 0) a late joiner never got a key frame. pps_id > 6 keeps `0xE3`. Receivers accept exactly these 7 values; AAD still covers the byte |
| unchanged | start codes, NAL header byte, SPS/PPS/AUD stay clear (server repacks RTP) |
| replay guard | per sender salt: 64-number window (audio AND, since r60, video) - receiver-side only, **no change of the wire format** |

Differences to v1 that matter:

* v1 audio started with `0xE2` and had a 6 byte header; v1 video blobs started
  with the NAL index byte. v2 starts with `0xE3` in both (video blobs of slices with pps_id 1..6: see `ver` above; a receiver before r82 drops those, Chromium senders always use pps_id 0). Old and new clients
  therefore **detect and drop** each other's frames (no garbage picture, no
  decryption with a wrong nonce). Native shows once per stream: "A participant
  uses an outdated end-to-end format".
* Overhead: audio +8 byte (14 instead of 6), video +10 byte per encrypted NAL.
* Replay guard is per sender salt (last 16 salts per track). Limit: a replay of
  a stream evicted from that list is not detected. Video: see "Replay guard for video (r60)".
* Receivers only cache a derived key after a frame authenticated (forged salts
  cannot flush the cache).
* The media server is NOT affected: it never parses the audio header or the
  encrypted NAL contents (checked: `freemeet-sfu` only knows the join flag
  `e2e`); NAL type 5 / start codes stay visible. **No SFU deploy needed.**

## Files

* `src/meete2e.rs` - native: `Schluessel`, `Sender` (owns salt+counter, API
  cannot repeat a nonce), `Wache`; `src/meetrtc.rs` uses one `Sender` for mic,
  camera and screen share.
* `web-patch/e2e.js` - browser. Built on the LIVE file (sha256 `17a4227f…`,
  identical to `/opt/freemeet/web/e2e.js` on CT111, checked read-only
  13.10.2026). Same global API (`window.FreeMeetE2E.spurSchuetzen`,
  `schluesselAufbauen`, ...), **`app.js` is unchanged**. The room key handle
  is opaque to `app.js`.
* `tools/freemeet-e2e/ref.py` - independent Python reference; writes
  `tests/freemeet-e2e-vectors.json` (keys, nonces, audio and H.264 frames).
* Tests: Rust `cargo test meete2e` (33 tests incl. vectors, uniqueness over
  2 senders / mic+camera+screen / rejoin, counter rotation), JS
  `node --test web-patch/` (12 tests, same vectors, same uniqueness test via
  `spurSchuetzen`), Python `ref.py --check`.

## Migration plan

v1 and v2 cannot talk to each other (on purpose: accepting v1 would keep the
nonce-reuse alive for those senders). Only rooms created with E2E are
affected; normal rooms are not.

The browser updates immediately (`cache-control: no-cache`, plain reload),
native clients only via release/auto-update. So the order is:

1. **Native release first** (Windows + Mac build with this commit), roll out via
   the usual update channel. Until the browser is switched, new native clients
   in an E2E room only talk to other new native clients - old browser tabs and
   old natives are dropped (clear message on native).
2. **Browser last** (copy `web-patch/e2e.js` to CT111, takes effect on reload).
   Do it when no important E2E meeting is running; everybody has to reload the
   tab once.
3. Afterwards old natives show "outdated end-to-end format" - they must
   update. There is no SFU change; optional follow-up: let the SFU refuse the
   join of clients that do not announce `e2e_ver: 2` in `caps` (clean error
   instead of silent drops). Roadmap item added.
4. Existing E2E links keep working (same root key). Rotate the key of
   important rooms anyway: everything recorded under v1 was exposed to
   (key, nonce) reuse, a new link does not fix the past but limits future
   exposure.

Rollback: restore the backup of `e2e.js` on CT111 (`cp e2e.js.bak_nonce_v1 e2e.js`) and
ship the previous native build; never run mixed v1/v2 in one E2E room.

## Deploy (needs the boss's OK; nothing of this was run)

Browser (CT111, `192.168.1.61`):

```
cd /opt/freemeet
cp web/e2e.js web/e2e.js.bak_nonce_v1
# upload web-patch/e2e.js from the repo to /opt/freemeet/web/e2e.js
sha256sum web/e2e.js                # compare with web-patch/e2e.js
git add web/e2e.js && git commit -m "FreeMeet E2E v2: per-sender salt + unique nonces (r1)"
curl -sI https://meet.fleitec.com/e2e.js | grep -i content-length
```
No service restart (static file). Check afterwards with two browsers in one E2E
room (camera + screen share): picture and sound arrive, tag says "E2E: an".

Native: normal release build/sign/publish of the commit that contains this
file; no config change.

## Correction (14.10.2026, found while preparing the rollout)

The "Deploy" section above is WRONG about the browser file: on CT111 `signal.rs` serves `e2e.js` via
`include_str!("../../web/e2e.js")`, i.e. it is compiled into the SFU binary. Copying the file changes nothing;
the SFU must be rebuilt (`cargo build --release`) and restarted (this ends running meetings).
The tested, scripted procedure is in `rollout/03-freemeet-nonce/` (see `rollout/README.md`).

## Replay guard for video (r60, 14.10.2026)

Before: only audio had the 64-number window; a server-side attacker could re-inject
old (valid) video frames. Now every receiver checks video too. **Wire format v2 is
unchanged** (receiver-side check only): v2 senders and old v2 receivers keep working,
old v2 receivers just do not have the check yet.

Rules for one received video frame (`Schluessel::oeffnen_h264_wache` / `oeffnenH264Kopf` + `wache`):

1. Every encrypted NAL unit of the frame must carry the SAME salt and counter. Each NAL has
   its own tag, so without this rule NAL units of different frames could be spliced
   into a frame that opens fine.
2. NAL indices must strictly increase inside the frame (no repeated, no reordered NAL
   units; the sender numbers by position, so this holds for every honest frame).
3. Only after ALL tags verified: (salt, counter) goes through the replay window of that
   track (`Wache`, one window per sender salt, last 16 salts). Forged frames cannot move
   the window.
4. A frame without any encrypted NAL unit (only SPS/PPS/AUD) has no counter and is passed on
   (it carries no picture content).

Same rules in Rust (`src/meete2e.rs`, wired in `meetrtc.rs`, one `Wache` per received track)
and in the browser (`web-patch/e2e.js`, `spurSchuetzen`, incoming video).

**Bug found while doing this (fixed in both):** a counter jump of EXACTLY 64 dropped the
old highest counter from the window, so a replay of the frame just before the jump was
accepted. The JS bit field also grew without bound over a long meeting (BigInt, never
masked); it is now masked to 64 bits.

Tests: the reference `tools/freemeet-e2e/ref.py` models the window with a SET (not a bit
field) and writes `guard` (13 counter sequences incl. step 63/64/65, wrap edge, 17-salt
eviction) and `replay_video` (9 frame sequences: replay, reorder, splice, swapped,
repeated NAL, second stream, clear-only frame) into `tests/freemeet-e2e-vectors.json`.
Rust (`vectors_guard_windows`, `vectors_video_replay`, ...) and JS (`node --test web-patch/`)
must reproduce every accept/reject. Documented limits unchanged: a replay of a stream
evicted from the 16-salt list is not detected; a replay older than 64 frames is
dropped (also legit late frames).

## Old clients in an E2E room (r61, 14.10.2026)

Correction to "No SFU deploy needed": for the SILENT DROP the SFU is indeed not involved
(it never parses audio; its `e2e_geschuetzt` only knows the v1 marker `0xE2` and is only
called for video keyframe detection, where a v1/v2 blob never starts the frame). But the
SFU can refuse an outdated client with a clear message instead of black tiles:

* `caps.e2e_ver` (u8, default 0 = not announced) in the join. Native client announces 2
  (`meete2e::FORMAT_VERSION`), the browser with the one-line patch
  `web-patch/app-join-e2e-ver.patch` (made against the LIVE `web/app.js`, sha256
  `714edc88...`, line 336; `patch -p1 --dry-run` checked on a synthetic copy only).
* SFU (`freemeet-sfu`, branch `main` + `live-merged`, rollout patch `0010`): env
  `FM_E2E_MIN_VER` (default **0 = off**). With `FM_E2E_MIN_VER=2` a join with `e2e: true` and
  `e2e_ver < 2` gets `denied` ("Dein Programm nutzt noch das alte Verschluesselungsformat ...")
  and the socket is closed. Plain joins are never touched. Always logs a warning for
  `e2e` joins without `e2e_ver`.
* Order: SFU patch (inert) -> native release (announces 2) -> app.js patch (browser announces 2)
  -> only then set `FM_E2E_MIN_VER=2` (systemd drop-in) and restart. Switching it on earlier
  would lock out every browser (they do not announce yet).
* Not proven live: the real join of old/new browsers against the patched SFU (unit tests +
  JSON parse only).
