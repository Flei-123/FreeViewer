// FreeMeet E2E worker for RTCRtpScriptTransform (Firefox, Safari).
//
// Chromium uses the legacy createEncodedStreams() on the main thread. Browsers
// that only have RTCRtpScriptTransform run the very same frame transform here:
// app.js posts the room key handle once, every sender/receiver transform then
// hands us its readable/writable pair of encoded frames.
// The key handle (non-extractable HKDF CryptoKey + a Map cache) survives
// structured clone; derived keys are re-derived on demand in this thread.
self.window = self;
importScripts('/e2e.js');

let roomKey = null;
let announceKey;
const keyReady = new Promise((resolve) => { announceKey = resolve; });

self.onmessage = (ev) => {
  const data = ev.data || {};
  if (data.key && !roomKey) {
    roomKey = data.key;
    announceKey();
  }
};

self.onrtctransform = (ev) => {
  const tr = ev.transformer;
  const { art, richtung } = tr.options;
  // Wait for the key; frames queue up in the readable stream meanwhile.
  keyReady.then(() => {
    self.FreeMeetE2E.spurSchuetzen(roomKey, art, tr.readable, tr.writable, richtung);
  });
};
