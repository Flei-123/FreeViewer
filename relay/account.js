// FreeViewer account sync - optional FleiTec/JARVIS account for the address book.
//
// FreeViewer works fine without any account. If somebody DOES sign in, the
// relay keeps one small file per account: the devices in the address book
// (id, name, folder, note, favourite, statistics). Deliberately NOT synced:
// saved passwords. Those are encrypted with the machine identity and would be
// worthless on another PC - and the relay must stay a zero knowledge box.
//
// Authentication is not our own: the client signs in at JARVIS
// (POST /api/login) and gets a token; we only ever ASK jarvis who a token
// belongs to (GET /api/me?token=..) and use the username as the key.
'use strict';

const fs = require('fs');
const path = require('path');
const http = require('http');
const https = require('https');

const DIR = process.env.FV_ACC || '/root/fvrelay/accounts';
const JARVIS = (process.env.FV_JARVIS || 'http://192.168.1.54:8000').replace(/\/$/, '');
const MAX_BODY = 512 * 1024;
const MAX_ENTRIES = 500;

// token -> { user, at } so a sync every few minutes does not hammer JARVIS
const cache = new Map();
const CACHE_MS = 5 * 60 * 1000;

function log(...a) { console.log(new Date().toISOString(), '[account]', ...a); }

function jsonOut(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { 'content-type': 'application/json', 'cache-control': 'no-store' });
  res.end(body);
}

function readBody(req, cb) {
  let size = 0;
  const chunks = [];
  req.on('data', (c) => {
    size += c.length;
    if (size > MAX_BODY) { req.destroy(); return; }
    chunks.push(c);
  });
  req.on('end', () => {
    try { cb(null, JSON.parse(Buffer.concat(chunks).toString('utf8') || '{}')); }
    catch (e) { cb(e); }
  });
  req.on('error', (e) => cb(e));
}

// ---------------------------------------------------------------- JARVIS

function call(method, url, body, cb, extraHeaders) {
  const u = new URL(url);
  const mod = u.protocol === 'https:' ? https : http;
  const data = body ? Buffer.from(JSON.stringify(body)) : null;
  const req = mod.request({
    method,
    hostname: u.hostname,
    port: u.port || (u.protocol === 'https:' ? 443 : 80),
    path: u.pathname + u.search,
    headers: Object.assign(
      { accept: 'application/json' },
      data ? { 'content-type': 'application/json', 'content-length': data.length } : {},
      extraHeaders || {}
    ),
    timeout: 8000,
  }, (r) => {
    const chunks = [];
    r.on('data', (c) => chunks.push(c));
    r.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      let obj = null;
      try { obj = JSON.parse(text); } catch (_) {}
      cb(null, r.statusCode || 0, obj, text);
    });
  });
  req.on('timeout', () => { req.destroy(new Error('Zeitueberschreitung')); });
  req.on('error', (e) => cb(e));
  if (data) req.write(data);
  req.end();
}

/** Who does this token belong to? `cb(null, username)` or `cb(null, null)`. */
function whoIs(token, cb) {
  if (!token || !/^[\w.\-]{10,4096}$/.test(token)) return cb(null, null);
  const hit = cache.get(token);
  if (hit && Date.now() - hit.at < CACHE_MS) return cb(null, hit.user);
  // The token travels as the session cookie, not in the address: JARVIS is
  // reached over https://jarvis.fleitec.com (the LAN port is firewalled), and
  // an address ends up in the proxy's access log.
  call('GET', JARVIS + '/api/me', null, (err, code, obj) => {
    if (err) return cb(err);
    if (code !== 200 || !obj || !obj.username) { cache.delete(token); return cb(null, null); }
    cache.set(token, { user: String(obj.username), at: Date.now() });
    cb(null, String(obj.username));
  }, { cookie: 'fleitec_session=' + token });
}

// ----------------------------------------------------------------- store

function fileFor(user) {
  const safe = require('crypto').createHash('sha256').update(user.toLowerCase()).digest('hex').slice(0, 32);
  return path.join(DIR, safe + '.json');
}

function load(user) {
  try { return JSON.parse(fs.readFileSync(fileFor(user), 'utf8')); }
  catch (_) { return { rev: 0, at: 0, devices: [] }; }
}

function save(user, rec) {
  fs.mkdirSync(DIR, { recursive: true });
  fs.writeFileSync(fileFor(user), JSON.stringify(rec));
}

