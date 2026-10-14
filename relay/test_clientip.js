#!/usr/bin/env node
// Tests for the real client address (r63): unit tests for clientip.js and an
// end-to-end check that the relay puts `client_ip` into the host's `incoming`.
//   node test_clientip.js     (needs the `ws` module like relay.js itself)
'use strict';
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');
const { clientIp } = require('./clientip');
let WebSocket;
try { WebSocket = require('ws'); } catch (e) { WebSocket = require('/root/jarvis/node_modules/ws'); }

let failed = 0;
function check(name, ok) { console.log((ok ? 'ok   ' : 'FAIL ') + name); if (!ok) failed++; }
const req = (peer, xff) => ({ socket: { remoteAddress: peer }, headers: xff === undefined ? {} : { 'x-forwarded-for': xff } });

// --- unit ---
check('no header -> direct peer', clientIp(req('192.168.1.5')) === '192.168.1.5');
check('proxy + single entry', clientIp(req('192.168.1.5', '203.0.113.7')) === '203.0.113.7');
check('proxy + chain -> LAST entry (earlier ones are forgeable)', clientIp(req('192.168.1.5', '6.6.6.6, 203.0.113.7')) === '203.0.113.7');
check('mapped IPv4 peer is treated as private', clientIp(req('::ffff:192.168.1.5', '203.0.113.7')) === '203.0.113.7');
check('loopback proxy', clientIp(req('127.0.0.1', '203.0.113.7')) === '203.0.113.7');
check('public peer: header ignored', clientIp(req('198.51.100.9', '6.6.6.6')) === '198.51.100.9');
check('garbage entry -> direct peer', clientIp(req('192.168.1.5', 'not-an-ip')) === '192.168.1.5');
check('empty header -> direct peer', clientIp(req('192.168.1.5', '')) === '192.168.1.5');
check('IPv6 client', clientIp(req('10.0.0.2', '2001:db8::1')) === '2001:db8::1');
check('mapped IPv4 client is normalised', clientIp(req('10.0.0.2', '::ffff:203.0.113.7')) === '203.0.113.7');
check('trailing garbage after comma -> direct peer', clientIp(req('192.168.1.5', '203.0.113.7,')) === '192.168.1.5');

// --- end to end ---
async function e2e() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fvip-'));
  const port = 20000 + Math.floor(Math.random() * 20000);
  const child = spawn(process.execPath, [path.join(__dirname, 'relay.js')], {
    env: { ...process.env, FV_PORT: String(port), FV_DATA: path.join(dir, 'hosts.json'), FV_DIST: path.join(dir, 'dist'), FV_UPLOAD_TOKEN_FILE: path.join(dir, 'tok') },
    stdio: 'ignore',
  });
  const cleanup = () => { try { child.kill(); } catch (_) {} fs.rmSync(dir, { recursive: true, force: true }); };
  try {
    await new Promise((r) => setTimeout(r, 800));
    const url = `ws://127.0.0.1:${port}/fv/ws`;
    const host = new WebSocket(url);
    await new Promise((r) => host.on('open', r));
    const inbox = [];
    host.on('message', (d, bin) => { if (!bin) inbox.push(JSON.parse(d.toString())); });
    host.send(JSON.stringify({ t: 'host_register', secret: 'b'.repeat(64) }));
    await new Promise((r) => setTimeout(r, 300));
    const id = (inbox.find((m) => m.t === 'registered') || {}).id;
    check('host registered', !!id);

    const viewer = new WebSocket(url, { headers: { 'X-Forwarded-For': '6.6.6.6, 203.0.113.7' } });
    await new Promise((r) => viewer.on('open', r));
    viewer.send(JSON.stringify({ t: 'connect', id }));
    await new Promise((r) => setTimeout(r, 400));
    const inc = inbox.find((m) => m.t === 'incoming');
    check('incoming carries client_ip from the proxy header', !!inc && inc.client_ip === '203.0.113.7');
    check('incoming still carries legacy `from`', !!inc && typeof inc.from === 'string' && inc.from.length > 0);
    viewer.close();
    await new Promise((r) => setTimeout(r, 200));

    const v2 = new WebSocket(url);
    await new Promise((r) => v2.on('open', r));
    v2.send(JSON.stringify({ t: 'connect', id }));
    await new Promise((r) => setTimeout(r, 400));
    const inc2 = inbox.filter((m) => m.t === 'incoming')[1];
    check('no header -> client_ip is the direct peer', !!inc2 && /127\.0\.0\.1$/.test(inc2.client_ip));
    v2.close(); host.close();
  } finally { cleanup(); }
}

e2e().then(() => {
  console.log(failed ? `\n${failed} TEST(S) FAILED` : '\nALL TESTS PASSED');
  process.exit(failed ? 1 : 0);
}).catch((e) => { console.error('CRASH', e); process.exit(2); });
