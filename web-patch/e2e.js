// End-to-end encryption in the browser - frame format v2.
//
// Must match the native client (src/meete2e.rs) BYTE FOR BYTE, otherwise
// browser and program only see noise from each other. Spec, test vectors and
// migration plan: docs/FREEMEET-E2E-NONCE.md, tests/freemeet-e2e-vectors.json.
//
// WHY v2 (roadmap r1): v1 used one key per track kind for ALL participants and
// the nonce was `track kind + counter` starting at 0 in every sender. Two
// senders, camera + screen share, or a rejoin therefore repeated (key, nonce)
// pairs - fatal for AES-GCM. v2:
//   * every outgoing stream draws a fresh random 8-byte SENDER SALT,
//   * stream key = HKDF-SHA256(root, salt "freemeet-e2e-v2",
//                              info = kind || 0x00 || sender salt),
//   * nonce = kind | NAL index (16 bit) | salt[0..5] | counter (32 bit),
//   * a stream switches to a new salt BEFORE its counter would wrap.
//
// Audio frame:
//   byte 0       0xE3      version (v1 frames start with 0xE2 and are dropped)
//   byte 1       flags     bit 0 = key frame
//   byte 2..10   sender salt
//   byte 10..14  counter   32 bit, big endian
//   from 14      AES-256-GCM: ciphertext + 16 byte tag
// The header is the GCM associated data: the server MAY read the key frame
// flag (it needs it to switch quality layers) but cannot forge it.
//
// Video: per NAL unit blob (inside the emulation-prevention escaping):
//   byte 0 version | 1..9 salt | 9..13 counter | 13..15 NAL index | tag...
//   associated data = NAL header byte + the 15 byte blob header.
// Byte 0 is 0xE3 for everything except slice NAL units (type 1/5) whose slice
// header names a pps_id 1..6: there it is one of VIDEO_TAG (r82). A
// receiving WebRTC stack reads first_mb_in_slice, slice_type and pps_id as
// Exp-Golomb codes from the first bytes of every slice and DROPS the frame when
// no PPS with that id was seen. Firefox gives its simulcast layers different
// SPS/PPS ids (layer 2 uses 1), so with a constant 0xE3 (reads 0, 0, 0) a late
// joiner never assembled a key frame. Byte 0 is built to read 0, 0, pps_id.

const E2E_MAGIC = 0xE3;
const E2E_MAGIC_V1 = 0xE2;
const E2E_HEADER = 14;
const E2E_SALT = 8;
const E2E_NAL_KOPF = 1 + E2E_SALT + 4 + 2;
// pps_id -> blob byte 0: bits '1' '1' ue(pps_id) + padding. 0 = the old 0xE3.
const VIDEO_TAG = { 0: 0xE3, 1: 0xD3, 2: 0xDB, 3: 0xC9, 4: 0xCB, 5: 0xCD, 6: 0xCF };
const VIDEO_KENNUNGEN = new Set(Object.values(VIDEO_TAG));
const E2E_KDF_SALT = 'freemeet-e2e-v2';
const E2E_KEY_CACHE_MAX = 64;
const E2E_GUARD_SALTS_MAX = 16;

/** 43 characters of URL-safe base64 -> 32 bytes. */
function keyBytes(text) {
  const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  const raw = String(text || '').trim().replace(/=+$/, '');
  const aus = [];
  for (let i = 0; i < raw.length; i += 4) {
    const st = raw.slice(i, i + 4);
    if (st.length < 2) return null;
    let n = 0;
    for (let j = 0; j < st.length; j++) {
      const w = abc.indexOf(st[j]);
      if (w < 0) return null;
      n |= w << (18 - j * 6);
    }
    for (let j = 0; j < st.length - 1; j++) aus.push((n >> (16 - j * 8)) & 0xff);
  }
  return aus.length === 32 ? new Uint8Array(aus) : null;
}

const hexOf = (u8) => Array.from(u8, (b) => b.toString(16).padStart(2, '0')).join('');