function str(v, n) { return String(v == null ? '' : v).slice(0, n); }
function num(v) { const x = Number(v); return Number.isFinite(x) && x >= 0 ? Math.floor(x) : 0; }

/** One address book entry, trimmed to what we are willing to store. */
function clean(e) {
  const id = String(e && e.id || '').replace(/\D/g, '');
  if (!id) return null;
  return {
    id,
    name: str(e.name, 40),
    group: str(e.group, 40),
    note: str(e.note, 300),
    favorite: !!e.favorite,
    last: num(e.last),
    count: num(e.count),
    seconds: num(e.seconds),
    at: num(e.at) || num(e.last) || Math.floor(Date.now() / 1000),
    deleted: !!e.deleted,
  };
}

/**
 * Union by id, newest `at` wins. Deletions travel as tombstones so a device
 * removed on one PC does not come back from the next sync of another.
 */
function merge(mine, theirs) {
  const map = new Map();
  for (const e of mine || []) { const c = clean(e); if (c) map.set(c.id, c); }
  for (const e of theirs || []) {
    const c = clean(e);
    if (!c) continue;
    const old = map.get(c.id);
    if (!old || c.at >= old.at) map.set(c.id, c);
  }
  const out = [...map.values()].sort((a, b) => b.at - a.at).slice(0, MAX_ENTRIES);
  return out;
}

// ---------------------------------------------------------------- routes

/** Returns true when the request was handled here. */
function handle(req, res) {
  const url = new URL(req.url, 'http://x');
  const p = url.pathname;
  if (!p.startsWith('/fv/account')) return false;

  // --- sign in: we forward to JARVIS so the client never needs a second account
  if (req.method === 'POST' && p === '/fv/account/login') {
    readBody(req, (err, b) => {
      if (err) return jsonOut(res, 400, { error: 'ungueltige Anfrage' });
      const username = str(b.username, 64).trim();
      const password = str(b.password, 256);
      if (!username || !password) return jsonOut(res, 400, { error: 'Benutzername und Passwort noetig' });
      call('POST', JARVIS + '/api/login', { username, password }, (e2, code, obj) => {
        if (e2) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
        if (code !== 200 || !obj || !obj.token) {
          return jsonOut(res, code === 429 ? 429 : 401, { error: (obj && obj.error) || 'Anmeldung fehlgeschlagen' });
        }
        log('login', obj.username);
        jsonOut(res, 200, { token: obj.token, username: obj.username });
      });
    });
    return true;
  }

  // --- everything below needs a token
  const token = url.searchParams.get('token') || '';

  if (req.method === 'GET' && p === '/fv/account/me') {
    whoIs(token, (err, user) => {
      if (err) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
      if (!user) return jsonOut(res, 401, { error: 'nicht angemeldet' });
      jsonOut(res, 200, { username: user });
    });
    return true;
  }

  if (p === '/fv/account/data' && (req.method === 'GET' || req.method === 'POST')) {
    whoIs(token, (err, user) => {
      if (err) return jsonOut(res, 502, { error: 'Anmeldedienst nicht erreichbar' });
      if (!user) return jsonOut(res, 401, { error: 'nicht angemeldet' });
      if (req.method === 'GET') {
        const rec = load(user);
        return jsonOut(res, 200, { ok: true, username: user, rev: rec.rev || 0, at: rec.at || 0, devices: rec.devices || [] });
      }
      readBody(req, (e2, b) => {
        if (e2) return jsonOut(res, 400, { error: 'ungueltige Anfrage' });
        const rec = load(user);
        const devices = merge(rec.devices, Array.isArray(b.devices) ? b.devices : []);
        const next = { rev: (rec.rev || 0) + 1, at: Math.floor(Date.now() / 1000), devices };
        try { save(user, next); } catch (e3) {
          log('save failed', e3.message);
          return jsonOut(res, 500, { error: 'konnte nicht speichern' });
        }
        jsonOut(res, 200, { ok: true, username: user, rev: next.rev, at: next.at, devices });
      });
    });
    return true;
  }

  jsonOut(res, 404, { error: 'unbekannt' });
  return true;
}


/** Merge a single device into a user's stored list (used by setup links). */
function addDevice(user, entry) {
  const rec = load(user);
  const devices = merge(rec.devices, [entry]);
  const next = { rev: (rec.rev || 0) + 1, at: Math.floor(Date.now() / 1000), devices };
  save(user, next);
  return next;
}

module.exports = { handle, merge, clean, whoIs, addDevice };
