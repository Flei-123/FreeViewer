# FreeMeet E2E in Firefox / Safari (roadmap r62) - measured 14.10.2026

Two code paths exist in the browser client:

| path | browsers | where the frame transform runs |
|---|---|---|
| `createEncodedStreams()` | Chromium | main thread (`app.js` -> `FreeMeetE2E.spurSchuetzen`) |
| `RTCRtpScriptTransform` | Firefox, Safari 15.4+ | worker `/e2e-worker.js` (same `spurSchuetzen`, key handle posted once) |

`FreeMeetE2E.moeglich()` is true for either path (ScriptTransform also needs `Worker`). Otherwise the page shows
"E2E: von diesem Browser nicht unterstuetzt", joins with `e2e:false` and the SFU keeps it out of E2E rooms.

## Files
* `web-patch/e2e-worker.js` - new worker (SFU serves it: route + `include_str!`, SFU patch `rollout/0017`).
* `web-patch/e2e.js` - `moeglich()` + fail-closed sender (video frame without H.264 NAL unit is dropped, never sent clear).
* `web-patch/app-e2e-worker.patch` - against live `web/app.js` sha `714edc88` AFTER `app-join-e2e-ver.patch`
  (`patch -p1` in this order; checked: result is byte-identical to the tested file).
* `tools/freemeet-e2e/browsers.mjs` - the Playwright check below (`inpage` and `sfu` modes, header explains usage).
* Order on CT111: `e2e-worker.js` into `web/` BEFORE the SFU build (include_str!), then `e2e.js`, then the two app.js patches.

## Bugs found and fixed (app.js)
1. No worker existed: Firefox/Safari could not do E2E at all (r62 gap, see below for the earlier moeglich() fix).
2. Mic/camera switched on mid-meeting, screen share and system audio created transceivers WITHOUT `e2eSender()`:
   they went out unencrypted (and E2E receivers drop them -> black/silent). Fixed (4 places).
3. VP8/VP9: `schuetzenH264` returned frames without NAL units unchanged -> a browser without H.264 sent CLEAR video in an
   "E2E: an" room. Now: video codec preference H.264 only when E2E is on (`e2eVideoCodecs`) and the sender drops such frames.
4. NOTE: the SFU repo's `web/` is a STALE snapshot (app.js 2798 lines, sends `join` BEFORE the key is built -> `e2e:false`).
   Live app.js (3778 lines, public at meet.fleitec.com) has the right order. All tests below ran against the live files.

## Results (Playwright 1.61.1 builds: Chromium, Firefox, WebKit; server, no real devices)
`node tools/freemeet-e2e/browsers.mjs inpage <webdir> [chromium firefox webkit]` (no SFU needed):

| check | Chromium | Firefox (+OpenH264) | WebKit |
|---|---|---|---|
| createEncodedStreams / RTCRtpScriptTransform | yes / yes | no / yes | no / no |
| `moeglich()` | true | true | false |
| audio + H.264 encrypt/decrypt round trip, tamper dropped | ok | ok | ok |
| replay dropped (r60), audio 40/80 and video 10/20 kept | ok | ok | ok |
| clear VP8 never leaves sender; clear video from network dropped | ok | ok | ok |
| worker loopback, both sides (video frames decoded in ~9 s) | 59 | 38 (audio concealed 0 %) | n/a |
| worker loopback, receiver WITHOUT transform | 0 frames | 0 frames (audio concealed 87.7 %) | n/a |

Stock Playwright Firefox has NO H.264 until the OpenH264 plugin is downloaded (~20 s, internet; profile kept in
`FF_PROFILE_TPL`). Without it Firefox sends no video in E2E rooms (fail closed, audio works).

SFU testhost (`FM_OFFEN=1`, live web files, 12 s, `framesDecoded` of the remote video):

| clients (first, second) | result |
|---|---|
| chromium, chromium | 261 / 265 - ok |
| chromium, firefox | 398 / 263 - ok (cross-browser E2E works) |
| firefox, firefox | first 397, second **0** (late joiner) - fixed by r82 below |
| firefox, chromium | firefox 265, chromium **0** (late joiner) - fixed by r82 below |
| same firefox pairs WITHOUT key (control) | all decode - not a general Firefox problem |
| firefox, webkit / webkit, firefox | the second one gets `denied` with the "Verschluesselung passt nicht" text - ok |
| webkit alone | joins `e2e:false`, tag "E2E: aus" |

