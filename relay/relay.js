#!/usr/bin/env node
// FreeViewer Relay v1 - dumb, zero-knowledge pipe between host and viewer.
// The relay only knows: host-id <-> connection. It never sees the session password
// and cannot decrypt traffic (end-to-end AES-256-GCM between host and viewer).
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { clientIp } = require('./clientip');
const reqauth = require('./reqauth');
let account = null;
let setups = null;
try { account = require(path.join(__dirname, 'account.js')); } catch (e) { console.log('account module missing:', e.message); }
try { setups = require(path.join(__dirname, 'setups.js')); if (account) setups.init(account); } catch (e) { console.log('setups module missing:', e.message); }
// r7: people with an account keep their friends in the ONE Fleitec address
// book (fleikontakte); buch.js translates. FV_BUCH=0 switches it off.
let buch = null;
if (process.env.FV_BUCH !== '0') {
  try { buch = require(require('path').join(__dirname, 'buch.js')); } catch (e) { console.log('buch module missing:', e.message); }
}

let WebSocket;
try {
  WebSocket = require('ws');
} catch (e) {
  WebSocket = require('/root/jarvis/node_modules/ws'); // deployment fallback
}

const PORT = parseInt(process.env.FV_PORT || '7180', 10);
const DATA = process.env.FV_DATA || '/root/fvrelay/hosts.json';
const MAX_PAYLOAD = 16 * 1024 * 1024;

// ---------- release hosting (auto update) ----------
const DIST = process.env.FV_DIST || '/root/fvrelay/dist';
const MAX_UPLOAD = 128 * 1024 * 1024;
const TOKEN_FILE = process.env.FV_UPLOAD_TOKEN_FILE || '/root/fvrelay/upload.token';
function uploadToken() {
  try { return fs.readFileSync(TOKEN_FILE, 'utf8').trim(); } catch (_) { return null; }
}

// ---------- publish hardening (r2) ----------
// Token only in a header (never the URL: it ends up in proxy/access logs),
// compared in constant time. Publishing is LAN-only: the reverse proxy adds
// X-Forwarded-For / X-Real-IP to every public request, so their presence (or a
// non-private peer address) means "from the internet" -> refuse.
// Rollout switches (see docs/UPDATE-SIGNING.md):
//   FV_PUBLISH_QUERY_TOKEN=1  also accept ?token= (old publish scripts), logs a warning
//   FV_PUBLISH_LAN_ONLY=0     disable the LAN check (not recommended)
//   FV_UPDATE_PUBKEY=<hex>    verify the uploaded signature before publishing
//   FV_REQUIRE_SIG=1          refuse unsigned uploads
const PUBLISH_QUERY_TOKEN = process.env.FV_PUBLISH_QUERY_TOKEN === '1';
const PUBLISH_LAN_ONLY = process.env.FV_PUBLISH_LAN_ONLY !== '0';
const UPDATE_PUBKEY = (process.env.FV_UPDATE_PUBKEY || '').trim().toLowerCase();
const REQUIRE_SIG = process.env.FV_REQUIRE_SIG === '1';

function tokenEquals(given, expected) {
  // hash first: equal length for timingSafeEqual, no length leak
  const a = crypto.createHash('sha256').update(String(given || '')).digest();
  const b = crypto.createHash('sha256').update(String(expected || '')).digest();
  return crypto.timingSafeEqual(a, b);
}
function isPrivateAddr(addr) {
  let a = String(addr || '').toLowerCase();
  if (a.startsWith('::ffff:')) a = a.slice(7);
  if (a === '::1' || a === '127.0.0.1') return true;
  if (/^127\./.test(a) || /^10\./.test(a) || /^192\.168\./.test(a)) return true;
  const m = /^172\.(\d+)\./.exec(a);
  if (m && +m[1] >= 16 && +m[1] <= 31) return true;
  return /^f[cd][0-9a-f]{2}:/.test(a); // IPv6 unique-local
}
function publishFromLan(req) {
  const h = req.headers;
  if (h['x-forwarded-for'] || h['x-real-ip'] || h['forwarded'] || h['cf-connecting-ip']) return false;
  return isPrivateAddr(req.socket.remoteAddress);
}
function publishToken(req, u) {
  const auth = String(req.headers['authorization'] || '');
  if (/^bearer /i.test(auth)) return auth.slice(7).trim();
  if (req.headers['x-upload-token']) return String(req.headers['x-upload-token']);
  if (PUBLISH_QUERY_TOKEN && u.searchParams.get('token')) {
    log('WARNING: publish token in URL (FV_PUBLISH_QUERY_TOKEN=1) - switch the publish script to the Authorization header');
    return u.searchParams.get('token');
  }
  return '';
}
// Same bytes as src/update.rs `signed_message` and tools/sign-release.
function signedMessage(platform, version, sha256, size) {
  return Buffer.from(['freeviewer-update-v1', platform, version, String(sha256).toLowerCase(), String(size)].join('\n'));
}
function verifyUpdateSig(pubHex, msg, sigHex) {
  try {
    if (!/^[0-9a-f]{64}$/.test(pubHex) || !/^[0-9a-f]{128}$/.test(sigHex)) return false;
    const spki = Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(pubHex, 'hex')]);
    const key = crypto.createPublicKey({ key: spki, format: 'der', type: 'spki' });
    return crypto.verify(null, msg, key, Buffer.from(sigHex, 'hex'));
  } catch (_) { return false; }
}
// Version compare ("0.29.10" > "0.29.9"); only digits and dots are expected.
function cmpVersion(a, b) {
  const pa = String(a || '').split('.').map((x) => parseInt(x, 10) || 0);
  const pb = String(b || '').split('.').map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return d < 0 ? -1 : 1;
  }
  return 0;
}
// Atomic feed write: a reader never sees a half-written JSON file.
function writeFeed(feed, obj) {
  const file = path.join(DIST, feed);
  const tmp = file + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(obj, null, 1));
  fs.renameSync(tmp, file);
}
function readFeed(feed) {
  try { return JSON.parse(fs.readFileSync(path.join(DIST, feed), 'utf8')); } catch (_) { return {}; }
}
// Mac packages (r28): the only names a `platform=mac` publish may write.
const MAC_ZIP_RE = /^(FreeViewer|X-Remote)-macOS\.zip$/i;

function safeName(n) {
  n = String(n || '').split(/[\\/]/).pop();
  return /^[A-Za-z0-9._-]{1,64}$/.test(n) ? n : null;
}


// ---------- persistent id directory: sha256(host_secret) -> id ----------
let dir = {};
try { dir = JSON.parse(fs.readFileSync(DATA, 'utf8')); } catch (_) { dir = {}; }
let saveTimer = null;
function saveDir() {
  if (saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    try {
      fs.mkdirSync(path.dirname(DATA), { recursive: true });
      fs.writeFileSync(DATA, JSON.stringify(dir, null, 1));
    } catch (e) { log('save failed: ' + e.message); }
  }, 500);
}
function usedIds() { return new Set(Object.values(dir)); }
function newId() {
  const used = usedIds();
  for (;;) {
    const n = 1000000000 + crypto.randomInt(0, 9000000000); // 10 Stellen wie TeamViewer
    const id = String(n);
    if (!used.has(id)) return id;
  }
}

