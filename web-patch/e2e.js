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
//   byte 0 version 0xE3 | 1..9 salt | 9..13 counter | 13..15 NAL index | tag...
//   associated data = NAL header byte + the 15 byte blob header.

const E2E_KENNUNG = 0xE3;
const E2E_KENNUNG_V1 = 0xE2;
const E2E_KOPF = 14;
const E2E_SALZ = 8;
const E2E_NAL_KOPF = 1 + E2E_SALZ + 4 + 2;
const E2E_KDF_SALZ = 'freemeet-e2e-v2';
const E2E_KEY_CACHE_MAX = 64;
const E2E_WACHE_SALZE_MAX = 16;

/** 43 characters of URL-safe base64 -> 32 bytes. */
function schluesselBytes(text) {
  const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  const roh = String(text || '').trim().replace(/=+$/, '');
  const aus = [];
  for (let i = 0; i < roh.length; i += 4) {
    const st = roh.slice(i, i + 4);
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

const hexVon = (u8) => Array.from(u8, (b) => b.toString(16).padStart(2, '0')).join('');

/**
 * Room key handle. Holds the HKDF base key and a cache of derived stream
 * keys. Opaque to app.js (it only passes it to spurSchuetzen).
 */
async function ableiten(roh) {
  const basis = await crypto.subtle.importKey('raw', roh, 'HKDF', false, ['deriveBits']);
  return { basis, cache: new Map(), version: 2 };
}

/** Derive the AES-GCM key of one stream. `art` is 'audio' or 'video'. */
async function streamKey(schluessel, art, salz) {
  const kind = art === 'audio' ? 'ton' : 'bild';
  const id = kind + ':' + hexVon(salz);
  const hit = schluessel.cache.get(id);
  if (hit) return hit;
  const key = await deriveStreamKey(schluessel, kind, salz);
  rememberKey(schluessel, id, key);
  return key;
}

async function deriveStreamKey(schluessel, kind, salz) {
  const enc = new TextEncoder();
  const info = new Uint8Array(kind.length + 1 + E2E_SALZ);
  info.set(enc.encode(kind), 0);
  info[kind.length] = 0;
  info.set(salz, kind.length + 1);
  const bits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: enc.encode(E2E_KDF_SALZ), info },
    schluessel.basis, 256);
  return crypto.subtle.importKey('raw', bits, 'AES-GCM', false, ['encrypt', 'decrypt']);
}

function rememberKey(schluessel, id, key) {
  // Starting over is always safe: keys are re-derived on demand.
  if (schluessel.cache.size >= E2E_KEY_CACHE_MAX) schluessel.cache.clear();
  schluessel.cache.set(id, key);
}

/**
 * Key for OPENING a frame. The salt is attacker-controlled until the tag
 * checks out, so a freshly derived key is only remembered by the caller after
 * a successful decryption (see `merken`).
 */
async function oeffnenKey(schluessel, art, salz) {
  const kind = art === 'audio' ? 'ton' : 'bild';
  const id = kind + ':' + hexVon(salz);
  const hit = schluessel.cache.get(id);
  if (hit) return { key: hit, merken: null };
  const key = await deriveStreamKey(schluessel, kind, salz);
  return { key, merken: () => rememberKey(schluessel, id, key) };
}

/** Fresh random sender salt. */
function neuesSalz() {
  const s = new Uint8Array(E2E_SALZ);
  crypto.getRandomValues(s);
  return s;
}

/** GCM nonce: kind | NAL index | first 5 salt bytes | counter. */
function nonce(spurNr, nal, salz, zaehler) {
  const n = new Uint8Array(12);
  n[0] = spurNr;
  new DataView(n.buffer).setUint16(1, nal, false);
  n.set(salz.subarray(0, 5), 3);
  new DataView(n.buffer).setUint32(8, zaehler >>> 0, false);
  return n;
}

// ---------------------------------------------------------- audio ---------

/** Encrypt one audio frame with an explicit salt + counter (LOW LEVEL). */
async function schuetzenTon(schluessel, salz, zaehler, schluesselbild, klar) {
  const key = await streamKey(schluessel, 'audio', salz);
  const kopf = new Uint8Array(E2E_KOPF);
  kopf[0] = E2E_KENNUNG;
  kopf[1] = schluesselbild ? 0x01 : 0x00;
  kopf.set(salz, 2);
  new DataView(kopf.buffer).setUint32(2 + E2E_SALZ, zaehler >>> 0, false);
  const geheim = new Uint8Array(await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv: nonce(1, 0, salz, zaehler), additionalData: kopf }, key, klar));
  const aus = new Uint8Array(E2E_KOPF + geheim.length);
  aus.set(kopf, 0);
  aus.set(geheim, E2E_KOPF);
  return aus;
}

