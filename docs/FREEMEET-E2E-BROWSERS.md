# FreeMeet E2E in Firefox / Safari (roadmap r62) - analysis 14.10.2026

## Findings (measured, not assumed)
Feature probe with Playwright on the server (`RTCRtpSender`, `createEncodedStreams`, `RTCRtpScriptTransform`):

| browser | createEncodedStreams | RTCRtpScriptTransform |
|---|---|---|
| Chromium 149 | yes | yes |
| Firefox 151 | **no** | yes |
| WebKit 26.5 (Playwright build) | no | no (real Safari 15.4+ has it; this build is minimal) |

`web/app.js` (live, 3778 lines, sha `714edc88...`) only wires the legacy path:
`e2eAnhaengen()` uses `RTCRtpScriptTransform` only `if (state.e2eWorker)` - and `state.e2eWorker` is
**never set** anywhere (grep over the live file). Then it needs `createEncodedStreams`, otherwise it logs
"Meeting laeuft unverschluesselt".

## The real bug (fixed in this change)
`FreeMeetE2E.moeglich()` answered **true** as soon as `RTCRtpScriptTransform` existed. So in Firefox/Safari:
tag "E2E: an" (green), join announces `e2e: true`, but media goes out **unencrypted** and E2E media from
others cannot be decoded. A false sense of security. Fix: `moeglich()` is true only with `createEncodedStreams`
(the path that exists). Now those browsers show "E2E: von diesem Browser nicht unterstuetzt", join with
`e2e: false` and the SFU keeps them out of E2E rooms with its "mixed room" message. Test: `node --test web-patch/`
("moeglich(): only browsers with createEncodedStreams count").

## Plan for real Firefox/Safari support (NOT done - not small, not testable here)
1. New file `web/e2e-worker.js` (served like `e2e.js`: route + `include_str!` in `signal.rs`, so SFU rebuild):
   ```js
   self.window = self; importScripts('/e2e.js'); let key; const ready = new Promise(r => self.onmessage = e => { key = e.data.key; r(); });
   self.onrtctransform = async (e) => { await ready; const { art, richtung } = e.transformer.options;
     FreeMeetE2E.spurSchuetzen(key, art, e.transformer.readable, e.transformer.writable, richtung); };
   ```
   `spurSchuetzen` already works on any readable/writable pair of encoded frames.
2. `app.js`: `state.e2eWorker = new Worker('/e2e-worker.js'); postMessage({key: state.e2eKey})` (the key handle holds a
   non-extractable `CryptoKey` + a `Map`; both survive structured clone - to verify), `moeglich()` back to
   "ScriptTransform OR createEncodedStreams", and `encodedInsertableStreams` stays for Chromium.
3. Open questions that need REAL devices: (a) in Safari the H.264 frames of `RTCRtpScriptTransform` may not be Annex-B
   (our NAL parser needs start codes; WebKit has used length-prefixed data in some versions) - must be measured with a
   real iPhone/Mac; (b) `RTCEncodedVideoFrame.type`/key-frame flag for audio vs video; (c) sender/receiver transform per
   simulcast layer in Firefox.
4. Test: Firefox headless can do fake media (`media.navigator.streams.fake`) - a 2-browser loopback test is possible
   there; Safari only on a real device (see r21).

Effort: ~0.5-1 day incl. real-device tests. Until then: Firefox/Safari users cannot join E2E rooms (clear message).