function log(...a) { console.log(new Date().toISOString(), ...a); }

// ---------- device directory: id -> { name, seen } ----------
// Only what a partner list needs: the machine name a host reports about
// itself and when it was last online. No screens, no passwords - the relay
// still cannot decrypt anything.
const META = process.env.FV_META || path.join(path.dirname(DATA), 'devices.json');
let meta = {};
try { meta = JSON.parse(fs.readFileSync(META, 'utf8')); } catch (_) { meta = {}; }
let metaTimer = null;
function saveMeta() {
  if (metaTimer) return;
  metaTimer = setTimeout(() => {
    metaTimer = null;
    try { fs.writeFileSync(META, JSON.stringify(meta)); } catch (e) { log('meta save failed: ' + e.message); }
  }, 1000);
}
function cleanName(s) {
  return String(s || '').replace(/[^\p{L}\p{N} ._-]/gu, '').slice(0, 40);
}

// ---------- live state ----------
const hosts = new Map();    // id -> ws (host connection)
const sessions = new Map(); // sid -> {host, viewer}
const stats = { started: Date.now(), sessionsTotal: 0, bytes: 0 };

function send(ws, obj) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    try { ws.send(JSON.stringify(obj)); } catch (_) {}
  }
}

function unpair(ws, reason) {
  const peer = ws.fvPeer;
  if (peer) {
    peer.fvPeer = null;
    send(peer, { t: 'peer_gone', reason: reason || 'closed' });
  }
  ws.fvPeer = null;
  if (ws.fvSid) {
    sessions.delete(ws.fvSid);
    if (peer) peer.fvSid = null;
    ws.fvSid = null;
  }
}

// ==========================================================================
// Freunde - Freundschaftsanfragen, Freundesliste, Blockliste
//
// Eine Freundschaft entsteht NUR, wenn beide Seiten zugestimmt haben: einer
// fragt (/fv/freunde/anfragen), der andere nimmt an (/fv/freunde/annehmen).
// Vorher gibt es nur eine offene Anfrage, keine Verbindung.
//
// Wer jemand ist, weist der Anrufer genauso nach wie beim WebSocket
// (host_register): mit dem Geheimnis seiner Installation. Das Relay kennt nur
// dessen SHA-256 (hosts.json) und findet darueber die FreeViewer-ID. Das
// Geheimnis steht IMMER im Rumpf der Anfrage, nie in der Adresse - es soll
// nicht in Zugriffsprotokollen landen. Neue IDs werden hier NICHT vergeben;
// wer noch nie verbunden war, bekommt 401.
//
// OHNE KONTO haengt die Freundesliste an der FreeViewer-ID. MIT KONTO wird
// zusaetzlich der Kontoname (ueber JARVIS, wie in account.js) an der eigenen
// Seite vermerkt - damit sieht dasselbe Konto seine Freunde auf jedem Geraet.
//
// Das Relay gibt NIEMALS die Liste eines anderen heraus: jede Antwort wird
// aus der Sicht des Anrufers gebaut, es gibt keinen Parameter "Liste von X".
//
// Geloescht wird nichts hart - Freundschaften bekommen einen Grabstein
// (entfernt = true), damit der Abgleich auf allen Geraeten aufraeumt.
// ==========================================================================

const FREUNDE = process.env.FV_FREUNDE || path.join(path.dirname(DATA), 'freunde.json');

const FR_KURZ_MS = 60 * 1000;                  // kurzes Spam-Fenster
const FR_KURZ_MAX = 5;                         // so viele Anfragen darin
const FR_TAG_MS = 24 * 3600 * 1000;
const FR_TAG_MAX = 30;                         // so viele Anfragen pro Tag
const FR_MAX_OFFEN_AUS = 50;                   // offene Anfragen, die einer verschickt hat
const FR_MAX_OFFEN_EIN = 200;                  // offene Anfragen, die bei einem liegen
const FR_MAX_FREUNDE = 500;
const FR_ANFRAGE_TTL = 30 * 24 * 3600 * 1000;  // unbeantwortete Anfragen
const FR_GRAB_TTL = 90 * 24 * 3600 * 1000;     // Grabsteine geloeschter Freundschaften
const FR_MAX_BODY = 64 * 1024;
// r32: meeting invitations between friends (a small mailbox, 24 h)
const FR_EINL_TTL = 24 * 3600 * 1000;          // an invitation that nobody picked up
const FR_EINL_MAX_OFFEN = 20;                  // waiting invitations per recipient device
const FR_EINL_KURZ_MS = 60 * 1000;             // burst limit per sender ...
const FR_EINL_KURZ_MAX = 6;                    // ... this many per minute
const FR_EINL_TAG_MAX = 100;                   // ... and per day
const frEinlTakt = new Map();                  // sender fvid -> [timestamps] (memory only)

let fr = null;
let frGeaendert = false;

function frLog(...a) { log('[freunde]', ...a); }
function frMerk() { frGeaendert = true; }

function frLaden() {
  if (fr) return fr;
  try { fr = JSON.parse(fs.readFileSync(FREUNDE, 'utf8')); } catch (_) { fr = null; }
  if (!fr || typeof fr !== 'object') fr = {};
  fr.version = 1;
  if (!Array.isArray(fr.freundschaften)) fr.freundschaften = [];
  if (!Array.isArray(fr.anfragen)) fr.anfragen = [];
  if (!Array.isArray(fr.blocks)) fr.blocks = [];
  if (!Array.isArray(fr.einladungen)) fr.einladungen = [];
  if (!fr.konten || typeof fr.konten !== 'object') fr.konten = {};
  if (!fr.takt || typeof fr.takt !== 'object') fr.takt = {};
  return fr;
}

/** Atomar: erst daneben schreiben, dann umbenennen - eine halbe Datei kann
 *  es damit nie geben, auch nicht bei Stromausfall mitten im Schreiben. */
function frSichern() {
  if (!fr) return;
  try {
    fs.mkdirSync(path.dirname(FREUNDE), { recursive: true });
    const tmp = FREUNDE + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(fr, null, 1));
    fs.renameSync(tmp, FREUNDE);
    try { fs.chmodSync(FREUNDE, 0o600); } catch (_) {}
  } catch (e) { frLog('speichern fehlgeschlagen: ' + e.message); }
}

/** Abgelaufene Anfragen, alte Grabsteine und alte Zaehler wegraeumen. */
function frAufraeumen(db) {
  const jetzt = Date.now();
  const vorher = db.anfragen.length + db.freundschaften.length + db.einladungen.length;
  db.einladungen = db.einladungen.filter((e) => jetzt - (e.wann || 0) < FR_EINL_TTL);
  db.anfragen = db.anfragen.filter((a) => jetzt - (a.wann || 0) < FR_ANFRAGE_TTL);
  db.freundschaften = db.freundschaften.filter(
    (f) => !f.entfernt || jetzt - (f.entfernt_am || 0) < FR_GRAB_TTL);
  for (const k of Object.keys(db.takt)) {
    const v = (db.takt[k] || []).filter((t) => jetzt - t < FR_TAG_MS);
    if (v.length) db.takt[k] = v; else delete db.takt[k];
  }
  return vorher !== db.anfragen.length + db.freundschaften.length + db.einladungen.length;
}

