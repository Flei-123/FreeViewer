// buch.js - FreeViewer friends out of the ONE Fleitec address book.
//
// Since r7 (24.09.2026) the relay no longer keeps its own friend list for
// people with an account. Who signs in (token in the body, a JARVIS JWT)
// gets his friends, requests and blocks from fleikontakte
// (/api/kontakte/*), the address book that FirnChat and the contacts page
// use as well. A friendship made in FreeViewer is the same friendship in
// FirnChat, and the other way round.
//
// WHAT STAYS OLD: a device WITHOUT an account has no book to live in. For
// it, and for the friendships between an account and such a bare id, the
// old store (freunde.json, relay.js) keeps running unchanged. The answer
// of /fv/freunde/stand for an account is therefore the book PLUS those old
// entries whose other side has no account (marked quelle:"fv").
//
// WHICH FREEVIEWER ID BELONGS TO WHOM is only believed when the relay saw
// it itself: a device that proved its secret (host_register hash) AND
// came with a token the address book accepts. That pair is written to
// buch-geraete.json and the id is put into the account's profile as a
// device (g=fv:<id>). An id somebody typed into his profile by hand is
// never used to route a request or to show a friend's PC -- otherwise
// anybody could claim a stranger's FreeViewer id.
//
// The answer keeps the old shape (fvid, name, seit, online, gesehen ...)
// so that FreeViewer 0.29 goes on working; new fields for newer clients:
// person (the account id), handle (@name), geraete[] (every PC of that
// person), quelle ("buch" | "fv"). A person without a FreeViewer PC has
// fvid "" -- a newer client keys it by "k:<person>".
'use strict';

const fs = require('fs');
const path = require('path');
const http = require('http');
const https = require('https');

const CONTACTS_URL = (process.env.FV_KONTAKTE || 'https://chat.fleitec.com').replace(/\/$/, '');
const DEVICES_FILE = process.env.FV_BUCH_GERAETE || path.join(__dirname, 'buch-geraete.json');
const PROFIL_MS = 5 * 60 * 1000;
const TIMEOUT_MS = 8000;

function log(...a) { console.log(new Date().toISOString(), '[buch]', ...a); }

// ------------------------------------------------ verified id -> account

let map = null; // fvid -> { uid, at }

function loadMap() {
  if (map) return map;
  try { map = JSON.parse(fs.readFileSync(DEVICES_FILE, 'utf8')); } catch (_) { map = null; }
  if (!map || typeof map !== 'object') map = {};
  return map;
}

function saveMap() {
  try {
    const tmp = DEVICES_FILE + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(map, null, 1));
    fs.renameSync(tmp, DEVICES_FILE);
    try { fs.chmodSync(DEVICES_FILE, 0o600); } catch (_) {}
  } catch (e) { log('speichern fehlgeschlagen: ' + e.message); }
}

/** The account a FreeViewer id belongs to (only relay-verified), or ''. */
function ownerOf(fvid) {
  const e = loadMap()[String(fvid || '')];
  return (e && e.uid) || '';
}

/** Every verified FreeViewer id of an account. */
function devicesOf(uid) {
  const m = loadMap();
  return Object.keys(m).filter((id) => m[id] && m[id].uid === uid);
}

function bind(fvid, uid) {
  const m = loadMap();
  if (m[fvid] && m[fvid].uid === uid) return false;
  if (m[fvid]) log('Geraet', fvid, 'wechselt das Konto', m[fvid].uid, '->', uid);
  m[fvid] = { uid, at: Date.now() };
  saveMap();
  return true;
}

// ------------------------------------------------------- the address book

function call(method, urlPath, token, body) {
  return new Promise((resolve) => {
    const u = new URL(CONTACTS_URL + urlPath);
    const mod = u.protocol === 'https:' ? https : http;
    const data = body ? Buffer.from(JSON.stringify(body)) : null;
    const headers = { accept: 'application/json', authorization: 'Bearer ' + token };
    if (data) { headers['content-type'] = 'application/json'; headers['content-length'] = data.length; }
    const req = mod.request({
      method, hostname: u.hostname, port: u.port || (u.protocol === 'https:' ? 443 : 80),
      path: u.pathname + u.search, headers, timeout: TIMEOUT_MS,
    }, (r) => {
      const teile = [];
      r.on('data', (c) => teile.push(c));
      r.on('end', () => {
        let j = null;
        try { j = JSON.parse(Buffer.concat(teile).toString('utf8')); } catch (_) {}
        resolve({ code: r.statusCode || 0, j: j || {} });
      });
      r.on('error', () => resolve({ code: 0, j: {} }));
    });
    req.on('timeout', () => req.destroy(new Error('timeout')));
    req.on('error', () => resolve({ code: 0, j: {} }));
    if (data) req.write(data);
    req.end();
  });
}

