// FreeViewer setup links - one-time codes that install and enrol a new machine.
//
// The owner (signed in with a FleiTec/JARVIS account) creates a code and gets
// a link. A fresh machine opens the link, picks its display name and redeems
// the code exactly once: it receives the password the owner chose. The owner
// finds the new machine in his account address book right away (id + name)
// and picks up the password once via the inbox - so his client can save the
// machine WITH its password and connect straight away.
//
// The password lives here only until the owner fetched it (or 30 days pass).
'use strict';

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const FILE = process.env.FV_SETUPS || '/root/fvrelay/setups.json';
const TTL_MS = 7 * 24 * 3600 * 1000;    // unclaimed codes live a week
const INBOX_MS = 30 * 24 * 3600 * 1000; // claimed results wait 30 days for the owner
const MAX_BODY = 64 * 1024;

let account = null; // injected: whoIs(token, cb) + addDevice(user, entry)

function log(...a) { console.log(new Date().toISOString(), '[setup]', ...a); }

function jsonOut(res, code, obj) {
  const s = JSON.stringify(obj);
  res.writeHead(code, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(s) });
  res.end(s);
}

function readBody(req, cb) {
  let n = 0;
  const parts = [];
  req.on('data', (c) => {
    n += c.length;
    if (n > MAX_BODY) { req.destroy(); cb(new Error('zu gross')); return; }
    parts.push(c);
  });
  req.on('end', () => {
    try { cb(null, JSON.parse(Buffer.concat(parts).toString('utf8') || '{}')); }
    catch (e) { cb(e); }
  });
  req.on('error', () => cb(new Error('Lesefehler')));
}

function load() {
  try { return JSON.parse(fs.readFileSync(FILE, 'utf8')); }
  catch (_) { return { setups: [] }; }
}

function save(db) {
  fs.mkdirSync(path.dirname(FILE), { recursive: true });
  fs.writeFileSync(FILE, JSON.stringify(db, null, 1));
  try { fs.chmodSync(FILE, 0o600); } catch (_) {}
}

/** Drop expired codes and results the owner had 30 days to fetch. */
function sweep(db) {
  const now = Date.now();
  db.setups = (db.setups || []).filter((s) => {
    const claims = s.claims || (s.claimed ? [s.claimed] : []);
    const max = s.max_uses === 0 ? Infinity : (s.max_uses || 1);
    if (claims.length >= max) {
      const last = claims.reduce((m, cl) => Math.max(m, cl.at || 0), 0);
      return now - last < INBOX_MS;
    }
    if (s.max_uses === 0) {
      // unbegrenzte Links bleiben, solange sie benutzt werden
      const last = claims.reduce((m, cl) => Math.max(m, cl.at || 0), s.at);
      return now - last < INBOX_MS;
    }
    return now - s.at < TTL_MS;
  });
}

function newCode() { return crypto.randomBytes(6).toString('base64url'); } // 8 URL-safe chars

function str(v, n) { return String(v == null ? '' : v).slice(0, n); }