// ------------------------------------------------------------- wer ruft an?

/**
 * Nachweis wie beim host_register: Geheimnis -> SHA-256 -> ID aus hosts.json.
 * Zusaetzlich (freiwillig) ein Konto-Zeichen wie in account.js.
 * cb(null, { fvid, konto }) oder cb(null, null, grund).
 */
function frWer(b, cb) {
  const secret = String((b && b.secret) || '');
  if (!/^[0-9a-fA-F]{32,128}$/.test(secret))
    return cb(null, null, 'Geheimnis fehlt oder ist ungueltig');
  const h = crypto.createHash('sha256').update(secret.toLowerCase(), 'utf8').digest('hex');
  const fvid = dir[h];
  if (!fvid) return cb(null, null, 'Unbekanntes Geraet - bitte zuerst verbinden');
  const token = String((b && b.token) || '');
  if (!token || !account) return cb(null, { fvid, konto: '' });
  account.whoIs(token, (err, user) => {
    // Konto nicht erreichbar oder Zeichen abgelaufen: es geht ohne Konto
    // weiter, die ID allein reicht. Der Anrufer sieht am Feld "konto", ob
    // seine Anmeldung erkannt wurde.
    if (err) return cb(null, { fvid, konto: '' });
    cb(null, { fvid, konto: user ? String(user).toLowerCase() : '' });
  });
}

/** Welches Konto gehoert zu dieser ID? (gelernt, wenn sie angemeldet anfragt) */
function frKontoVon(db, fvid) {
  const e = db.konten[fvid];
  return (e && e.konto) || '';
}

function frKontoMerken(db, ich) {
  if (!ich.konto) return false;
  const alt = db.konten[ich.fvid];
  if (alt && alt.konto === ich.konto) return false;
  db.konten[ich.fvid] = { konto: ich.konto, at: Date.now() };
  return true;
}

/** Alle IDs, die zu mir gehoeren (eigene + alle Geraete meines Kontos). */
function frMeineIds(db, ich) {
  const s = new Set([ich.fvid]);
  if (ich.konto) {
    for (const id of Object.keys(db.konten)) {
      if (db.konten[id] && db.konten[id].konto === ich.konto) s.add(id);
    }
  }
  return [...s];
}

/** Ist diese Seite einer Freundschaft meine? */
function frMeins(seite, ich) {
  if (!seite) return false;
  if (seite.fvid && seite.fvid === ich.fvid) return true;
  return !!(ich.konto && seite.konto && seite.konto === ich.konto);
}

function frAnMich(a, ich) {
  if (a.an_fvid === ich.fvid) return true;
  return !!(ich.konto && a.an_konto && a.an_konto === ich.konto);
}

function frVonMir(a, ich) {
  if (a.von_fvid === ich.fvid) return true;
  return !!(ich.konto && a.von_konto && a.von_konto === ich.konto);
}

function frFreundeVon(db, ich) {
  return db.freundschaften.filter((f) => !f.entfernt && (frMeins(f.a, ich) || frMeins(f.b, ich)));
}

/** Die (lebende) Freundschaft zwischen mir und dieser ID, falls es sie gibt. */
function frFreundschaft(db, ich, anderer) {
  return db.freundschaften.find((f) => !f.entfernt
    && ((frMeins(f.a, ich) && f.b.fvid === anderer) || (frMeins(f.b, ich) && f.a.fvid === anderer)));
}

/** Hat irgendeine ID von A irgendeine ID von B blockiert? */
function frBlockiert(db, a, b) {
  const va = frMeineIds(db, a);
  const vb = frMeineIds(db, b);
  return db.blocks.some((x) => va.includes(x.wer) && vb.includes(x.wen));
}

function frName(fvid, gespeichert) {
  const n = cleanName(gespeichert);
  if (n) return n;
  const e = meta[fvid] || {};
  return cleanName(e.name || '');
}

// ------------------------------------------------------------- Antwortform

function frFreundAus(db, f, ich) {
  const anderer = frMeins(f.a, ich) ? f.b : f.a;
  const e = meta[anderer.fvid] || {};
  return {
    fvid: anderer.fvid,
    name: frName(anderer.fvid, anderer.name),
    seit: Math.floor((f.seit || 0) / 1000),
    online: hosts.has(anderer.fvid),
    gesehen: e.seen || 0,
  };
}

function frAnfrageAus(a, ich) {
  const eingehend = frAnMich(a, ich);
  const gegen = eingehend ? a.von_fvid : a.an_fvid;
  return {
    fvid: gegen,
    name: frName(gegen, eingehend ? a.von_name : ''),
    wann: Math.floor((a.wann || 0) / 1000),
    richtung: eingehend ? 'eingehend' : 'ausgehend',
    nachricht: a.nachricht || '',
    online: hosts.has(gegen),
  };
}

function frLies(req, cb) {
  let n = 0;
  const teile = [];
  req.on('data', (c) => {
    n += c.length;
    if (n > FR_MAX_BODY) { req.destroy(); return; }
    teile.push(c);
  });
  req.on('end', () => {
    try { cb(null, JSON.parse(Buffer.concat(teile).toString('utf8') || '{}')); }
    catch (e) { cb(e); }
  });
  req.on('error', (e) => cb(e));
}

function frAus(res, code, obj) {
  const s = JSON.stringify(obj);
  res.writeHead(code, {
    'content-type': 'application/json',
    'cache-control': 'no-store',
    'content-length': Buffer.byteLength(s),
  });
  res.end(s);
}

/** Antworten und dabei speichern, falls sich etwas geaendert hat. */
function frAntwort(res, code, obj) {
  if (frGeaendert) { frGeaendert = false; frSichern(); }
  frAus(res, code, obj);
}

// ------------------------------------------------------------- die Schritte

/** Eine Freundschaft schliessen (beide haben zugestimmt). */
function frSchliessen(db, a, ich, meinName) {
  // offene Anfragen zwischen den beiden fallen weg (in beide Richtungen)
  db.anfragen = db.anfragen.filter((x) =>
    !(x.von_fvid === a.von_fvid && frAnMich(x, ich)) &&
    !(x.an_fvid === a.von_fvid && frVonMir(x, ich)));
  const da = frFreundschaft(db, ich, a.von_fvid);
  if (da) { frMerk(); return da; }
  const jetzt = Date.now();
  const f = {
    id: crypto.randomBytes(8).toString('hex'),
    a: {
      fvid: a.von_fvid,
      konto: a.von_konto || frKontoVon(db, a.von_fvid),
      name: cleanName(a.von_name),
    },
    b: { fvid: ich.fvid, konto: ich.konto, name: meinName || frName(ich.fvid, '') },
    seit: jetzt,
    at: jetzt,
    entfernt: false,
    entfernt_am: 0,
  };
  db.freundschaften.push(f);
  frMerk();
  frLog('Freundschaft', f.a.fvid, '<->', f.b.fvid);
  return f;
}

