'use strict';
// Request authentication helpers (roadmap r6 + r7, relay patch 0004).
//
// r7  The JARVIS login token must not travel in the URL (proxy/access logs, browser
//     history, Referer). Clients now send it as `Authorization: Bearer <token>`
//     (or `X-Fv-Token`). The old `?token=` form still works as a deprecated
//     fallback so already-installed clients keep syncing:
//       - the response carries `Deprecation: true` + a `Warning` header,
//       - a hit is counted (health) and logged once per client IP and 10 minutes,
//         WITHOUT the token and without the query string,
//       - FV_ACCOUNT_QUERY_TOKEN=0 refuses the fallback once old clients are gone.
//     account.js / setups.js are not touched: `prepare()` moves a header token into
//     the in-memory `req.url` they already read (never written to any log).
// r6  GET /fv/online used to tell anybody the device name and last-online time of
//     any ID. Now: per-IP rate limit, and `name`/`seen` only for a signed-in caller
//     and only for IDs in that caller's own address book. Everybody else gets
//     `{online: bool}` and nothing else.
//       FV_ONLINE_RATE=<n>            requests per minute and client IP (default 120)
//       FV_ONLINE_PUBLIC_DETAILS=1    old behaviour (name/seen for everybody)

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const TOKEN_RE = /^[\w.\-]{10,4096}$/;
const ACCOUNT_QUERY_TOKEN = process.env.FV_ACCOUNT_QUERY_TOKEN !== '0';
const ONLINE_RATE = Math.max(1, parseInt(process.env.FV_ONLINE_RATE || '120', 10) || 120);
const ONLINE_PUBLIC_DETAILS = process.env.FV_ONLINE_PUBLIC_DETAILS === '1';
const ACC_DIR = process.env.FV_ACC || '/root/fvrelay/accounts';
const MAX_IDS = 200;

const stats = { queryTokenHits: 0, onlineRateLimited: 0 };

// ---------------------------------------------------------------- r7

/** Token from the request headers, '' when none/invalid. Never looks at the URL. */
function headerToken(req) {
  const auth = String(req.headers['authorization'] || '');
  let t = '';
  if (/^bearer /i.test(auth)) t = auth.slice(7).trim();
  else if (req.headers['x-fv-token']) t = String(req.headers['x-fv-token']).trim();
  return TOKEN_RE.test(t) ? t : '';
}

const warned = new Map(); // ip -> last log time (ms)
function warnOnce(ip, pathname, log) {
  const now = Date.now();
  const last = warned.get(ip) || 0;
  if (now - last < 10 * 60 * 1000) return;
  if (warned.size > 2000) warned.clear();
  warned.set(ip, now);
  log('DEPRECATED: login token in the URL on ' + pathname + ' from ' + ip + ' - client must send Authorization: Bearer');
}

/**
 * Call before account.js / setups.js. Returns true when the request was answered here.
 * A header token wins over a query token and is handed on through req.url, so the
 * existing handlers keep working unchanged.
 */
function prepare(req, res, ip, log) {
  let u;
  try { u = new URL(req.url, 'http://x'); } catch (_) { return false; }
  const viaHeader = headerToken(req);
  if (viaHeader) {
    u.searchParams.set('token', viaHeader);
    req.url = u.pathname + u.search;
    return false;
  }
  if (!u.searchParams.has('token')) return false;
  // legacy: token in the URL
  stats.queryTokenHits++;
  if (!ACCOUNT_QUERY_TOKEN) {
    res.writeHead(401, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    res.end(JSON.stringify({ error: 'nicht angemeldet', hint: 'send the token as Authorization: Bearer' }));
    return true;
  }
  res.setHeader('Deprecation', 'true');
  res.setHeader('Warning', '299 - "login token in the URL is deprecated, use Authorization: Bearer"');
  warnOnce(ip, u.pathname, log);
  return false;
}

// ---------------------------------------------------------------- r6

const hits = new Map(); // ip -> { start, count }
/** Fixed one-minute window per client IP. */
function rateLimit(ip, now) {
  now = now || Date.now();
  if (hits.size > 5000) {
    for (const [k, v] of hits) if (now - v.start >= 60000) hits.delete(k);
    if (hits.size > 5000) hits.clear();
  }
  let h = hits.get(ip);
  if (!h || now - h.start >= 60000) { h = { start: now, count: 0 }; hits.set(ip, h); }
  h.count++;
  if (h.count > ONLINE_RATE) {
    stats.onlineRateLimited++;
    return { ok: false, retryAfter: Math.max(1, Math.ceil((h.start + 60000 - now) / 1000)) };
  }
  return { ok: true, retryAfter: 0 };
}

// Same file name as account.js `fileFor` (test_reqauth.js guards drift).
function accountFile(user) {
  const safe = crypto.createHash('sha256').update(String(user).toLowerCase()).digest('hex').slice(0, 32);
  return path.join(process.env.FV_ACC || ACC_DIR, safe + '.json');
}
const ownedCache = new Map(); // user -> { at, ids:Set }
/** IDs in the signed-in user's own address book (tombstones excluded). */
function ownedIds(user, now) {
  now = now || Date.now();
  const hit = ownedCache.get(user);
  if (hit && now - hit.at < 15000) return hit.ids;
  let ids = new Set();
  try {
    const rec = JSON.parse(fs.readFileSync(accountFile(user), 'utf8'));
    for (const d of rec.devices || []) if (d && !d.deleted && d.id) ids.add(String(d.id).replace(/\D/g, ''));
  } catch (_) { ids = new Set(); }
  if (ownedCache.size > 1000) ownedCache.clear();
  ownedCache.set(user, { at: now, ids });
  return ids;
}

function parseIds(raw) {
  const out = [];
  const seen = new Set();
  for (const part of String(raw || '').split(',').slice(0, MAX_IDS)) {
    const id = part.replace(/\D/g, '');
    if (!id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

/**
 * Answer for GET /fv/online. `hosts` = Map of connected hosts, `meta` = id -> {name, seen},
 * `whoIs(token, cb)` from account.js (or null). `cb(code, body, headers)`.
 */
function onlineReply(req, ip, hosts, meta, whoIs, cb) {
  const rl = rateLimit(ip);
  if (!rl.ok) return cb(429, { error: 'zu viele Anfragen' }, { 'retry-after': String(rl.retryAfter) });
  const ids = parseIds(new URL(req.url, 'http://x').searchParams.get('ids'));
  const build = (owned) => {
    const out = {};
    for (const id of ids) {
      const e = { online: hosts.has(id) };
      if (ONLINE_PUBLIC_DETAILS || (owned && owned.has(id))) {
        const m = meta[id] || {};
        e.name = m.name || '';
        e.seen = m.seen || 0;
      }
      out[id] = e;
    }
    return { ids: out };
  };
  const token = headerToken(req);
  if (!token || !whoIs) return cb(200, build(null), {});
  whoIs(token, (err, user) => {
    // JARVIS down or token unknown: still answer, just without the details
    if (err || !user) return cb(200, build(null), {});
    cb(200, build(ownedIds(user)), {});
  });
}

module.exports = { headerToken, prepare, rateLimit, ownedIds, accountFile, parseIds, onlineReply, stats, _hits: hits };