Join announces `e2e:true, e2e_ver:2` for Chromium/Firefox, `e2e:false` for WebKit.

## FIXED (r82): Firefox as SENDER, late joiner saw no video (E2E only)
Symptom: a client joining AFTER a Firefox sender got ~1500 video packets but its receiver transform saw 1 frame
(`framesReceived 0`, no PLI, no NACK).

Root cause (measured with a debug copy of `e2e.js` that logged every frame, NAL layout and pps_id):
* Firefox gives its three simulcast layers DIFFERENT parameter set ids: layer 1 sends `SPS id 0 / PPS id 0`, layer 2
  `SPS id 1 / PPS id 1` (`67 42c00c 43..`, `68 48e3c8`), layer 3 id 0 again. Chromium's layers all use id 0.
* A WebRTC receiver (libwebrtc, also inside Firefox) reads `first_mb_in_slice`, `slice_type`, `pic_parameter_set_id`
  as Exp-Golomb codes from the FIRST BITS OF EVERY SLICE NAL (in the RTP depacketizer, before our transform) and drops
  the frame when no PPS with that id was seen. The encrypted slice starts with the blob byte `0xE3` = bits `1 1 1` =
  `0, 0, 0` -> pps_id 0 always. For the layer with PPS id 1 the key frame is "missing PPS" and dropped, forever.
  (Why a receiver present from the start was fine was not isolated; likely it was served an id-0 layer first.)
* Not the SFU, not `generateKeyFrame()`, not the nonce counter / frame header (all frames decrypted fine in loopback).

Fix (format extension, no new version, no SFU change): blob byte 0 of slice NAL units (type 1/5) is chosen so that the
receiver's parse reads `0, 0, <real pps_id>`: table `VIDEO_KENNUNG` (pps 0..6 -> `E3 D3 DB C9 CB CD CF`), pps_id > 6
keeps `E3`. Receivers accept exactly these 7 values; the byte stays inside the GCM AAD. Implemented in `web-patch/e2e.js`,
`src/meete2e.rs`, `tools/freemeet-e2e/ref.py` (new vectors `h264_pps`, the existing `h264`/`replay_video` vectors
changed because their synthetic slices happen to parse to pps_id 0..6), spec in `docs/FREEMEET-E2E-NONCE.md`.

Compatibility: Chromium and the native client always use pps_id 0 -> byte stays `E3`, nothing changes for them. Only
Firefox/Safari senders (which cannot do E2E today at all) emit the new values. A receiver WITHOUT this change drops
such slices, so roll out the receivers (web `e2e.js`, native client 0.30.x) before Firefox/Safari may send E2E video.

Measured after the fix (`EXPECT_VIDEO=1`, same setup, live web files + patches, 12 s, `framesDecoded`):

| clients (first, second) | before | after |
|---|---|---|
| firefox, chromium | firefox 265, chromium **0** | 262 / 446 |
| firefox, firefox | 397 / **0** | 399 / 463 |
| chromium, firefox | 398 / 263 | 400 / 266 |
| chromium, chromium | 261 / 265 | 266 / 263 |

Control: the same run with the old `e2e.js` fails the check (`chromium frames=0`).

## Test rig (server, no devices)
* SFU binary (embeds an OLDER web copy) + `tools/freemeet-e2e/webproxy.mjs <port> <sfuHttp> <webdir>`: serves the LIVE
  web files (`index.html app.js e2e.js e2e-worker.js hintergrund.js ton.js avatar.js style.css` + the two app.js patches)
  and forwards API/WebSocket to the SFU. Playwright `route()` does NOT work for this: Chromium then blocks the WebSocket
  (`ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`).
* `--public-ip` must be the LAN IP (ICE fails with 127.0.0.1); Firefox needs a PulseAudio null sink (`XDG_RUNTIME_DIR`),
  otherwise `FMTon.starte` hangs and the join never finishes.
* `EXPECT_VIDEO=1 node tools/freemeet-e2e/browsers.mjs sfu <base> firefox chromium` = hard check; `DUMPDIR` writes JSON.

## Not measurable here
* Real Safari / iPhone (RTCRtpScriptTransform on WebKit with H.264: Annex-B or length-prefixed frames?) - needs a real device
  (r21). The Playwright WebKit build has no ScriptTransform and no camera; only the crypto core ran in WebKit.
* Firefox with real cameras, Android Firefox, simulcast under load.