class Unreachable extends Error {}

function checkReply(r) {
  if (r.code === 0 || r.code >= 500) throw new Unreachable('Adressbuch gerade nicht erreichbar');
  return r;
}

// token -> { uid, handle, at, fv: Set }
const profile = new Map();

/**
 * Who is this in the address book? Binds the calling device to the
 * account (verified: secret + token) and puts it into the profile.
 * null when the book does not accept the token.
 */
async function meInBook(ich, token, geraetName) {
  const hit = profile.get(token);
  if (hit && Date.now() - hit.at < PROFIL_MS && hit.fv.has(ich.fvid)) {
    return hit;
  }
  const r = checkReply(await call('GET', '/api/kontakte/profil', token));
  if (r.code === 401) { profile.delete(token); return null; }
  if (r.code !== 200 || !r.j.profil || !r.j.profil.konto) throw new Unreachable('Adressbuch antwortet unerwartet');
  const p = r.j.profil;
  const e = { uid: p.konto, handle: p.handle || '', at: Date.now(), fv: new Set() };
  for (const g of p.geraete || []) if (g.art === 'fv') e.fv.add(String(g.wert));
  bind(ich.fvid, e.uid);
  if (!e.fv.has(ich.fvid)) {
    const label = String(geraetName || '').slice(0, 48) || 'FreeViewer-PC';
    const w = await call('POST', '/api/kontakte/geraet', token, { art: 'fv', wert: ich.fvid, label });
    if (w.code === 200) { e.fv.add(ich.fvid); log('Geraet', ich.fvid, 'ins Profil von', e.uid); }
    else log('Geraet', ich.fvid, 'nicht ins Profil:', w.code, w.j.error || '');
    e.fv.add(ich.fvid); // do not retry on every poll; the binding above is what counts
  }
  if (profile.size > 5000) profile.clear();
  profile.set(token, e);
  return e;
}

// ------------------------------------------------------------ the answer

/**
 * "497 628 420" | "@anna" | "k:<uid>" -> what the book understands.
 * { konto } | { handle } | { fv } (a bare id without a verified account)
 */
function targetFrom(s) {
  const raw = String(s == null ? '' : s).trim();
  if (raw.startsWith('@')) {
    const h = raw.slice(1).trim().toLowerCase();
    return h ? { handle: h } : null;
  }
  if (raw.startsWith('k:')) {
    const k = raw.slice(2).trim();
    return /^[A-Za-z0-9_-]{1,96}$/.test(k) ? { konto: k } : null;
  }
  const id = raw.replace(/\D/g, '');
  if (!id) return null;
  const uid = ownerOf(id);
  return uid ? { konto: uid, fvid: id } : { fv: id };
}

function targetBody(z) {
  return z.konto ? { konto: z.konto } : { handle: z.handle };
}

/** The devices of a friend: relay-verified ids of his account, plus the
 *  ids I noted myself in my row about him. */
function deviceRow(it, state) {
  const ids = new Map(); // fvid -> label
  for (const g of it.geraete || []) {
    if (g.art !== 'fv') continue;
    const id = String(g.wert || '');
    if (!id) continue;
    if (g.quelle === 'profil' && ownerOf(id) !== it.konto) continue;
    if (!ids.has(id)) ids.set(id, String(g.label || ''));
  }
  for (const id of devicesOf(it.konto)) if (!ids.has(id)) ids.set(id, '');
  const out = [];
  for (const [fvid, label] of ids) {
    const z = state(fvid);
    out.push({ fvid, label: label || z.name || '', online: z.online, gesehen: z.gesehen });
  }
  out.sort((a, b) => (b.online - a.online) || (b.gesehen - a.gesehen));
  return out;
}