/**
 * Room key handle. Holds the HKDF base key and a cache of derived stream
 * keys. Opaque to app.js (it only passes it to protectTrack).
 */
async function derive(raw) {
  const base = await crypto.subtle.importKey('raw', raw, 'HKDF', false, ['deriveBits']);
  return { base, cache: new Map(), version: 2 };
}

/** Derive the AES-GCM key of one stream. `art` is 'audio' or 'video'. */
async function streamKey(keys, art, salt) {
  const kind = art === 'audio' ? 'ton' : 'bild';
  const id = kind + ':' + hexOf(salt);
  const hit = keys.cache.get(id);
  if (hit) return hit;
  const key = await deriveStreamKey(keys, kind, salt);
  rememberKey(keys, id, key);
  return key;
}

async function deriveStreamKey(keys, kind, salt) {
  const enc = new TextEncoder();
  const info = new Uint8Array(kind.length + 1 + E2E_SALT);
  info.set(enc.encode(kind), 0);
  info[kind.length] = 0;
  info.set(salt, kind.length + 1);
  const bits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: enc.encode(E2E_KDF_SALT), info },
    keys.base, 256);
  return crypto.subtle.importKey('raw', bits, 'AES-GCM', false, ['encrypt', 'decrypt']);
}

function rememberKey(keys, id, key) {
  // Starting over is always safe: keys are re-derived on demand.
  if (keys.cache.size >= E2E_KEY_CACHE_MAX) keys.cache.clear();
  keys.cache.set(id, key);
}

/**
 * Key for OPENING a frame. The salt is attacker-controlled until the tag
 * checks out, so a freshly derived key is only remembered by the caller after
 * a successful decryption (see `remember`).
 */
async function openKey(keys, art, salt) {
  const kind = art === 'audio' ? 'ton' : 'bild';
  const id = kind + ':' + hexOf(salt);
  const hit = keys.cache.get(id);
  if (hit) return { key: hit, remember: null };
  const key = await deriveStreamKey(keys, kind, salt);
  return { key, remember: () => rememberKey(keys, id, key) };
}

/** Fresh random sender salt. */
function newSalt() {
  const s = new Uint8Array(E2E_SALT);
  crypto.getRandomValues(s);
  return s;
}

/** GCM nonce: kind | NAL index | first 5 salt bytes | counter. */
function nonce(trackNo, nal, salt, counter) {
  const n = new Uint8Array(12);
  n[0] = trackNo;
  new DataView(n.buffer).setUint16(1, nal, false);
  n.set(salt.subarray(0, 5), 3);
  new DataView(n.buffer).setUint32(8, counter >>> 0, false);
  return n;
}

// ---------------------------------------------------------- audio ---------

/** Encrypt one audio frame with an explicit salt + counter (LOW LEVEL). */
async function protectAudio(keys, salt, counter, keyframe, plain) {
  const key = await streamKey(keys, 'audio', salt);
  const header = new Uint8Array(E2E_HEADER);
  header[0] = E2E_MAGIC;
  header[1] = keyframe ? 0x01 : 0x00;
  header.set(salt, 2);
  new DataView(header.buffer).setUint32(2 + E2E_SALT, counter >>> 0, false);
  const geheim = new Uint8Array(await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv: nonce(1, 0, salt, counter), additionalData: header }, key, plain));
  const aus = new Uint8Array(E2E_HEADER + geheim.length);
  aus.set(header, 0);
  aus.set(geheim, E2E_HEADER);
  return aus;
}

/** Header fields of an audio frame, or null if it is not a v2 frame. */
function audioHeader(daten) {
  if (daten.length <= E2E_HEADER + 16 || daten[0] !== E2E_MAGIC) return null;
  return {
    salt: daten.slice(2, 2 + E2E_SALT),
    counter: new DataView(daten.buffer, daten.byteOffset).getUint32(2 + E2E_SALT, false),
  };
}