/** Header fields of an audio frame, or null if it is not a v2 frame. */
function tonKopf(daten) {
  if (daten.length <= E2E_KOPF + 16 || daten[0] !== E2E_KENNUNG) return null;
  return {
    salz: daten.slice(2, 2 + E2E_SALZ),
    zaehler: new DataView(daten.buffer, daten.byteOffset).getUint32(2 + E2E_SALZ, false),
  };
}

/** Decrypt one audio frame. Returns {klar, salz, zaehler} or null. */
async function oeffnenTon(schluessel, daten) {
  const k = tonKopf(daten);
  if (!k) return null;
  const { key, merken } = await oeffnenKey(schluessel, 'audio', k.salz);
  let klar;
  try {
    klar = new Uint8Array(await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: nonce(1, 0, k.salz, k.zaehler),
        additionalData: daten.slice(0, E2E_KOPF) },
      key, daten.slice(E2E_KOPF)));
  } catch (e) {
    return null;
  }
  if (merken) merken();
  return { klar, salz: k.salz, zaehler: k.zaehler };
}

// ------------------------------------------------------- H.264 (video) ----
//
// The media server repacks images into RTP and must understand the H.264
// structure (measured 07.08.2026: a fully encrypted payload fails with
// "NaluTypeIsNotHandled"). So the STRUCTURE stays visible and only the
// CONTENT is encrypted. Must match meete2e.rs byte for byte.

