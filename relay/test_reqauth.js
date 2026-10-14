#!/usr/bin/env node
// Tests for relay patch 0004 (r6 + r7): login token in a header instead of the URL,
// and /fv/online without leaking names/last-seen. Starts real relay processes with a
// fake JARVIS in front of them.   node test_reqauth.js   (needs `ws` like relay.js)
'use strict';
const http = require('http');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');
let WebSocket;
try { WebSocket = require('ws'); } catch (e) { WebSocket = require('/root/jarvis/node_modules/ws'); }

let failed = 0;
function check(name, ok) { console.log((ok ? 'ok   ' : 'FAIL ') + name); if (!ok) failed++; }

const TOK = 'tok-justin-0123456789';
const TOK_OTHER = 'tok-other-0123456789x';
const SECRET = 'c'.repeat(64);
const SECRET2 = 'd'.repeat(64);

function request(port, opts, body) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, ...opts }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString();
        let json = null; try { json = JSON.parse(text); } catch (_) {}
        resolve({ status: res.statusCode, headers: res.headers, text, json });
      });
    });
    r.on('error', reject);
    if (body) r.write(body);
    r.end();
  });
}

// fake JARVIS: GET /api/me with cookie fleitec_session=<token>
function startJarvis() {
  return new Promise((resolve) => {
    const srv = http.createServer((req, res) => {
      const m = /fleitec_session=([^;]+)/.exec(req.headers.cookie || '');
      const t = m && m[1];
      const user = t === TOK ? 'Justin' : t === TOK_OTHER ? 'Other' : null;
      if (req.url === '/api/me' && user) { res.writeHead(200, { 'content-type': 'application/json' }); return res.end(JSON.stringify({ username: user })); }
      res.writeHead(401); res.end('{}');
    });
    srv.listen(0, '127.0.0.1', () => resolve(srv));
  });
}

async function startRelay(port, jarvisPort, extraEnv) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fvauth-'));
  const env = {
    ...process.env,
    FV_PORT: String(port),
    FV_DATA: path.join(dir, 'hosts.json'),
    FV_DIST: path.join(dir, 'dist'),
    FV_UPLOAD_TOKEN_FILE: path.join(dir, 'tok'),
    FV_ACC: path.join(dir, 'acc'),
    FV_JARVIS: 'http://127.0.0.1:' + jarvisPort,
    FV_BUCH: '0',
    ...extraEnv,
  };
  const child = spawn(process.execPath, [path.join(__dirname, 'relay.js')], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  child.stdout.on('data', (d) => { log += d; });
  child.stderr.on('data', (d) => { log += d; });
  for (let i = 0; i < 50; i++) {
    try { const r = await request(port, { path: '/fv/health' }); if (r.status === 200) return { child, dir, getLog: () => log }; } catch (_) {}
    await new Promise((r) => setTimeout(r, 100));
  }
  child.kill();
  throw new Error('relay did not start');
}
function stop(h) { h.child.kill(); fs.rmSync(h.dir, { recursive: true, force: true }); }