/** Decrypt one audio frame. Returns {plain, salt, counter} or null. */
async function openAudio(keys, daten) {
  const k = audioHeader(daten);
  if (!k) return null;
  const { key, remember } = await openKey(keys, 'audio', k.salt);
  let plain;
  try {
    plain = new Uint8Array(await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: nonce(1, 0, k.salt, k.counter),
        additionalData: daten.slice(0, E2E_HEADER) },
      key, daten.slice(E2E_HEADER)));
  } catch (e) {
    return null;
  }
  if (remember) remember();
  return { plain, salt: k.salt, counter: k.counter };
}

// ------------------------------------------------------- H.264 (video) ----
//
// The media server repacks images into RTP and must understand the H.264
// structure (measured 07.08.2026: a fully encrypted payload fails with
// "NaluTypeIsNotHandled"). So the STRUCTURE stays visible and only the
// CONTENT is encrypted. Must match meete2e.rs byte for byte.

function nextStartCode(d, ab) {
  let i = ab;
  while (i + 3 <= d.length) {
    if (d[i] === 0 && d[i + 1] === 0) {
      if (d[i + 2] === 1) return [i, i + 3];
      if (i + 4 <= d.length && d[i + 2] === 0 && d[i + 3] === 1) return [i, i + 4];
    }
    i++;
  }
  return null;
}

function nalUnits(d) {
  const aus = [];
  let cursor = 0, offen = null;
  for (;;) {
    const t = nextStartCode(d, cursor);
    if (!t) break;
    if (offen) aus.push([offen[0], offen[1], t[0]]);
    offen = t;
    cursor = t[1];
  }
  if (offen) aus.push([offen[0], offen[1], d.length]);
  return aus;
}

/** SPS, PPS and AUD stay in the clear - the packer in the server needs them. */
const nalStaysClear = (typ) => typ === 7 || typ === 8 || typ === 9;

/** Protection against false start codes (like H.264 itself). */
function epbAdd(d) {
  const aus = [];
  let nullen = 0;
  for (const b of d) {
    if (nullen >= 2 && b <= 3) { aus.push(3); nullen = 0; }
    aus.push(b);
    nullen = b === 0 ? nullen + 1 : 0;
  }
  return Uint8Array.from(aus);
}

function epbRemove(d) {
  const aus = [];
  let nullen = 0;
  for (let i = 0; i < d.length; i++) {
    const b = d[i];
    if (nullen >= 2 && b === 3 && i + 1 < d.length && d[i + 1] <= 3) { nullen = 0; continue; }
    aus.push(b);
    nullen = b === 0 ? nullen + 1 : 0;
  }
  return Uint8Array.from(aus);
}

function concat(pieces) {
  let n = 0; for (const s of pieces) n += s.length;
  const aus = new Uint8Array(n);
  let o = 0; for (const s of pieces) { aus.set(s, o); o += s.length; }
  return aus;
}

/** Exp-Golomb ue(v) at a bit offset: [value, next bit] or null. */
function readUe(d, bit) {
  let nullen = 0;
  for (;;) {
    if (bit >= d.length * 8 || nullen > 16) return null;
    if ((d[bit >> 3] >> (7 - (bit & 7))) & 1) break;
    nullen++; bit++;
  }
  bit++;
  let wert = 1;
  for (let i = 0; i < nullen; i++) {
    if (bit >= d.length * 8) return null;
    wert = (wert * 2) + ((d[bit >> 3] >> (7 - (bit & 7))) & 1);
    bit++;
  }
  return [wert - 1, bit];
}

/** pic_parameter_set_id of a slice NAL payload (after the NAL header byte) or null. */
function slicePpsId(payload) {
  const header = payload.subarray(0, 8);
  const a = readUe(header, 0);
  const b = a && readUe(header, a[1]);
  const c = b && readUe(header, b[1]);
  return c ? c[0] : null;
}

