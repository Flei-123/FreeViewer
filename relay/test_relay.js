// E2E smoke test for the FreeViewer relay: host registers, viewer connects, binary pipe both ways.
// usage: node test_relay.js            (against ws://127.0.0.1:7180/fv/ws)
//        FV_URL=wss://host/fv/ws node test_relay.js
let WebSocket;
try { WebSocket = require('ws'); } catch (e) { WebSocket = require('/root/jarvis/node_modules/ws'); }

const URL = process.env.FV_URL || 'ws://127.0.0.1:7180/fv/ws';
const RESOLVE = process.env.FV_RESOLVE || null; // force an IP (split DNS test)
const WSOPT = RESOLVE ? { lookup: (h, o, cb) => cb(null, RESOLVE, 4) } : {};
const SECRET = 'a'.repeat(64);
let failed = 0;
const ok = (c, m) => { console.log((c ? 'PASS' : 'FAIL') + ' - ' + m); if (!c) failed++; };

function json(ws, o) { ws.send(JSON.stringify(o)); }

(async () => {
  const host = new WebSocket(URL, WSOPT);
  await new Promise(r => host.on('open', r));
  json(host, { t: 'host_register', secret: SECRET });
  const reg = JSON.parse(await new Promise(r => host.once('message', r)));
  ok(reg.t === 'registered' && /^[1-9]\d{8}$/.test(reg.id), 'host registered, id=' + reg.id);

  const host2 = new WebSocket(URL, WSOPT);
  await new Promise(r => host2.on('open', r));
  json(host2, { t: 'host_register', secret: SECRET });
  const reg2 = JSON.parse(await new Promise(r => host2.once('message', r)));
  ok(reg2.id === reg.id, 'stable id across reconnect (' + reg2.id + ')');

  const v0 = new WebSocket(URL, WSOPT);
  await new Promise(r => v0.on('open', r));
  json(v0, { t: 'connect', id: '999999999' });
  const e0 = JSON.parse(await new Promise(r => v0.once('message', r)));
  ok(e0.t === 'error' && e0.msg === 'offline', 'unknown id -> offline');
  v0.close();

  const viewer = new WebSocket(URL, WSOPT);
  await new Promise(r => viewer.on('open', r));
  const hostMsgs = [];
  host2.on('message', (d, bin) => hostMsgs.push(bin ? d : JSON.parse(d.toString())));
  json(viewer, { t: 'connect', id: reg.id });
  const pair = JSON.parse(await new Promise(r => viewer.once('message', r)));
  ok(pair.t === 'paired', 'viewer paired, sid=' + pair.sid);
  await new Promise(r => setTimeout(r, 150));
  ok(hostMsgs.some(m => m.t === 'incoming'), 'host got incoming');

  const payload = Buffer.from([1, 2, 3, 4, 5]);
  viewer.send(payload, { binary: true });
  await new Promise(r => setTimeout(r, 150));
  ok(hostMsgs.some(m => Buffer.isBuffer(m) && m.equals(payload)), 'binary viewer->host');

  const back = Buffer.alloc(65536, 7);
  const got = new Promise(r => viewer.once('message', (d, bin) => r({ d, bin })));
  host2.send(back, { binary: true });
  const g = await got;
  ok(g.bin && Buffer.compare(g.d, back) === 0, 'binary host->viewer (64 KB)');

  const v2 = new WebSocket(URL, WSOPT);
  await new Promise(r => v2.on('open', r));
  json(v2, { t: 'connect', id: reg.id });
  const e2 = JSON.parse(await new Promise(r => v2.once('message', r)));
  ok(e2.t === 'error' && e2.msg === 'busy', 'second viewer -> busy');
  v2.close();

  const gone = new Promise(r => host2.on('message', d => { const m = JSON.parse(d.toString()); if (m.t === 'peer_gone') r(m); }));
  viewer.close();
  const gm = await Promise.race([gone, new Promise(r => setTimeout(() => r(null), 2000))]);
  ok(gm && gm.t === 'peer_gone', 'host notified peer_gone');

  host.close(); host2.close();
  console.log(failed ? '\n' + failed + ' TEST(S) FAILED' : '\nALL TESTS PASSED');
  process.exit(failed ? 1 : 0);
})().catch(e => { console.error('CRASH', e); process.exit(2); });