function frAnfragen(res, db, ich, b) {
  const an = String(b.an || b.fvid || '').replace(/\D/g, '');
  const meinName = cleanName(b.name);
  const nachricht = String(b.nachricht || '').replace(/[\x00-\x1f]/g, '').slice(0, 200);
  if (!an) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  if (an === ich.fvid) return frAntwort(res, 400, { error: 'Das ist die eigene ID' });
  if (frMeineIds(db, ich).includes(an))
    return frAntwort(res, 400, { error: 'Das ist ein eigenes Geraet' });
  if (!usedIds().has(an))
    return frAntwort(res, 404, { error: 'Diese FreeViewer-ID gibt es nicht' });
  if (frFreundschaft(db, ich, an))
    return frAntwort(res, 409, { error: 'Ihr seid schon befreundet' });

  const ziel = { fvid: an, konto: frKontoVon(db, an) };
  // Wer blockiert wurde, erfaehrt das nicht - sonst wird die Blockliste zum
  // Melder. Nach aussen sieht es aus, als waere die Anfrage nicht moeglich.
  if (frBlockiert(db, ziel, ich))
    return frAntwort(res, 403, { error: 'Anfrage nicht moeglich' });
  if (frBlockiert(db, ich, ziel))
    return frAntwort(res, 403, { error: 'Diese ID steht auf deiner Blockliste' });

  const meine = frMeineIds(db, ich);
  const schon = db.anfragen.find((x) => frVonMir(x, ich) && x.an_fvid === an);
  if (schon)
    return frAntwort(res, 409, { error: 'Anfrage laeuft schon', anfrage: frAnfrageAus(schon, ich) });

  // Der andere hat MICH schon gefragt: dann sind sich beide einig.
  const gegen = db.anfragen.find((x) => x.von_fvid === an && frAnMich(x, ich));
  if (gegen) {
    const f = frSchliessen(db, gegen, ich, meinName);
    return frAntwort(res, 200, {
      ok: true, konto: ich.konto, angenommen: true, freund: frFreundAus(db, f, ich),
    });
  }

  const jetzt = Date.now();
  const takt = (db.takt[ich.fvid] || []).filter((t) => jetzt - t < FR_TAG_MS);
  if (takt.filter((t) => jetzt - t < FR_KURZ_MS).length >= FR_KURZ_MAX)
    return frAntwort(res, 429, { error: 'Zu viele Anfragen - bitte kurz warten' });
  if (takt.length >= FR_TAG_MAX)
    return frAntwort(res, 429, { error: 'Zu viele Anfragen heute' });
  if (db.anfragen.filter((x) => meine.includes(x.von_fvid)).length >= FR_MAX_OFFEN_AUS)
    return frAntwort(res, 429, { error: 'Zu viele offene Anfragen' });
  if (db.anfragen.filter((x) => x.an_fvid === an).length >= FR_MAX_OFFEN_EIN)
    return frAntwort(res, 429, { error: 'Der Empfaenger hat zu viele offene Anfragen' });
  if (frFreundeVon(db, ich).length >= FR_MAX_FREUNDE)
    return frAntwort(res, 429, { error: 'Freundesliste ist voll' });

  const a = {
    id: crypto.randomBytes(8).toString('hex'),
    von_fvid: ich.fvid,
    von_konto: ich.konto,
    von_name: meinName || frName(ich.fvid, ''),
    an_fvid: an,
    an_konto: ziel.konto,
    nachricht,
    wann: jetzt,
  };
  db.anfragen.push(a);
  takt.push(jetzt);
  db.takt[ich.fvid] = takt;
  frMerk();
  frLog('Anfrage', ich.fvid, '->', an);
  frAntwort(res, 200, { ok: true, konto: ich.konto, anfrage: frAnfrageAus(a, ich) });
}

function frOffen(res, db, ich) {
  frNachziehen(db, ich);
  const eingehend = db.anfragen.filter((a) => frAnMich(a, ich)).map((a) => frAnfrageAus(a, ich));
  const ausgehend = db.anfragen.filter((a) => frVonMir(a, ich)).map((a) => frAnfrageAus(a, ich));
  frAntwort(res, 200, { ok: true, fvid: ich.fvid, konto: ich.konto, eingehend, ausgehend });
}

function frAnnehmen(res, db, ich, b) {
  const von = String(b.von || b.fvid || '').replace(/\D/g, '');
  if (!von) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  const a = db.anfragen.find((x) => x.von_fvid === von && frAnMich(x, ich));
  if (!a) {
    // Schon befreundet? Dann war es ein zweiter Klick, kein Fehler.
    const da = frFreundschaft(db, ich, von);
    if (da) return frAntwort(res, 200, { ok: true, konto: ich.konto, freund: frFreundAus(db, da, ich) });
    return frAntwort(res, 404, { error: 'Keine offene Anfrage von dieser ID' });
  }
  const f = frSchliessen(db, a, ich, cleanName(b.name));
  frAntwort(res, 200, { ok: true, konto: ich.konto, freund: frFreundAus(db, f, ich) });
}

/** Ablehnen (eingehend) bzw. die eigene Anfrage zurueckziehen (ausgehend). */
function frAblehnen(res, db, ich, b) {
  const wen = String(b.von || b.fvid || b.an || '').replace(/\D/g, '');
  if (!wen) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  const vorher = db.anfragen.length;
  db.anfragen = db.anfragen.filter((a) =>
    !(a.von_fvid === wen && frAnMich(a, ich)) &&
    !(a.an_fvid === wen && frVonMir(a, ich)));
  const weg = vorher - db.anfragen.length;
  if (weg) frMerk();
  frAntwort(res, 200, { ok: true, konto: ich.konto, weg });
}

function frEntfernen(res, db, ich, b) {
  const wen = String(b.fvid || b.freund || '').replace(/\D/g, '');
  if (!wen) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  const f = frFreundschaft(db, ich, wen);
  if (!f) return frAntwort(res, 404, { error: 'Kein Freund mit dieser ID' });
  f.entfernt = true;
  f.entfernt_am = Date.now();
  f.at = f.entfernt_am;
  frMerk();
  frLog('entfernt', f.a.fvid, '<->', f.b.fvid);
  frAntwort(res, 200, { ok: true, konto: ich.konto });
}

function frBlockieren(res, db, ich, b) {
  const wen = String(b.fvid || '').replace(/\D/g, '');
  const an = b.an === undefined ? true : !!b.an;
  if (!wen) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  if (wen === ich.fvid) return frAntwort(res, 400, { error: 'Sich selbst blockieren geht nicht' });
  const da = db.blocks.findIndex((x) => x.wer === ich.fvid && x.wen === wen);
  if (an && da < 0) {
    db.blocks.push({ wer: ich.fvid, wen, at: Date.now() });
    db.anfragen = db.anfragen.filter((a) =>
      !(a.von_fvid === wen && frAnMich(a, ich)) &&
      !(a.an_fvid === wen && frVonMir(a, ich)));
    frMerk();
  } else if (!an && da >= 0) {
    db.blocks.splice(da, 1);
    frMerk();
  }
  frAntwort(res, 200, { ok: true, konto: ich.konto, blockiert: an });
}