/** Blob byte 0 of one encrypted NAL unit (see the video format comment). */
function videoTag(nalTyp, payload) {
  if (nalTyp === 1 || nalTyp === 5) {
    const pps = slicePpsId(payload);
    if (pps !== null && Object.prototype.hasOwnProperty.call(VIDEO_TAG, pps)) return VIDEO_TAG[pps];
  }
  return E2E_MAGIC;
}

/** Encrypt one H.264 frame with an explicit salt + counter (LOW LEVEL). */
async function protectH264(keys, salt, counter, frame) {
  const units = nalUnits(frame);
  if (!units.length) return frame;
  // The NAL index is part of the nonce; it must not repeat inside a frame.
  if (units.length > 0xffff) return new Uint8Array(0);
  const key = await streamKey(keys, 'video', salt);
  const pieces = [];
  for (let nr = 0; nr < units.length; nr++) {
    const [sc, db, ende] = units[nr];
    const header = frame[db];
    pieces.push(frame.slice(sc, db + 1));
    const payload = frame.slice(db + 1, ende);
    if (nalStaysClear(header & 0x1f) || payload.length === 0) { pieces.push(payload); continue; }
    const blobHeader = new Uint8Array(E2E_NAL_KOPF);
    blobHeader[0] = videoTag(header & 0x1f, payload);
    blobHeader.set(salt, 1);
    const dv = new DataView(blobHeader.buffer);
    dv.setUint32(1 + E2E_SALT, counter >>> 0, false);
    dv.setUint16(1 + E2E_SALT + 4, nr, false);
    const aad = new Uint8Array(1 + E2E_NAL_KOPF);
    aad[0] = header;
    aad.set(blobHeader, 1);
    const geheim = new Uint8Array(await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv: nonce(2, nr, salt, counter), additionalData: aad }, key, payload));
    pieces.push(epbAdd(concat([blobHeader, geheim])));
  }
  return concat(pieces);
}

/** Open one encrypted H.264 frame. Returns Uint8Array or null (old/foreign/forged).
 *  Does NOT guard against replays: a receiver uses openH264Head + guard. */
async function openH264(keys, frame) {
  const r = await openH264Head(keys, frame);
  return r ? r.plain : null;
}

/**
 * Like openH264, but returns {plain, salt, counter} (salt/counter = null when
 * the frame has no encrypted NAL unit, e.g. only SPS/PPS). Frame-level rules on
 * top of the per-NAL tags (r60): all encrypted NAL units must name the SAME
 * salt+counter and their NAL indices must strictly increase, otherwise NAL
 * units of different frames could be spliced, repeated or reordered (every tag
 * is valid on its own).
 */
async function openH264Head(keys, frame) {
  const units = nalUnits(frame);
  if (!units.length) return null;
  const pieces = [];
  let frameSalt = null, frameCounter = 0, letzteNr = -1;
  for (const [sc, db, ende] of units) {
    const header = frame[db];
    pieces.push(frame.slice(sc, db + 1));
    const payload = frame.slice(db + 1, ende);
    if (nalStaysClear(header & 0x1f) || payload.length === 0) { pieces.push(payload); continue; }
    const raw = epbRemove(payload);
    // An old (v1) blob starts with the NAL index, not with the version byte.
    if (raw.length < E2E_NAL_KOPF + 16 || !VIDEO_KENNUNGEN.has(raw[0])) return null;
    const salt = raw.slice(1, 1 + E2E_SALT);
    const dv = new DataView(raw.buffer, raw.byteOffset);
    const counter = dv.getUint32(1 + E2E_SALT, false);
    const nr = dv.getUint16(1 + E2E_SALT + 4, false);
    if (frameSalt === null) {
      frameSalt = salt; frameCounter = counter;
    } else if (counter !== frameCounter || hexOf(salt) !== hexOf(frameSalt)) {
      return null;                 // spliced from two frames
    }
    if (nr <= letzteNr) return null;   // repeated or reordered NAL unit
    letzteNr = nr;
    const aad = new Uint8Array(1 + E2E_NAL_KOPF);
    aad[0] = header;
    aad.set(raw.subarray(0, E2E_NAL_KOPF), 1);
    const { key, remember } = await openKey(keys, 'video', salt);
    let plain;
    try {
      plain = new Uint8Array(await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: nonce(2, nr, salt, counter), additionalData: aad },
        key, raw.slice(E2E_NAL_KOPF)));
    } catch (e) {
      return null;
    }
    if (remember) remember();
    pieces.push(plain);
  }
  return { plain: concat(pieces), salt: frameSalt, counter: frameSalt === null ? null : frameCounter };
}