async function registerHost(port, secret, name) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/fv/ws`);
  await new Promise((r) => ws.on('open', r));
  const reg = new Promise((r) => ws.once('message', (d) => r(JSON.parse(d.toString()))));
  ws.send(JSON.stringify({ t: 'host_register', secret, name }));
  const m = await reg;
  return { ws, id: m.id };
}

const bearer = (t) => ({ authorization: 'Bearer ' + t });
const online = (port, ids, headers) => request(port, { path: '/fv/online?ids=' + ids.join(','), headers: headers || {} });

(async () => {
  const jarvis = await startJarvis();
  const jp = jarvis.address().port;

  // ======== default config ========
  let h = await startRelay(17281, jp, { FV_ONLINE_RATE: '1000' });
  try {
    const P = 17281;
    // ---- r7: token in header ----
    let r = await request(P, { path: '/fv/account/me', headers: bearer(TOK) });
    check('r7: Authorization Bearer works', r.status === 200 && r.json.username === 'Justin');
    check('r7: header token -> no Deprecation header', !r.headers['deprecation']);
    r = await request(P, { path: '/fv/account/me', headers: { 'x-fv-token': TOK } });
    check('r7: X-Fv-Token works', r.status === 200 && r.json.username === 'Justin');
    r = await request(P, { path: '/fv/account/me', headers: bearer('tok-bad-0123456789') });
    check('r7: unknown header token -> 401', r.status === 401);
    r = await request(P, { path: '/fv/account/me' });
    check('r7: no token -> 401', r.status === 401);
    r = await request(P, { path: '/fv/account/me?token=' + TOK_OTHER, headers: bearer(TOK) });
    check('r7: header wins over a query token', r.status === 200 && r.json.username === 'Justin');
    r = await request(P, { path: '/fv/account/me', headers: { authorization: 'Bearer x y' } });
    check('r7: malformed header token -> 401', r.status === 401);

    // ---- r7: deprecated query fallback ----
    r = await request(P, { path: '/fv/account/me?token=' + TOK });
    check('r7: ?token= still works (old clients)', r.status === 200 && r.json.username === 'Justin');
    check('r7: ?token= answer carries Deprecation: true', r.headers['deprecation'] === 'true');
    check('r7: ?token= answer carries a Warning header', /deprecated/.test(String(r.headers['warning'] || '')));
    await request(P, { path: '/fv/account/me?token=' + TOK });
    const health = (await request(P, { path: '/fv/health' })).json;
    check('r7: health counts query-token hits', health.account_query_token_hits === 2);
    await new Promise((x) => setTimeout(x, 150));
    const lg = h.getLog();
    check('r7: deprecation logged', /DEPRECATED: login token in the URL on \/fv\/account\/me/.test(lg));
    check('r7: deprecation logged once per IP, not per request', (lg.match(/DEPRECATED/g) || []).length === 1);

    // ---- r7: sync + setups with header ----
    r = await request(P, { path: '/fv/account/data', method: 'POST', headers: { ...bearer(TOK), 'content-type': 'application/json' } },
      JSON.stringify({ devices: [{ id: '123456789', name: 'laptop' }] }));
    check('r7: POST /fv/account/data with header', r.status === 200 && r.json.devices.length === 1);
    r = await request(P, { path: '/fv/setup/inbox', headers: bearer(TOK) });
    check('r7: /fv/setup/inbox with header', r.status === 200);
    r = await request(P, { path: '/fv/setup/inbox' });
    check('r7: /fv/setup/inbox without token -> 401', r.status === 401);

    // ---- r6: /fv/online ----
    const host = await registerHost(P, SECRET, 'Flei-ONE');
    const host2 = await registerHost(P, SECRET2, 'Secret-PC');
    await request(P, { path: '/fv/account/data', method: 'POST', headers: { ...bearer(TOK), 'content-type': 'application/json' } },
      JSON.stringify({ devices: [{ id: host.id, name: 'mine' }] }));
    const ids = [host.id, host2.id, '999999999'];

    r = await online(P, ids);
    check('r6: anonymous gets online flags', r.status === 200 && r.json.ids[host.id].online === true && r.json.ids['999999999'].online === false);
    check('r6: anonymous gets NO name / seen', !('name' in r.json.ids[host.id]) && !('seen' in r.json.ids[host.id]) && !/Flei-ONE/.test(r.text));
    r = await online(P, ids, bearer(TOK));
    check('r6: signed in: name + seen for an ID in the own address book', r.json.ids[host.id].name === 'Flei-ONE' && r.json.ids[host.id].seen > 0);
    check('r6: signed in: still nothing for IDs not in the own book', !('name' in r.json.ids[host2.id]) && !/Secret-PC/.test(r.text));
    r = await online(P, ids, bearer(TOK_OTHER));
    check('r6: another account does not see the first account\'s devices', !('name' in r.json.ids[host.id]));
    r = await online(P, ids, bearer('tok-bad-0123456789'));
    check('r6: invalid token -> minimal answer (no error)', r.status === 200 && !('name' in r.json.ids[host.id]) && r.json.ids[host.id].online === true);
    r = await request(P, { path: '/fv/online?ids=' + host.id + '&token=' + TOK });
    check('r6: token in the URL is NOT accepted for details', !('name' in r.json.ids[host.id]));
    r = await online(P, ['abc', '', host.id, host.id]);
    check('r6: junk ids ignored, duplicates folded', Object.keys(r.json.ids).length === 1);
    r = await request(P, { path: '/fv/online' });
    check('r6: no ids -> empty map', r.status === 200 && Object.keys(r.json.ids).length === 0);
    check('r6: answer is not cacheable', /no-store/.test(r.headers['cache-control'] || ''));

    // file name derivation equals account.js (guards drift of reqauth.accountFile)
    const ra = require('./reqauth');
    process.env.FV_ACC = path.join(h.dir, 'acc');
    check('reqauth.accountFile points at the file account.js wrote', fs.existsSync(ra.accountFile('Justin')));
    host.ws.close(); host2.ws.close();
  } finally { stop(h); }

  // ======== escape hatches ========
  h = await startRelay(17282, jp, { FV_ONLINE_PUBLIC_DETAILS: '1', FV_ACCOUNT_QUERY_TOKEN: '0', FV_ONLINE_RATE: '1000' });
  try {
    const P = 17282;
    const host = await registerHost(P, SECRET, 'Flei-ONE');
    let r = await online(P, [host.id]);
    check('FV_ONLINE_PUBLIC_DETAILS=1: old behaviour (names for everybody)', r.json.ids[host.id].name === 'Flei-ONE');
    r = await request(P, { path: '/fv/account/me?token=' + TOK });
    check('FV_ACCOUNT_QUERY_TOKEN=0: ?token= refused', r.status === 401 && /Authorization/.test(r.text));
    r = await request(P, { path: '/fv/account/me', headers: bearer(TOK) });
    check('FV_ACCOUNT_QUERY_TOKEN=0: header still works', r.status === 200);
    host.ws.close();
  } finally { stop(h); }

  // ======== rate limit ========
  h = await startRelay(17283, jp, { FV_ONLINE_RATE: '5' });
  try {
    const P = 17283;
    const codes = [];
    let last = null;
    for (let i = 0; i < 8; i++) { last = await online(P, ['111111111']); codes.push(last.status); }
    check('r6: first 5 requests pass', codes.slice(0, 5).every((c) => c === 200));
    check('r6: 6th request -> 429', codes[5] === 429 && codes[7] === 429);
    check('r6: 429 carries Retry-After', Number(last.headers['retry-after']) >= 1);
    const hh = (await request(P, { path: '/fv/health' })).json;
    check('r6: health counts rate-limited requests', hh.online_rate_limited === 3);
    // another client address (behind the proxy) has its own budget
    let r = await request(P, { path: '/fv/online?ids=1', headers: { 'x-forwarded-for': '203.0.113.5' } });
    check('r6: budget is per client IP (X-Forwarded-For from the proxy)', r.status === 200);
  } finally { stop(h); }

  jarvis.close();
  console.log(failed ? `\n${failed} TEST(S) FAILED` : '\nALL TESTS PASSED');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error('CRASH', e); process.exit(2); });