function person(it) {
  const pr = it.profil || {};
  return { person: it.konto, handle: pr.handle || '', name: it.anzeige || pr.name || '' };
}

/** The address book as FreeViewer's /stand. */
async function bookState(token, state) {
  const r = checkReply(await call('GET', '/api/kontakte/liste', token));
  if (r.code !== 200) throw new Unreachable(r.j.error || 'Adressbuch antwortet unerwartet');
  const s = { freunde: [], eingehend: [], ausgehend: [], blockiert: [] };
  for (const it of r.j.items || []) {
    if (!it.konto || it.geloescht) continue; // rows without an account are contacts, not friends
    const p = person(it);
    if (it.status === 'freunde') {
      const g = deviceRow(it, state);
      const first = g[0] || { fvid: '', online: false, gesehen: 0 };
      s.freunde.push({
        fvid: first.fvid, name: p.name, seit: Number(it.seit) || 0,
        online: g.some((x) => x.online), gesehen: Math.max(0, ...g.map((x) => x.gesehen || 0)),
        person: p.person, handle: p.handle, geraete: g, quelle: 'buch',
      });
    } else if (it.status === 'eingehend' || it.status === 'angefragt') {
      const incoming = it.status === 'eingehend';
      (incoming ? s.eingehend : s.ausgehend).push({
        fvid: '', name: p.name, wann: Number(it.at) || 0,
        richtung: incoming ? 'eingehend' : 'ausgehend', nachricht: '', online: false,
        person: p.person, handle: p.handle, quelle: 'buch',
      });
    } else if (it.status === 'blockiert') {
      s.blockiert.push({ fvid: '', seit: Number(it.at) || 0, person: p.person, handle: p.handle, name: p.name, quelle: 'buch' });
    }
  }
  return s;
}

/** Old entries (freunde.json) whose other side has NO verified account. */
function legacyShare(alt) {
  const unowned = (x) => x && x.fvid && !ownerOf(x.fvid);
  const mark = (x) => Object.assign({}, x, { quelle: 'fv' });
  return {
    freunde: (alt.freunde || []).filter(unowned).map(mark),
    eingehend: (alt.eingehend || []).filter(unowned).map(mark),
    ausgehend: (alt.ausgehend || []).filter(unowned).map(mark),
    blockiert: (alt.blockiert || []).filter(unowned).map(mark),
  };
}

function aus(res, code, obj, alt) { alt.reply(res, code, obj); }

function errorFrom(res, r, alt) {
  const code = r.code === 401 ? 401 : (r.code || 502);
  aus(res, code, { error: r.j.error || 'Adressbuch meldet Fehler ' + r.code }, alt);
}

/**
 * The /fv/freunde/* request of a device that came with a token.
 * `alt` is the old store bound to this caller (relay.js):
 *   alt.stand() -> the old /stand object,  alt.weiter() -> old handler,
 *   alt.antwort(res, code, obj), alt.zustand(fvid) -> {online, gesehen, name}
 * Returns false when the book does not accept the token (the caller then
 * goes on the old way, as a device without an account).
 */
async function handle(p, b, ich, res, alt) {
  const token = String(b.token || '');
  let me;
  try {
    me = await meInBook(ich, token, alt.state(ich.fvid).name || b.name);
  } catch (e) {
    aus(res, 503, { error: e.message }, alt);
    return true;
  }
  if (!me) return false;
  try {
    return await step(p, b, ich, me, token, res, alt);
  } catch (e) {
    if (e instanceof Unreachable) { aus(res, 503, { error: e.message }, alt); return true; }
    throw e;
  }
}