// ------------------------------------------------------- replay guard -----

/**
 * Replay guard with a 64-number window PER SENDER SALT (each stream counts on
 * its own). Remembers the last E2E_GUARD_SALTS_MAX salts; a replay of a stream
 * evicted since is not detected (documented limit). Call it only AFTER a
 * frame decrypted.
 */
const MASKE64 = (1n << 64n) - 1n;
function newGuard() {
  const windows = new Map(); // salt hex -> {highest, bits}
  return function check(salt, z) {
    const id = hexOf(salt);
    let f = windows.get(id);
    if (!f) {
      if (windows.size >= E2E_GUARD_SALTS_MAX) windows.delete(windows.keys().next().value);
      windows.set(id, { highest: z, bits: 0n });
      return true;
    }
    if (z > f.highest) {
      const schritt = z - f.highest;
      // The old highest becomes bit (schritt - 1); a step of exactly 64 keeps
      // only that one (bit 63), a larger step forgets everything. Mask to 64
      // bits so the number cannot grow over a long meeting.
      f.bits = schritt > 64 ? 0n : (((f.bits << BigInt(schritt)) | (1n << BigInt(schritt - 1))) & MASKE64);
      f.highest = z;
      return true;
    }
    const behind = f.highest - z;
    if (behind === 0 || behind > 64) return false;
    const bit = 1n << BigInt(behind - 1);
    if (f.bits & bit) return false;
    f.bits |= bit;
    return true;
  };
}

/**
 * One OUTGOING stream: owns the salt and the counter, so a (key, nonce) pair
 * never repeats. Rotates to a new salt right before the 32-bit counter would
 * wrap. Create one per sender (microphone, camera, screen share, rejoin).
 */
function newSender(salt) {
  let current = salt || newSalt();
  let next = 0;
  return {
    salt: () => current,
    nimm() {
      if (next > 0xffffffff) { current = newSalt(); next = 0; }
      return { salt: current, counter: next++ };
    },
    setzeNaechsten(n) { next = n; },   // test hook
  };
}

/**
 * Attach encryption/decryption to ONE sender or receiver.
 *
 * `art` is 'audio' or 'video'. Audio and video get separate keys, so an audio
 * frame never passes as video. Every call creates its own sender salt.
 */