/** Returns true when the request was handled here. */
function handle(req, res) {
  const url = new URL(req.url, 'http://x');
  const p = url.pathname;
  if (!p.startsWith('/fv/setup')) return false;

  // --- owner creates a one-time code -------------------------------------
  if (req.method === 'POST' && p === '/fv/setup/create') {
    const token = url.searchParams.get('token') || '';
    account.whoIs(token, (err, user) => {
      if (err) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
      if (!user) return jsonOut(res, 401, { error: 'nicht angemeldet' });
      readBody(req, (e, b) => {
        if (e) return jsonOut(res, 400, { error: 'ungueltige Anfrage' });
        const password = str(b.password, 64).trim();
        if (password.length < 4) return jsonOut(res, 400, { error: 'Passwort zu kurz (min. 4 Zeichen)' });
        const name_hint = str(b.name_hint, 40).trim();
        // wie viele Geraete sich mit diesem Link einrichten duerfen (1-25)
        // 0 = unbegrenzt, sonst 1-25
        let max_uses = parseInt(b.max_uses, 10);
        if (!Number.isFinite(max_uses) || max_uses < 0) max_uses = 1;
        if (max_uses > 25) max_uses = 25;
        const db = load(); sweep(db);
        let code;
        do { code = newCode(); } while (db.setups.some((s) => s.code === code));
        // Marke des erzeugenden Builds - der Download liefert spaeter die
        // passende Datei dazu
        const brand = ['freeviewer', 'xoffi'].includes(str(b.brand, 20)) ? str(b.brand, 20) : 'freeviewer';
        db.setups.push({ code, owner: user, password, name_hint, at: Date.now(), max_uses, claims: [], brand });
        save(db);
        log('code created by', user);
        jsonOut(res, 200, {
          ok: true,
          code,
          link: 'freeviewer://setup/' + code,
          web: 'https://freeviewer.fleitec.com/setup/' + code,
        });
      });
    });
    return true;
  }

  // --- the fresh machine redeems the code, exactly once -------------------
  if (req.method === 'POST' && p === '/fv/setup/claim') {
    readBody(req, (e, b) => {
      if (e) return jsonOut(res, 400, { error: 'ungueltige Anfrage' });
      const code = str(b.code, 24).trim();
      const id = str(b.id, 12).replace(/\D/g, '');
      const name = str(b.name, 40).trim();
      if (!code || id.length < 6 || !name) return jsonOut(res, 400, { error: 'code, id und name noetig' });
      const db = load(); sweep(db);
      const s = db.setups.find((x) => x.code === code);
      if (!s) return jsonOut(res, 404, { error: 'Code unbekannt oder abgelaufen' });
      const claims = s.claims || (s.claimed ? [s.claimed] : []);
      const max = s.max_uses === 0 ? Infinity : (s.max_uses || 1);
      if (claims.some((cl) => cl.id === id)) return jsonOut(res, 409, { error: 'Dieses Geraet ist mit dem Code schon eingerichtet' });
      if (claims.length >= max) return jsonOut(res, 410, { error: 'Code wurde schon verwendet' });
      const claim = { id, name, at: Date.now(), acked: false };
      s.claims = claims.concat([claim]);
      delete s.claimed;
      save(db);
      // show up in the owner's account address book right away (no password there)
      if (account.addDevice) {
        try {
          account.addDevice(s.owner, { id, name, group: 'Neu eingerichtet', at: Math.floor(Date.now() / 1000) });
        } catch (e2) { log('addDevice failed:', e2.message); }
      }
      log('claimed', code, '->', id, '"' + name + '"', 'owner', s.owner);
      jsonOut(res, 200, { ok: true, password: s.password, name_hint: s.name_hint, owner: s.owner });
    });
    return true;
  }

  // --- owner revokes a pending link (original stops working) --------------
  if (req.method === 'POST' && p === '/fv/setup/revoke') {
    const token = url.searchParams.get('token') || '';
    account.whoIs(token, (err, user) => {
      if (err) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
      if (!user) return jsonOut(res, 401, { error: 'nicht angemeldet' });
      readBody(req, (e, b) => {
        if (e) return jsonOut(res, 400, { error: 'ungueltige Anfrage' });
        const code = str(b.code, 24).trim();
        const db = load(); sweep(db);
        const s = db.setups.find((x) => x.code === code && x.owner === user);
        if (!s) return jsonOut(res, 404, { error: 'Code unbekannt' });
        const claims = s.claims || (s.claimed ? [s.claimed] : []);
        // schon eingeloese Geraete behalten ihre Einrichtung - nur der Rest
        // des Links wird unbrauchbar
        if (claims.length) {
          s.max_uses = claims.length;
        } else {
          db.setups = db.setups.filter((x) => x !== s);
        }
        save(db);
        log('revoked', code, 'by', user, '(' + claims.length + ' schon eingerichtet)');
        jsonOut(res, 200, { ok: true });
      });
    });
    return true;
  }

  // --- owner fetches what got set up (password included, exactly once) ----
  if (req.method === 'GET' && p === '/fv/setup/inbox') {
    const token = url.searchParams.get('token') || '';
    account.whoIs(token, (err, user) => {
      if (err) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
      if (!user) return jsonOut(res, 401, { error: 'nicht angemeldet' });
      const db = load(); sweep(db);
      const mine = db.setups.filter((s) => s.owner === user);
      const claimsOf = (s) => s.claims || (s.claimed ? [s.claimed] : []);
      const pending = mine
        .filter((s) => s.max_uses === 0 || claimsOf(s).length < (s.max_uses || 1))
        .map((s) => ({
          code: s.code,
          name_hint: s.name_hint,
          at: Math.floor(s.at / 1000),
          max_uses: s.max_uses === 0 ? 0 : (s.max_uses || 1),
          uses: claimsOf(s).length,
        }));
      const fresh = [];
      for (const s of mine) {
        for (const cl of claimsOf(s)) {
          if (!cl.acked) fresh.push({ s, cl });
        }
      }
      const claimed = fresh.map(({ s, cl }) => ({
        id: cl.id,
        name: cl.name,
        password: s.password,
        at: Math.floor(cl.at / 1000),
      }));
      if (fresh.length) {
        for (const { cl } of fresh) cl.acked = true;
        save(db);
      }
      jsonOut(res, 200, { ok: true, pending, claimed });
    });
    return true;
  }

  jsonOut(res, 404, { error: 'unbekannt' });
  return true;
}

/** Welche Marke gehoert zu diesem Code? (Download-Datei danach waehlen) */
function brand_of(code) {
  const db = load();
  const s = (db.setups || []).find((x) => x.code === code);
  return (s && s.brand) || 'freeviewer';
}

/** Oeffentliche Kurzinfo fuer die Einrichtungs-Seite (kein Passwort!). */
function info(code) {
  const db = load(); sweep(db);
  const s = (db.setups || []).find((x) => x.code === code);
  if (!s) return null;
  return { brand: (s && s.brand) || 'freeviewer', name_hint: s.name_hint || '' };
}

/** Darf dieser Code noch eingeloest werden? (fuer den Download mit Auftrag) */
function claimable(code) {
  const db = load(); sweep(db);
  const s = (db.setups || []).find((x) => x.code === code);
  if (!s) return false;
  const claims = s.claims || (s.claimed ? [s.claimed] : []);
  const max = s.max_uses === 0 ? Infinity : (s.max_uses || 1);
  return claims.length < max;
}

module.exports = {
  handle,
  claimable,
  brand_of,
  info,
  init(acc) { account = acc; },
};