async function step(p, b, ich, me, token, res, alt) {
  const envelope = { ok: true, fvid: ich.fvid, konto: ich.account || me.handle, handle: me.handle, person: me.uid, buch: true };
  if (p === '/fv/freunde/stand' || p === '/fv/freunde/liste' || p === '/fv/freunde/offen') {
    const s = await bookState(token, alt.state);
    const a = legacyShare(alt.snapshot());
    const full = {
      freunde: s.freunde.concat(a.freunde),
      eingehend: s.eingehend.concat(a.eingehend),
      ausgehend: s.ausgehend.concat(a.ausgehend),
      blockiert: s.blockiert.concat(a.blockiert),
    };
    if (p === '/fv/freunde/offen') {
      aus(res, 200, Object.assign(envelope, { eingehend: full.eingehend, ausgehend: full.ausgehend }), alt);
    } else {
      aus(res, 200, Object.assign(envelope, full), alt);
    }
    return true;
  }

  let who = '';
  if (p === '/fv/freunde/anfragen') who = b.an || b.fvid;
  else if (p === '/fv/freunde/annehmen') who = b.von || b.fvid;
  else if (p === '/fv/freunde/ablehnen') who = b.von || b.fvid || b.an;
  else if (p === '/fv/freunde/entfernen') who = b.fvid || b.freund;
  else if (p === '/fv/freunde/blockieren') who = b.fvid;
  else { aus(res, 404, { error: 'unbekannt' }, alt); return true; }

  const z = targetFrom(who);
  if (!z) { aus(res, 400, { error: 'FreeViewer-ID oder @Name fehlt' }, alt); return true; }
  if (z.fv) { alt.next(); return true; } // a bare id: old store
  if (z.konto && z.konto === me.uid) {
    aus(res, 400, { error: z.fvid ? 'Das ist ein eigenes Geraet' : 'Das bist du selbst' }, alt);
    return true;
  }
  const payload = targetBody(z);
  const ref = { fvid: z.fvid || '', person: z.konto || '', handle: z.handle || '' };

  if (p === '/fv/freunde/anfragen') {
    const r = await call('POST', '/api/kontakte/anfragen', token, payload);
    checkReply(r);
    if (r.code !== 200) { errorFrom(res, r, alt); return true; }
    ref.person = r.j.konto || ref.person;
    if (r.j.status === 'freunde') {
      aus(res, 200, Object.assign(envelope, { angenommen: true, freund: Object.assign({ name: '', seit: Math.floor(Date.now() / 1000), online: false, gesehen: 0 }, ref) }), alt);
    } else {
      aus(res, 200, Object.assign(envelope, { anfrage: Object.assign({ name: '', wann: Math.floor(Date.now() / 1000), richtung: 'ausgehend', nachricht: '', online: false }, ref) }), alt);
    }
    return true;
  }
  if (p === '/fv/freunde/annehmen') {
    const r = await call('POST', '/api/kontakte/annehmen', token, payload);
    checkReply(r);
    if (r.code !== 200) {
      if (r.code === 404) { aus(res, 404, { error: 'Keine offene Anfrage von dieser Person' }, alt); return true; }
      errorFrom(res, r, alt);
      return true;
    }
    ref.person = r.j.konto || ref.person;
    aus(res, 200, Object.assign(envelope, { freund: Object.assign({ name: '', seit: Math.floor(Date.now() / 1000), online: false, gesehen: 0 }, ref) }), alt);
    return true;
  }
  if (p === '/fv/freunde/ablehnen') {
    // eingehend: decline (silent); ausgehend: take my own request back
    const r = await call('POST', '/api/kontakte/ablehnen', token, payload);
    checkReply(r);
    if (r.code !== 200) { errorFrom(res, r, alt); return true; }
    let weg = 1;
    if (r.j.status === 'angefragt') {
      const r2 = checkReply(await call('POST', '/api/kontakte/entfernen', token, payload));
      if (r2.code !== 200) { errorFrom(res, r2, alt); return true; }
    } else if (r.j.status === 'freunde' || r.j.status === 'blockiert') {
      weg = 0;
    }
    aus(res, 200, Object.assign(envelope, { weg }), alt);
    return true;
  }
  if (p === '/fv/freunde/entfernen') {
    const r = await call('POST', '/api/kontakte/entfernen', token, payload);
    checkReply(r);
    if (r.code !== 200) { errorFrom(res, r, alt); return true; }
    aus(res, 200, envelope, alt);
    return true;
  }
  // blockieren
  const an = b.an === undefined ? true : !!b.an;
  const r = await call('POST', '/api/kontakte/blockieren', token, Object.assign({ aus: !an }, payload));
  checkReply(r);
  if (r.code !== 200) { errorFrom(res, r, alt); return true; }
  aus(res, 200, Object.assign(envelope, { blockiert: an }), alt);
  return true;
}

module.exports = { handle, ownerOf, devicesOf, targetFrom, bind, _profile: profile, _reset: () => { map = null; profile.clear(); } };