/** Alles in einem Zug - das holt sich der Client beim Abgleich. */
function frStand(res, db, ich) {
  frAntwort(res, 200, frStandDaten(db, ich));
}

function frStandDaten(db, ich) {
  frNachziehen(db, ich);
  const meine = frMeineIds(db, ich);
  return {
    ok: true,
    fvid: ich.fvid,
    konto: ich.konto,
    freunde: frFreundeVon(db, ich).map((f) => frFreundAus(db, f, ich)),
    eingehend: db.anfragen.filter((a) => frAnMich(a, ich)).map((a) => frAnfrageAus(a, ich)),
    ausgehend: db.anfragen.filter((a) => frVonMir(a, ich)).map((a) => frAnfrageAus(a, ich)),
    blockiert: db.blocks.filter((x) => meine.includes(x.wer))
      .map((x) => ({ fvid: x.wen, seit: Math.floor(x.at / 1000) })),
  };
}

/**
 * Konto an der eigenen Seite nachtragen. Wer sich erst nach der Freundschaft
 * anmeldet, sieht sie danach auch auf seinen anderen Geraeten.
 */
function frNachziehen(db, ich) {
  if (!ich.konto) return;
  for (const a of db.anfragen) {
    if (a.an_fvid === ich.fvid && !a.an_konto) { a.an_konto = ich.konto; frMerk(); }
    if (a.von_fvid === ich.fvid && !a.von_konto) { a.von_konto = ich.konto; frMerk(); }
  }
  for (const f of db.freundschaften) {
    for (const s of [f.a, f.b]) {
      if (s.fvid === ich.fvid && s.konto !== ich.konto) { s.konto = ich.konto; frMerk(); }
    }
  }
}

// ------------------------------------------------- r32: Meeting-Einladungen

/**
 * Is `an` (a FreeViewer id) one of my friends' devices? Old store first, then
 * the address book: the book's own /stand answer is captured through the same
 * `alt` hook buch.js uses (no change to buch.js needed).
 * cb(true|false)
 */
function frIstFreundGeraet(db, ich, b, an, cb) {
  if (frFreundschaft(db, ich, an)) return cb(true);
  if (!(buch && b.token)) return cb(false);
  let erledigt = false;
  const fertig = (ok) => { if (!erledigt) { erledigt = true; cb(ok); } };
  const alt = {
    stand: () => frStandDaten(db, ich),
    weiter: () => fertig(false),
    antwort: (_res, code, obj) => {
      if (code !== 200 || !obj) return fertig(false);
      const hit = (obj.freunde || []).some((f) => f && (f.fvid === an
        || (Array.isArray(f.geraete) && f.geraete.some((g) => g && g.fvid === an))));
      fertig(hit);
    },
    zustand: (id) => {
      const e = meta[id] || {};
      return { online: hosts.has(id), gesehen: e.seen || 0, name: cleanName(e.name || '') };
    },
  };
  buch.handle('/fv/freunde/stand', b, ich, null, alt).then((ok) => {
    if (!ok) fertig(false);
  }).catch(() => fertig(false));
}