function protectTrack(keys, art, readable, writable, direction) {
  const isAudio = art === 'audio';
  const sender = newSender();
  const guard = newGuard();

  const durch = new TransformStream({
    async transform(frame, weiter) {
      const daten = new Uint8Array(frame.data);
      try {
        if (direction === 'raus') {
          const { salt, counter } = sender.nimm();
          if (!isAudio) {
            // Fail closed: a frame without any H.264 NAL unit (VP8/VP9 from a
            // browser without H.264) cannot be protected by this format and
            // would go out in the clear - drop it instead.
            if (!nalUnits(daten).length) return;
            // Video: structure stays visible, only the content is encrypted.
            frame.data = (await protectH264(keys, salt, counter, daten)).buffer;
            weiter.enqueue(frame);
            return;
          }
          // Key frame flag: the browser tells us directly (audio frames have
          // no `type`, they are flagged like before).
          const keyframe = frame.type === 'key' || frame.type === undefined;
          frame.data = (await protectAudio(keys, salt, counter, keyframe, daten)).buffer;
          weiter.enqueue(frame);
          return;
        }
        // ---- incoming ----
        if (!isAudio) {
          const offen = await openH264Head(keys, daten);
          if (!offen) return;         // wrong key, forged, or old format
          // Replay guard (r60), only after the tags verified. A frame without
          // an encrypted NAL unit has no counter and is passed on.
          if (offen.salt !== null && !guard(offen.salt, offen.counter)) return;
          const plain = offen.plain;
          frame.data = plain.buffer;
          weiter.enqueue(frame);
          return;
        }
        if (daten.length <= E2E_HEADER + 16 || daten[0] !== E2E_MAGIC) {
          // Not our format: a peer WITHOUT encryption or on the OLD format
          // (0xE2). Passing it through would show media the server could
          // read, so drop it.
          return;
        }
        const offen = await openAudio(keys, daten);
        if (!offen) return;           // wrong key or forged
        if (!guard(offen.salt, offen.counter)) return;   // repeat or ancient
        frame.data = offen.plain.buffer;
        weiter.enqueue(frame);
      } catch (e) {
        // Wrong key or forged: do NOT let it through.
      }
    },
  });
  // Returned so tests can wait for the end of the stream; app.js ignores it.
  return readable.pipeThrough(durch).pipeTo(writable);
}

/** Get the key from the fragment of the link (#k=...).
 *
 * LIVE since 08.08.2026. Two things had to hold before:
 * 1. Only the CONTENT of the NAL units is encrypted, their headers stay
 *    readable - otherwise the media server cannot repack the images into RTP
 *    ("NaluTypeIsNotHandled") and nothing would arrive.
 * 2. End-to-end is a deliberate choice when the meeting is created and the
 *    server negotiates it: only clients that can do it enter an encrypted
 *    room, the rest is refused with a clear message instead of black tiles.
 *
 * A key in the fragment therefore means: encrypt. The extra switch "&e2e=1"
 * is obsolete; it does no harm in an old link.
 */
function keyFromLink() {
  const frag = (location.hash || '').replace(/^#/, '');
  for (const kv of frag.split('&')) {
    if (kv.startsWith('k=')) {
      const b = keyBytes(kv.slice(2));
      if (b) return b;
    }
  }
  return null;
}

/** Build the room key handle from the 32 key bytes. */
async function buildKeys(raw) {
  return derive(raw);
}

/**
 * Can this browser do end-to-end at all?
 *
 * Two paths exist: the legacy Insertable Streams API (createEncodedStreams,
 * Chromium, main thread) and RTCRtpScriptTransform (Firefox, Safari 15.4+),
 * which app.js drives through web/e2e-worker.js. A browser with neither would
 * announce "E2E on" and send unencrypted media (r62), so it gets "no" and the
 * server keeps it out of E2E rooms. Workers are required for the second path.
 */
function supported() {
  if (typeof RTCRtpSender === 'undefined') return false;
  if (typeof RTCRtpSender.prototype.createEncodedStreams === 'function') return true;
  return typeof RTCRtpScriptTransform !== 'undefined' && typeof Worker !== 'undefined';
}


// Expose as a global object - app.js is not an ES module.
window.FreeMeetE2E = {
  protectTrack, keyFromLink, buildKeys, supported,
  keyBytes, E2E_MAGIC, E2E_MAGIC_V1, E2E_HEADER,
  // for the check against the shared test vectors (tests/freemeet-e2e-vectors.json)
  protectAudio, openAudio, protectH264, openH264, openH264Head, nalUnits, epbAdd, epbRemove,
  newSender, newGuard, newSalt, streamKey, nonce, slicePpsId, videoTag, VIDEO_TAG,
};

// Legacy German names, still called by the deployed app.js / e2e-worker.js.
for (const [legacy, name] of [['spurSchuetzen', 'protectTrack'], ['schluesselAusLink', 'keyFromLink'],
  ['schluesselAufbauen', 'buildKeys'], ['moeglich', 'supported']]) {
  window.FreeMeetE2E[legacy] = window.FreeMeetE2E[name];
}