function naechsterStartcode(d, ab) {
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

function nalEinheiten(d) {
  const aus = [];
  let suche = 0, offen = null;
  for (;;) {
    const t = naechsterStartcode(d, suche);
    if (!t) break;
    if (offen) aus.push([offen[0], offen[1], t[0]]);
    offen = t;
    suche = t[1];
  }
  if (offen) aus.push([offen[0], offen[1], d.length]);
  return aus;
}

/** SPS, PPS and AUD stay in the clear - the packer in the server needs them. */
const nalBleibtKlar = (typ) => typ === 7 || typ === 8 || typ === 9;

/** Protection against false start codes (like H.264 itself). */
function epbEin(d) {
  const aus = [];
  let nullen = 0;
  for (const b of d) {
    if (nullen >= 2 && b <= 3) { aus.push(3); nullen = 0; }
    aus.push(b);
    nullen = b === 0 ? nullen + 1 : 0;
  }
  return Uint8Array.from(aus);
}

function epbRaus(d) {
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

function zusammen(stuecke) {
  let n = 0; for (const s of stuecke) n += s.length;
  const aus = new Uint8Array(n);
  let o = 0; for (const s of stuecke) { aus.set(s, o); o += s.length; }
  return aus;
}

/** Encrypt one H.264 frame with an explicit salt + counter (LOW LEVEL). */
async function schuetzenH264(schluessel, salz, zaehler, rahmen) {
  const teile = nalEinheiten(rahmen);
  if (!teile.length) return rahmen;
  // The NAL index is part of the nonce; it must not repeat inside a frame.
  if (teile.length > 0xffff) return new Uint8Array(0);
  const key = await streamKey(schluessel, 'video', salz);
  const stuecke = [];
  for (let nr = 0; nr < teile.length; nr++) {
    const [sc, db, ende] = teile[nr];
    const kopf = rahmen[db];
    stuecke.push(rahmen.slice(sc, db + 1));
    const inhalt = rahmen.slice(db + 1, ende);
    if (nalBleibtKlar(kopf & 0x1f) || inhalt.length === 0) { stuecke.push(inhalt); continue; }
    const blobKopf = new Uint8Array(E2E_NAL_KOPF);
    blobKopf[0] = E2E_KENNUNG;
    blobKopf.set(salz, 1);
    const dv = new DataView(blobKopf.buffer);
    dv.setUint32(1 + E2E_SALZ, zaehler >>> 0, false);
    dv.setUint16(1 + E2E_SALZ + 4, nr, false);
    const aad = new Uint8Array(1 + E2E_NAL_KOPF);
    aad[0] = kopf;
    aad.set(blobKopf, 1);
    const geheim = new Uint8Array(await crypto.subtle.encrypt(
      { name: 'AES-GCM', iv: nonce(2, nr, salz, zaehler), additionalData: aad }, key, inhalt));
    stuecke.push(epbEin(zusammen([blobKopf, geheim])));
  }
  return zusammen(stuecke);
}

/** Open one encrypted H.264 frame. Returns Uint8Array or null (old/foreign/forged).
 *  Does NOT guard against replays: a receiver uses oeffnenH264Kopf + wache. */
async function oeffnenH264(schluessel, rahmen) {
  const r = await oeffnenH264Kopf(schluessel, rahmen);
  return r ? r.klar : null;
}

/**
 * Like oeffnenH264, but returns {klar, salz, zaehler} (salz/zaehler = null when
 * the frame has no encrypted NAL unit, e.g. only SPS/PPS). Frame-level rules on
 * top of the per-NAL tags (r60): all encrypted NAL units must name the SAME
 * salt+counter and their NAL indices must strictly increase, otherwise NAL
 * units of different frames could be spliced, repeated or reordered (every tag
 * is valid on its own).
 */
async function oeffnenH264Kopf(schluessel, rahmen) {
  const teile = nalEinheiten(rahmen);
  if (!teile.length) return null;
  const stuecke = [];
  let frameSalz = null, frameZaehler = 0, letzteNr = -1;
  for (const [sc, db, ende] of teile) {
    const kopf = rahmen[db];
    stuecke.push(rahmen.slice(sc, db + 1));
    const inhalt = rahmen.slice(db + 1, ende);
    if (nalBleibtKlar(kopf & 0x1f) || inhalt.length === 0) { stuecke.push(inhalt); continue; }
    const roh = epbRaus(inhalt);
    // An old (v1) blob starts with the NAL index, not with the version byte.
    if (roh.length < E2E_NAL_KOPF + 16 || roh[0] !== E2E_KENNUNG) return null;
    const salz = roh.slice(1, 1 + E2E_SALZ);
    const dv = new DataView(roh.buffer, roh.byteOffset);
    const zaehler = dv.getUint32(1 + E2E_SALZ, false);
    const nr = dv.getUint16(1 + E2E_SALZ + 4, false);
    if (frameSalz === null) {
      frameSalz = salz; frameZaehler = zaehler;
    } else if (zaehler !== frameZaehler || hexVon(salz) !== hexVon(frameSalz)) {
      return null;                 // spliced from two frames
    }
    if (nr <= letzteNr) return null;   // repeated or reordered NAL unit
    letzteNr = nr;
    const aad = new Uint8Array(1 + E2E_NAL_KOPF);
    aad[0] = kopf;
    aad.set(roh.subarray(0, E2E_NAL_KOPF), 1);
    const { key, merken } = await oeffnenKey(schluessel, 'video', salz);
    let klar;
    try {
      klar = new Uint8Array(await crypto.subtle.decrypt(
        { name: 'AES-GCM', iv: nonce(2, nr, salz, zaehler), additionalData: aad },
        key, roh.slice(E2E_NAL_KOPF)));
    } catch (e) {
      return null;
    }
    if (merken) merken();
    stuecke.push(klar);
  }
  return { klar: zusammen(stuecke), salz: frameSalz, zaehler: frameSalz === null ? null : frameZaehler };
}

// ------------------------------------------------------- replay guard -----

/**
 * Replay guard with a 64-number window PER SENDER SALT (each stream counts on
 * its own). Remembers the last E2E_WACHE_SALZE_MAX salts; a replay of a stream
 * evicted since is not detected (documented limit). Call it only AFTER a
 * frame decrypted.
 */
const MASKE64 = (1n << 64n) - 1n;
function neueWache() {
  const fenster = new Map(); // salt hex -> {hoechste, bits}
  return function pruefen(salz, z) {
    const id = hexVon(salz);
    let f = fenster.get(id);
    if (!f) {
      if (fenster.size >= E2E_WACHE_SALZE_MAX) fenster.delete(fenster.keys().next().value);
      fenster.set(id, { hoechste: z, bits: 0n });
      return true;
    }
    if (z > f.hoechste) {
      const schritt = z - f.hoechste;
      // The old highest becomes bit (schritt - 1); a step of exactly 64 keeps
      // only that one (bit 63), a larger step forgets everything. Mask to 64
      // bits so the number cannot grow over a long meeting.
      f.bits = schritt > 64 ? 0n : (((f.bits << BigInt(schritt)) | (1n << BigInt(schritt - 1))) & MASKE64);
      f.hoechste = z;
      return true;
    }
    const zurueck = f.hoechste - z;
    if (zurueck === 0 || zurueck > 64) return false;
    const bit = 1n << BigInt(zurueck - 1);
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
function neuerSender(salz) {
  let aktuell = salz || neuesSalz();
  let naechster = 0;
  return {
    salz: () => aktuell,
    nimm() {
      if (naechster > 0xffffffff) { aktuell = neuesSalz(); naechster = 0; }
      return { salz: aktuell, zaehler: naechster++ };
    },
    setzeNaechsten(n) { naechster = n; },   // test hook
  };
}

/**
 * Attach encryption/decryption to ONE sender or receiver.
 *
 * `art` is 'audio' or 'video'. Audio and video get separate keys, so an audio
 * frame never passes as video. Every call creates its own sender salt.
 */
function spurSchuetzen(schluessel, art, lesbar, schreibbar, richtung) {
  const istTon = art === 'audio';
  const sender = neuerSender();
  const wache = neueWache();

  const durch = new TransformStream({
    async transform(rahmen, weiter) {
      const daten = new Uint8Array(rahmen.data);
      try {
        if (richtung === 'raus') {
          const { salz, zaehler } = sender.nimm();
          if (!istTon) {
            // Fail closed: a frame without any H.264 NAL unit (VP8/VP9 from a
            // browser without H.264) cannot be protected by this format and
            // would go out in the clear - drop it instead.
            if (!nalEinheiten(daten).length) return;
            // Video: structure stays visible, only the content is encrypted.
            rahmen.data = (await schuetzenH264(schluessel, salz, zaehler, daten)).buffer;
            weiter.enqueue(rahmen);
            return;
          }
          // Key frame flag: the browser tells us directly (audio frames have
          // no `type`, they are flagged like before).
          const schluesselbild = rahmen.type === 'key' || rahmen.type === undefined;
          rahmen.data = (await schuetzenTon(schluessel, salz, zaehler, schluesselbild, daten)).buffer;
          weiter.enqueue(rahmen);
          return;
        }
        // ---- incoming ----
        if (!istTon) {
          const offen = await oeffnenH264Kopf(schluessel, daten);
          if (!offen) return;         // wrong key, forged, or old format
          // Replay guard (r60), only after the tags verified. A frame without
          // an encrypted NAL unit has no counter and is passed on.
          if (offen.salz !== null && !wache(offen.salz, offen.zaehler)) return;
          const klar = offen.klar;
          rahmen.data = klar.buffer;
          weiter.enqueue(rahmen);
          return;
        }
        if (daten.length <= E2E_KOPF + 16 || daten[0] !== E2E_KENNUNG) {
          // Not our format: a peer WITHOUT encryption or on the OLD format
          // (0xE2). Passing it through would show media the server could
          // read, so drop it.
          return;
        }
        const offen = await oeffnenTon(schluessel, daten);
        if (!offen) return;           // wrong key or forged
        if (!wache(offen.salz, offen.zaehler)) return;   // repeat or ancient
        rahmen.data = offen.klar.buffer;
        weiter.enqueue(rahmen);
      } catch (e) {
        // Wrong key or forged: do NOT let it through.
      }
    },
  });
  // Returned so tests can wait for the end of the stream; app.js ignores it.
  return lesbar.pipeThrough(durch).pipeTo(schreibbar);
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
function schluesselAusLink() {
  const frag = (location.hash || '').replace(/^#/, '');
  for (const kv of frag.split('&')) {
    if (kv.startsWith('k=')) {
      const b = schluesselBytes(kv.slice(2));
      if (b) return b;
    }
  }
  return null;
}

/** Build the room key handle from the 32 key bytes. */
async function schluesselAufbauen(roh) {
  return ableiten(roh);
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
function moeglich() {
  if (typeof RTCRtpSender === 'undefined') return false;
  if (typeof RTCRtpSender.prototype.createEncodedStreams === 'function') return true;
  return typeof RTCRtpScriptTransform !== 'undefined' && typeof Worker !== 'undefined';
}


// Expose as a global object - app.js is not an ES module.
window.FreeMeetE2E = {
  spurSchuetzen, schluesselAusLink, schluesselAufbauen, moeglich,
  schluesselBytes, E2E_KENNUNG, E2E_KENNUNG_V1, E2E_KOPF,
  // for the check against the shared test vectors (tests/freemeet-e2e-vectors.json)
  schuetzenTon, oeffnenTon, schuetzenH264, oeffnenH264, oeffnenH264Kopf, nalEinheiten, epbEin, epbRaus,
  neuerSender, neueWache, neuesSalz, streamKey, nonce,
};