/** Only a plain https link to a meeting - and NEVER an end-to-end key. */
function frEinlLink(link) {
  const l = String(link || '').trim();
  if (l.length < 12 || l.length > 600) return '';
  if (!/^https:\/\/[A-Za-z0-9.-]+(:\d{1,5})?(\/[^\s\x00-\x1f"'<>\\]*)?$/.test(l)) return '';
  if (l.includes('#')) return ''; // the key lives in the fragment and must never reach the relay
  return l;
}

function frEinladen(res, db, ich, b) {
  const an = String(b.an || b.fvid || '').replace(/\D/g, '');
  const link = frEinlLink(b.link);
  const titel = String(b.titel || '').replace(/[\x00-\x1f]/g, '').trim().slice(0, 80);
  if (!an) return frAntwort(res, 400, { error: 'FreeViewer-ID fehlt' });
  if (!link) return frAntwort(res, 400, { error: 'Link ungueltig (nur https, ohne Schluessel)' });
  if (an === ich.fvid || frMeineIds(db, ich).includes(an))
    return frAntwort(res, 400, { error: 'Das ist ein eigenes Geraet' });
  const jetzt = Date.now();
  const takt = (frEinlTakt.get(ich.fvid) || []).filter((t) => jetzt - t < FR_TAG_MS);
  if (takt.filter((t) => jetzt - t < FR_EINL_KURZ_MS).length >= FR_EINL_KURZ_MAX)
    return frAntwort(res, 429, { error: 'Zu viele Einladungen - bitte kurz warten' });
  if (takt.length >= FR_EINL_TAG_MAX)
    return frAntwort(res, 429, { error: 'Zu viele Einladungen heute' });
  frIstFreundGeraet(db, ich, b, an, (freund) => {
    if (!freund) return frAntwort(res, 403, { error: 'Einladung nicht moeglich - kein Freund' });
    const ziel = { fvid: an, konto: frKontoVon(db, an) };
    // A recipient who blocked me must not learn about it: report success, deliver nothing.
    if (frBlockiert(db, ziel, ich))
      return frAntwort(res, 200, { ok: true, konto: ich.konto, zugestellt: false });
    if (!usedIds().has(an)) return frAntwort(res, 404, { error: 'Diese FreeViewer-ID gibt es nicht' });
    if (db.einladungen.filter((e) => e.an_fvid === an).length >= FR_EINL_MAX_OFFEN)
      return frAntwort(res, 429, { error: 'Der Freund hat zu viele offene Einladungen' });
    // the same link twice in a row to the same device = one entry (refresh)
    const alt = db.einladungen.find((e) => e.an_fvid === an && e.von_fvid === ich.fvid && e.link === link);
    if (alt) { alt.wann = jetzt; alt.titel = titel || alt.titel; }
    else {
      db.einladungen.push({
        id: crypto.randomBytes(8).toString('hex'),
        von_fvid: ich.fvid,
        von_konto: ich.konto,
        von_name: cleanName(b.name) || frName(ich.fvid, ''),
        an_fvid: an,
        an_konto: ziel.konto,
        titel,
        link,
        wann: jetzt,
      });
    }
    takt.push(jetzt);
    frEinlTakt.set(ich.fvid, takt);
    if (frEinlTakt.size > 5000) frEinlTakt.clear();
    frMerk();
    frLog('Einladung', ich.fvid, '->', an);
    frAntwort(res, 200, { ok: true, konto: ich.konto, zugestellt: true });
  });
}

function frEinladungAus(e) {
  return {
    id: e.id,
    fvid: e.von_fvid,
    name: frName(e.von_fvid, e.von_name),
    titel: e.titel || '',
    link: e.link,
    wann: Math.floor((e.wann || 0) / 1000),
    online: hosts.has(e.von_fvid),
  };
}

/** The invitations waiting for me (any device of my account). */
function frEinladungen(res, db, ich) {
  const meine = frMeineIds(db, ich);
  const liste = db.einladungen.filter((e) => meine.includes(e.an_fvid))
    .sort((a, b) => (b.wann || 0) - (a.wann || 0)).map(frEinladungAus);
  frAntwort(res, 200, { ok: true, fvid: ich.fvid, konto: ich.konto, einladungen: liste });
}

/** Mark invitations as seen/handled (remove them). b.id or b.ids[]. */
function frEinladungWeg(res, db, ich, b) {
  const ids = (Array.isArray(b.ids) ? b.ids : [b.id]).map((x) => String(x || '')).filter(Boolean).slice(0, 50);
  const meine = frMeineIds(db, ich);
  const vorher = db.einladungen.length;
  db.einladungen = db.einladungen.filter((e) => !(ids.includes(e.id) && meine.includes(e.an_fvid)));
  const weg = vorher - db.einladungen.length;
  if (weg) frMerk();
  frAntwort(res, 200, { ok: true, konto: ich.konto, weg });
}

// ------------------------------------------------------------------ Routen

/** Gibt true zurueck, wenn die Anfrage hier beantwortet wurde. */
function freundeHandle(req, res) {
  const p = new URL(req.url, 'http://x').pathname;
  if (!p.startsWith('/fv/freunde')) return false;
  if (req.method !== 'POST') { frAus(res, 405, { error: 'nur POST' }); return true; }
  frLies(req, (e, b) => {
    if (e) return frAus(res, 400, { error: 'ungueltige Anfrage' });
    frWer(b, (e2, ich, grund) => {
      if (!ich) return frAus(res, 401, { error: grund || 'nicht angemeldet' });
      frGeaendert = false;
      const db = frLaden();
      if (frAufraeumen(db)) frMerk();
      if (frKontoMerken(db, ich)) frMerk();
      // r32: invitations are relay-local (not part of the address book)
      if (p === '/fv/freunde/einladen' || p === '/fv/freunde/einladungen' || p === '/fv/freunde/einladung-weg') {
        try {
          if (p === '/fv/freunde/einladen') return frEinladen(res, db, ich, b);
          if (p === '/fv/freunde/einladungen') return frEinladungen(res, db, ich);
          return frEinladungWeg(res, db, ich, b);
        } catch (e4) { frLog('Fehler: ' + e4.message); return frAus(res, 500, { error: 'Fehler' }); }
      }
      if (buch && b.token) {
        const alt = {
          stand: () => frStandDaten(db, ich),
          weiter: () => frWeiter(p, res, db, ich, b),
          antwort: frAntwort,
          zustand: (id) => {
            const e = meta[id] || {};
            return { online: hosts.has(id), gesehen: e.seen || 0, name: cleanName(e.name || '') };
          },
        };
        buch.handle(p, b, ich, res, alt).then((erledigt) => {
          if (!erledigt) frWeiter(p, res, db, ich, b);
        }).catch((e3) => {
          frLog('Buch-Fehler: ' + e3.message);
          if (!res.headersSent) frAus(res, 500, { error: 'Fehler' });
        });
        return;
      }
      frWeiter(p, res, db, ich, b);
    });
  });
  return true;
}

/** Der alte Weg: freunde.json (Geraete ohne Konto, IDs ohne Konto). */
function frWeiter(p, res, db, ich, b) {
  try {
    switch (p) {
      case '/fv/freunde/anfragen': return frAnfragen(res, db, ich, b);
      case '/fv/freunde/offen': return frOffen(res, db, ich);
      case '/fv/freunde/annehmen': return frAnnehmen(res, db, ich, b);
      case '/fv/freunde/ablehnen': return frAblehnen(res, db, ich, b);
      case '/fv/freunde/entfernen': return frEntfernen(res, db, ich, b);
      case '/fv/freunde/liste': return frStand(res, db, ich);
      case '/fv/freunde/stand': return frStand(res, db, ich);
      case '/fv/freunde/blockieren': return frBlockieren(res, db, ich, b);
      default: return frAntwort(res, 404, { error: 'unbekannt' });
    }
  } catch (e3) {
    frLog('Fehler: ' + e3.message);
    frAus(res, 500, { error: 'Fehler' });
  }
}

const server = http.createServer((req, res) => {
  // r7: login token via Authorization header; ?token= only as a deprecated fallback (reqauth.js)
  if (req.url.startsWith('/fv/account') || req.url.startsWith('/fv/setup')) {
    if (reqauth.prepare(req, res, clientIp(req), log)) return;
  }
  // optionales Konto (Adressbuch-Abgleich) - siehe account.js
  if (account && req.url.startsWith('/fv/account')) {
    try { if (account.handle(req, res)) return; }
    catch (e) { log('account error: ' + e.message); res.writeHead(500); return res.end('account error'); }
  }
  // Freunde (Anfragen, Freundesliste, Blockliste) - Abschnitt weiter oben
  if (req.url.startsWith('/fv/freunde')) {
    try { if (freundeHandle(req, res)) return; }
    catch (e) { log('freunde error: ' + e.message); res.writeHead(500); return res.end('freunde error'); }
  }
  // Einmal-Links zum Einrichten neuer Geraete - siehe setups.js
  // ---- public info for the setup page (which brand, no secrets) ----
  if (req.method === 'GET' && req.url.startsWith('/fv/setup/info/')) {
    const u = new URL(req.url, 'http://x');
    const code = u.pathname.slice('/fv/setup/info/'.length).replace(/[^A-Za-z0-9_-]/g, '');
    const inf = setups ? setups.info(code) : null;
    if (!inf) { res.writeHead(404, { 'content-type': 'application/json' }); return res.end('{"error":"unbekannt"}'); }
    const body = JSON.stringify(inf);
    res.writeHead(200, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) });
    return res.end(body);
  }

  // ---- personalised setup download: exe with the setup ticket attached ----
  // The setup page lets a fresh machine download FreeViewer with code + name
  // appended (marker FVSETUP1:<json> at the very end of the file). On first
  // start the client finds it and sets itself up - nothing to type.
  if (req.method === 'GET' && req.url.startsWith('/fv/setup/dl/')) {
    const u = new URL(req.url, 'http://x');
    const code = u.pathname.slice('/fv/setup/dl/'.length).replace(/[^A-Za-z0-9_-]/g, '');
    if (!code || !setups || !setups.claimable(code)) {
      res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
      return res.end('Link unbekannt oder schon aufgebraucht');
    }
    const name = (u.searchParams.get('name') || '').replace(/[\x00-\x1f"\\]/g, '').slice(0, 40);
    // Marke des Codes (beim Erzeugen gespeichert) - sonst bekaeme ein
    // Xoffi-Geraet die FreeViewer-Datei. ?brand= bleibt als Uebersteuerung.
    const MARKEN = { freeviewer: 'freeviewer.exe', xoffi: 'x-remote.exe' };
    // r30: the download carries the product name of ITS brand (was always FreeViewer-...)
    const DL_NAMEN = { freeviewer: 'FreeViewer-Einrichtung.exe', xoffi: 'X-Remote-Einrichtung.exe' };
    const marke = (u.searchParams.get('brand') || (setups ? setups.brand_of(code) : 'freeviewer')).toLowerCase();
    const file = path.join(DIST, MARKEN[marke] || 'freeviewer.exe');
    if (!fs.existsSync(file)) { res.writeHead(500); return res.end('build fehlt'); }
    const exe = fs.readFileSync(file);
    const paket = Buffer.from('\nFVSETUP1:' + JSON.stringify({ code, name }), 'utf8');
    const out = Buffer.concat([exe, paket]);
    res.writeHead(200, {
      'content-type': 'application/octet-stream',
      'content-length': out.length,
      'content-disposition': 'attachment; filename="' + (DL_NAMEN[marke] || DL_NAMEN.freeviewer) + '"',
    });
    return res.end(out);
  }

  if (setups && req.url.startsWith('/fv/setup')) {
    try { if (setups.handle(req, res)) return; }
    catch (e) { log('setup error: ' + e.message); res.writeHead(500); return res.end('setup error'); }
  }
  if (req.url === '/fv/health' || req.url === '/health') {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({
      ok: true,
      hosts_online: hosts.size,
      sessions_active: sessions.size,
      sessions_total: stats.sessionsTotal,
      ids_known: Object.keys(dir).length,
      uptime_s: Math.round((Date.now() - stats.started) / 1000),
      relayed_mb: +(stats.bytes / 1048576).toFixed(1),
      account_query_token_hits: reqauth.stats.queryTokenHits,
      online_rate_limited: reqauth.stats.onlineRateLimited,
    }));
    return;
  }
  // ---- what is the newest build? ----
  if (req.method === 'GET' && req.url === '/fv/version') {
    // JEDE MARKE IHREN EIGENEN FEED. Frueher stand hier hart 'version.json':
    // damit bekam auch ein X-Remote-Client die FreeViewer-Datei angeboten und
    // haette sich beim naechsten Update in eine andere Marke verwandelt.
    // Entschieden wird nach der Adresse, ueber die gefragt wurde.
    const host = String(req.headers.host || '').toLowerCase();
    const feed = /(^|\.)remote\.fleitec\.com|xoffi/.test(host) ? 'version-xoffi.json' : 'version.json';
    let j = null;
    try { j = fs.readFileSync(path.join(DIST, feed), 'utf8'); } catch (_) {}
    if (!j) { res.writeHead(404, { 'content-type': 'application/json' }); return res.end('{"error":"no release"}'); }
    res.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    return res.end(j);
  }

  // ---- download a published build ----
  if (req.method === 'GET' && req.url.startsWith('/fv/dist/')) {
    const name = safeName(req.url.slice('/fv/dist/'.length).split('?')[0]);
    const file = name && path.join(DIST, name);
    if (!file || !fs.existsSync(file)) { res.writeHead(404); return res.end('not found'); }
    const st = fs.statSync(file);
    res.writeHead(200, {
      'content-type': 'application/octet-stream',
      'content-length': st.size,
      'content-disposition': 'attachment; filename="' + name + '"',
    });
    return fs.createReadStream(file).pipe(res);
  }

  // ---- publish a new build:  POST /fv/publish?token=..&version=..&name=..&notes=.. ----
  if (req.method === 'POST' && req.url.startsWith('/fv/publish')) {
    const u = new URL(req.url, 'http://x');
    const tok = uploadToken();
    if (PUBLISH_LAN_ONLY && !publishFromLan(req)) { log('publish refused: not from LAN'); res.writeHead(403); return res.end('forbidden'); }
    if (!tok || !tokenEquals(publishToken(req, u), tok)) { res.writeHead(403); return res.end('forbidden'); }
    const name = safeName(u.searchParams.get('name') || 'freeviewer.exe');
    const version = String(u.searchParams.get('version') || '').slice(0, 32);
    const notes = String(u.searchParams.get('notes') || '').slice(0, 500);
    if (!name || !/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) { res.writeHead(400); return res.end('bad name/version'); }
    // platform=mac (r28): store the Mac zip and write ONLY the mac_* fields of the feed.
    // The win fields (version/sha256/size/sig) are never touched, so a Mac publish
    // cannot break the Windows signature - and a Windows publish cannot break mac_sig,
    // because the Mac signature binds mac_version, not the Windows version.
    const isMac = u.searchParams.get('platform') === 'mac';
    if (isMac && !MAC_ZIP_RE.test(name)) { res.writeHead(400); return res.end('bad mac package name'); }
    // Signature of the package (hex Ed25519 over signedMessage), made offline
    // with tools/sign-release; the relay only checks it and passes it through.
    const sigHex = String(req.headers['x-fv-sig'] || '').trim().toLowerCase();
    if (sigHex && !/^[0-9a-f]{128}$/.test(sigHex)) { res.writeHead(400); return res.end('bad signature format'); }
    if (!sigHex && REQUIRE_SIG) { res.writeHead(400); return res.end('signature required'); }
    fs.mkdirSync(DIST, { recursive: true });
    const tmp = path.join(DIST, name + '.upload');
    const out = fs.createWriteStream(tmp);
    const hash = crypto.createHash('sha256');
    let size = 0, bad = false;
    let head = Buffer.alloc(0);
    req.on('data', (c) => {
      if (head.length < 4) head = Buffer.concat([head, c.subarray(0, 4 - head.length)]);
      size += c.length;
      if (size > MAX_UPLOAD) { bad = true; req.destroy(); return; }
      hash.update(c);
      out.write(c);
    });
    req.on('end', () => {
      out.end(() => {
        if (bad) { try { fs.unlinkSync(tmp); } catch (_) {} res.writeHead(413); return res.end('too big'); }
        const sha = hash.digest('hex');
        if (isMac && head.toString('latin1') !== 'PK\x03\x04') {
          try { fs.unlinkSync(tmp); } catch (_) {}
          log('publish refused: mac package is not a zip', name);
          res.writeHead(400); return res.end('not a zip');
        }
        if (isMac) {
          // never move the Mac package backwards (same version = re-upload is fine)
          const cur = readFeed(/^x-remote/i.test(name) ? 'version-xoffi.json' : 'version.json').mac_version;
          if (cur && cmpVersion(version, cur) < 0) {
            try { fs.unlinkSync(tmp); } catch (_) {}
            log('publish refused: mac version older than the feed', version, cur);
            res.writeHead(409); return res.end('older than mac_version ' + cur);
          }
        }
        if (sigHex && UPDATE_PUBKEY && !verifyUpdateSig(UPDATE_PUBKEY, signedMessage(isMac ? 'mac' : 'win', version, sha, size), sigHex)) {
          try { fs.unlinkSync(tmp); } catch (_) {}
          log('publish refused: signature does not verify', version, name);
          res.writeHead(400); return res.end('signature invalid');
        }
        fs.renameSync(tmp, path.join(DIST, name));
        const meta = {
          version, sha256: sha, size, notes,
          file: name,
          // Jede Marke zeigt auf ihre eigene Adresse.
          url: (/^x-remote/i.test(name) ? 'https://remote.fleitec.com/fv/dist/'
                                        : 'https://freeviewer.fleitec.com/fv/dist/') + name,
          published: new Date().toISOString(),
        };
        if (isMac) {
          const feed = /^x-remote/i.test(name) ? 'version-xoffi.json' : 'version.json';
          const cur = readFeed(feed);
          cur.mac_version = version;
          cur.mac_url = meta.url;
          cur.mac_sha256 = sha;
          cur.mac_size = size;
          cur.mac_published = meta.published;
          if (sigHex) cur.mac_sig = sigHex; else delete cur.mac_sig; // no stale signature of an older zip
          writeFeed(feed, cur);
          log('published mac', version, name, size + ' bytes', sha.slice(0, 12));
          res.writeHead(200, { 'content-type': 'application/json' });
          return res.end(JSON.stringify({ platform: 'mac', mac_version: version, mac_sha256: sha, mac_size: size, mac_url: meta.url, mac_sig: sigHex || undefined }));
        }
        if (sigHex) meta.sig = sigHex;
        // noversion=1: nur die Datei ablegen (z. B. Mac-Zip aus der CI) -
        // version.json bleibt dem Haupt-Build vorbehalten.
        if (u.searchParams.get('noversion') !== '1') {
          // Feed je nach Datei: x-remote.* -> version-xoffi.json (mac_*-Felder erhalten)
          const feed = /^x-remote/i.test(name) ? 'version-xoffi.json' : 'version.json';
          // Mac-Felder gehoeren dem Mac-Build: sie bleiben IMMER erhalten,
          // egal welche Marke gerade eine Windows-Exe veroeffentlicht.
          let outmeta = Object.assign({}, meta);
          try {
            const old = JSON.parse(fs.readFileSync(path.join(DIST, feed), 'utf8'));
            for (const k of Object.keys(old)) { if (k.startsWith('mac_')) outmeta[k] = old[k]; }
          } catch (_) {}
          writeFeed(feed, outmeta);
        }
        log('published', version, name, size + ' bytes', sha.slice(0, 12));
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(JSON.stringify(meta));
      });
    });
    req.on('error', () => { try { fs.unlinkSync(tmp); } catch (_) {} });
    return;
  }

  // ---- who is online:  GET /fv/online?ids=1,2,3 ----
  // r6: rate limited; name/seen only for a signed-in caller and only for IDs in that
  // caller's own address book, everybody else gets just {online} (see reqauth.js).
  if (req.method === 'GET' && (req.url === '/fv/online' || req.url.startsWith('/fv/online?'))) {
    reqauth.onlineReply(req, clientIp(req), hosts, meta, account ? account.whoIs : null, (code, body, headers) => {
      res.writeHead(code, Object.assign({ 'content-type': 'application/json', 'cache-control': 'no-store' }, headers));
      res.end(JSON.stringify(body));
    });
    return;
  }
  res.writeHead(404); res.end('freeviewer-relay');
});

const wss = new WebSocket.Server({ server, path: '/fv/ws', maxPayload: MAX_PAYLOAD });

wss.on('connection', (ws, req) => {
  ws.fvRole = null;
  ws.fvId = null;
  ws.fvPeer = null;
  ws.fvSid = null;
  ws.isAlive = true;
  ws.on('pong', () => { ws.isAlive = true; });

  ws.on('message', (data, isBinary) => {
    // binary payloads are relayed verbatim (encrypted end-to-end)
    if (isBinary) {
      const peer = ws.fvPeer;
      if (peer && peer.readyState === WebSocket.OPEN) {
        stats.bytes += data.length;
        peer.send(data, { binary: true });
      }
      return;
    }
    let m;
    try { m = JSON.parse(data.toString()); } catch (_) { return send(ws, { t: 'error', msg: 'bad_json' }); }

    switch (m.t) {
      case 'host_register': {
        if (typeof m.secret !== 'string' || !/^[0-9a-fA-F]{32,128}$/.test(m.secret))
          return send(ws, { t: 'error', msg: 'bad_secret' });
        const h = crypto.createHash('sha256').update(m.secret.toLowerCase(), 'utf8').digest('hex');
        let id = dir[h];
        if (!id) { id = newId(); dir[h] = id; saveDir(); }
        const old = hosts.get(id);
        if (old && old !== ws) { send(old, { t: 'replaced' }); try { old.close(4001, 'replaced'); } catch (_) {} }
        ws.fvRole = 'host';
        ws.fvId = id;
        hosts.set(id, ws);
        {
          const nm = cleanName(m.name);
          const e = meta[id] || {};
          if (nm) e.name = nm;
          e.seen = Date.now();
          meta[id] = e;
          saveMeta();
        }
        log('host online', id, req.socket.remoteAddress);
        return send(ws, { t: 'registered', id });
      }

      case 'connect': {
        const id = String(m.id || '').replace(/\D/g, '');
        const host = hosts.get(id);
        if (!host || host.readyState !== WebSocket.OPEN)
          return send(ws, { t: 'error', msg: 'offline' });
        if (host.fvPeer)
          return send(ws, { t: 'error', msg: 'busy' });
        const sid = crypto.randomBytes(8).toString('hex');
        ws.fvRole = 'viewer';
        ws.fvPeer = host; host.fvPeer = ws;
        ws.fvSid = sid; host.fvSid = sid;
        sessions.set(sid, { host, viewer: ws });
        stats.sessionsTotal++;
        log('paired', id, sid);
        // `from` stays the direct peer (old hosts); `client_ip` is the real viewer
        // address behind the proxy (r63), optional for hosts that do not know it.
        send(host, { t: 'incoming', sid, from: req.socket.remoteAddress, client_ip: clientIp(req) });
        return send(ws, { t: 'paired', sid, id });
      }

      case 'bye':
        unpair(ws, m.reason || 'bye');
        return;

      case 'ping':
        return send(ws, { t: 'pong', ts: m.ts || 0 });

      default:
        return send(ws, { t: 'error', msg: 'unknown_type' });
    }
  });

  ws.on('close', () => {
    if (ws.fvRole === 'host' && ws.fvId && hosts.get(ws.fvId) === ws) {
      hosts.delete(ws.fvId);
      if (meta[ws.fvId]) { meta[ws.fvId].seen = Date.now(); saveMeta(); }
      log('host offline', ws.fvId);
    }
    unpair(ws, 'closed');
  });

  ws.on('error', () => {});
});

setInterval(() => {
  wss.clients.forEach((ws) => {
    if (ws.isAlive === false) { try { ws.terminate(); } catch (_) {} return; }
    ws.isAlive = false;
    try { ws.ping(); } catch (_) {}
  });
}, 25000);

server.listen(PORT, '0.0.0.0', () => log('freeviewer-relay listening on :' + PORT + ' (ws path /fv/ws)'));
