#!/usr/bin/env node
// FreeViewer Relay v1 - dumb, zero-knowledge pipe between host and viewer.
// The relay only knows: host-id <-> connection. It never sees the session password
// and cannot decrypt traffic (end-to-end AES-256-GCM between host and viewer).
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
let WebSocket;
try {
  WebSocket = require('ws');
} catch (e) {
  WebSocket = require('/root/jarvis/node_modules/ws'); // deployment fallback
}

const PORT = parseInt(process.env.FV_PORT || '7180', 10);
const DATA = process.env.FV_DATA || path.join(__dirname, 'hosts.json');
const MAX_PAYLOAD = 16 * 1024 * 1024;

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
    const n = 100000000 + crypto.randomInt(0, 900000000);
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

const server = http.createServer((req, res) => {
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
    }));
    return;
  }
  // ---- who is online:  GET /fv/online?ids=1,2,3 ----
  if (req.method === 'GET' && req.url.startsWith('/fv/online')) {
    const q = new URL(req.url, 'http://x').searchParams.get('ids') || '';
    const out = {};
    for (const raw of q.split(',').slice(0, 200)) {
      const id = raw.replace(/\D/g, '');
      if (!id) continue;
      const e = meta[id] || {};
      out[id] = { online: hosts.has(id), name: e.name || '', seen: e.seen || 0 };
    }
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ ids: out }));
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
        send(host, { t: 'incoming', sid, from: req.socket.remoteAddress });
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
