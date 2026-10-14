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
| firefox, firefox | first 397, second **0** (late joiner) |
| firefox, chromium | firefox 265, chromium **0** (late joiner) |
| same firefox pairs WITHOUT key (control) | all decode - not a general Firefox problem |
| firefox, webkit / webkit, firefox | the second one gets `denied` with the "Verschluesselung passt nicht" text - ok |
| webkit alone | joins `e2e:false`, tag "E2E: aus" |

Join announces `e2e:true, e2e_ver:2` for Chromium/Firefox, `e2e:false` for WebKit.

## OPEN: Firefox as SENDER, late joiner sees no video (E2E only)
A client that joins AFTER a Firefox sender receives ~1500 video packets but its receiver transform gets 1 frame (Firefox) /
decodes 0 frames (Chromium). Firefox answers the SFU's PLI with fresh key frames (seen in the worker: 3 layers every ~0.4 s).
Experiment: same worker as pure pass-through (transform present, no encryption): late joiner decodes (400 frames). So the
transform itself and PLI work; the combination "encrypted key frame from Firefox after PLI" is not assembled downstream.
Not yet isolated (suspects: SFU H.264 repacketizing of the encrypted key frame from Firefox - SPS/PPS/IDR layout or size;
EPB in the blob; `is_keyframe`). Next: dump the SFU-side frame (`FM_TRACE`) for a Firefox PLI key frame and compare with a
Chromium one; try `generateKeyFrame()` in the worker.

## Not measurable here
* Real Safari / iPhone (RTCRtpScriptTransform on WebKit with H.264: Annex-B or length-prefixed frames?) - needs a real device
  (r21). The Playwright WebKit build has no ScriptTransform and no camera; only the crypto core ran in WebKit.
* Firefox with real cameras, Android Firefox, simulcast under load.
