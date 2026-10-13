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
| video, per encrypted NAL | blob = `0xE3 ‖ salt(8) ‖ counter(4) ‖ nal index(2) ‖ ct ‖ tag16`, then emulation-prevention escaping; AAD = NAL header byte ‖ first 15 blob bytes |
| unchanged | start codes, NAL header byte, SPS/PPS/AUD stay clear (server repacks RTP), 64-number replay window |

Differences to v1 that matter:

* v1 audio started with `0xE2` and had a 6 byte header; v1 video blobs started
  with the NAL index byte. v2 starts with `0xE3` in both. Old and new clients
  therefore **detect and drop** each other's frames (no garbage picture, no
  decryption with a wrong nonce). Native shows once per stream: "A participant
  uses an outdated end-to-end format".
* Overhead: audio +8 byte (14 instead of 6), video +10 byte per encrypted NAL.
* Replay guard is per sender salt (last 16 salts per track). Limit: a replay of
  a stream evicted from that list is not detected.
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
