#!/usr/bin/env node
// Tests for relay patch 0005: r32 meeting invitations between friends (mailbox) and
// r30 setup download file name per brand. Real relay process + fake JARVIS.
//   node test_einladung.js   (needs `ws` like relay.js)
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
const TOK2 = 'tok-anna-0123456789xx';
const SA = 'a'.repeat(64);
const SB = 'b'.repeat(64);
const SC = 'c'.repeat(64);

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
const post = (port, p, obj, headers) => request(port, {
  path: p, method: 'POST', headers: { 'content-type': 'application/json', ...(headers || {}) },
}, JSON.stringify(obj));

function startJarvis() {
  return new Promise((resolve) => {
    const srv = http.createServer((req, res) => {
      const m = /fleitec_session=([^;]+)/.exec(req.headers.cookie || '');
      if (req.url === '/api/me' && m && (m[1] === TOK || m[1] === TOK2)) {
        res.writeHead(200, { 'content-type': 'application/json' });
        return res.end(JSON.stringify({ username: m[1] === TOK ? 'Justin' : 'Anna' }));
      }
      res.writeHead(401); res.end('{}');
    });
    srv.listen(0, '127.0.0.1', () => resolve(srv));
  });
}

async function startRelay(port, jarvisPort, extraEnv) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fveinl-'));
  fs.mkdirSync(path.join(dir, 'dist'));
  fs.writeFileSync(path.join(dir, 'dist', 'freeviewer.exe'), 'FV-EXE');
  fs.writeFileSync(path.join(dir, 'dist', 'x-remote.exe'), 'XR-EXE');
  const env = {
    ...process.env,
    FV_PORT: String(port),
    FV_DATA: path.join(dir, 'hosts.json'),
    FV_DIST: path.join(dir, 'dist'),
    FV_UPLOAD_TOKEN_FILE: path.join(dir, 'tok'),
    FV_ACC: path.join(dir, 'acc'),
    FV_SETUPS: path.join(dir, 'setups.json'),
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

// fake address book (fleikontakte): Justin and Anna are friends
function startKontakte() {
  return new Promise((resolve) => {
    const srv = http.createServer((req, res) => {
      const t = ((req.headers.authorization || '').match(/^Bearer (.+)$/) || [])[1];
      const ich = t === TOK ? { konto: 'u-justin', handle: 'justin' } : t === TOK2 ? { konto: 'u-anna', handle: 'anna' } : null;
      const out = (code, obj) => { res.writeHead(code, { 'content-type': 'application/json' }); res.end(JSON.stringify(obj)); };
      if (!ich) return out(401, { error: 'no' });
      if (req.url === '/api/kontakte/profil') return out(200, { profil: { konto: ich.konto, handle: ich.handle, geraete: [] } });
      if (req.url === '/api/kontakte/geraet') { req.resume(); return req.on('end', () => out(200, { ok: true })); }
      if (req.url === '/api/kontakte/liste') {
        const other = ich.konto === 'u-justin' ? { konto: 'u-anna', handle: 'anna', name: 'Anna' } : { konto: 'u-justin', handle: 'justin', name: 'Justin' };
        return out(200, { items: [{ konto: other.konto, status: 'freunde', seit: 1, anzeige: other.name, profil: { handle: other.handle, name: other.name }, geraete: [] }] });
      }
      return out(404, { error: 'unbekannt' });
    });
    srv.listen(0, '127.0.0.1', () => resolve(srv));
  });
}

async function registerHost(port, secret, name) {
  const ws = new WebSocket(`ws://127.0.0.1:${port}/fv/ws`);
  await new Promise((r) => ws.on('open', r));
  const reg = new Promise((r) => ws.once('message', (d) => r(JSON.parse(d.toString()))));
  ws.send(JSON.stringify({ t: 'host_register', secret, name }));
  const m = await reg;
  return { ws, id: m.id };
}

const LINK = 'https://meet.fleitec.com/?room=482-913-770&pass=abc123';

(async () => {
  const jarvis = await startJarvis();
  const h = await startRelay(17291, jarvis.address().port);
  const P = 17291;
  const socks = [];
  try {
    const a = await registerHost(P, SA, 'PC-A'); socks.push(a.ws);
    const b = await registerHost(P, SB, 'PC-B'); socks.push(b.ws);
    const c = await registerHost(P, SC, 'PC-C'); socks.push(c.ws);
    const A = { secret: SA, name: 'PC-A' };
    const B = { secret: SB, name: 'PC-B' };
    const C = { secret: SC, name: 'PC-C' };

    // not friends yet
    let r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK, titel: 'Mit B' });
    check('r32: no friendship -> 403', r.status === 403);

    // friendship A <-> B (old store)
    await post(P, '/fv/freunde/anfragen', { ...A, an: b.id });
    await post(P, '/fv/freunde/annehmen', { ...B, von: a.id });

    r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK, titel: 'Mit B' });
    check('r32: friend -> 200 delivered', r.status === 200 && r.json.zugestellt === true);

    r = await post(P, '/fv/freunde/einladungen', B);
    check('r32: recipient sees exactly one invitation', r.status === 200 && r.json.einladungen.length === 1);
    const e = r.json.einladungen[0] || {};
    check('r32: invitation carries link, title and sender', e.link === LINK && e.titel === 'Mit B' && e.fvid === a.id && !!e.id);

    r = await post(P, '/fv/freunde/einladungen', A);
    check('r32: sender has none in his own mailbox', r.status === 200 && r.json.einladungen.length === 0);
    r = await post(P, '/fv/freunde/einladungen', C);
    check('r32: a stranger sees nothing', r.status === 200 && r.json.einladungen.length === 0);

    // same link again = refresh, not a second entry
    await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK, titel: 'Mit B' });
    r = await post(P, '/fv/freunde/einladungen', B);
    check('r32: same link twice -> still one entry', r.json.einladungen.length === 1);

    // validation
    r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK + '#k=' + 'x'.repeat(43) });
    check('r32: link with an E2E key fragment is refused', r.status === 400);
    r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: 'http://meet.fleitec.com/?room=1' });
    check('r32: plain http link is refused', r.status === 400);
    r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: 'javascript:alert(1)' });
    check('r32: javascript: link is refused', r.status === 400);
    r = await post(P, '/fv/freunde/einladen', { ...A, an: a.id, link: LINK });
    check('r32: invitation to yourself is refused', r.status === 400);
    r = await post(P, '/fv/freunde/einladen', { ...A, link: LINK });
    check('r32: missing id -> 400', r.status === 400);
    r = await post(P, '/fv/freunde/einladen', { secret: 'f'.repeat(64), an: b.id, link: LINK });
    check('r32: unknown device -> 401', r.status === 401);

    // stranger C cannot invite B
    r = await post(P, '/fv/freunde/einladen', { ...C, an: b.id, link: LINK });
    check('r32: non-friend C -> 403', r.status === 403);

    // dismiss: only the owner of the mailbox can
    const id = e.id;
    r = await post(P, '/fv/freunde/einladung-weg', { ...C, id });
    check('r32: a stranger cannot delete my invitation', r.json.weg === 0);
    r = await post(P, '/fv/freunde/einladung-weg', { ...B, id });
    check('r32: owner deletes it', r.status === 200 && r.json.weg === 1);
    r = await post(P, '/fv/freunde/einladungen', B);
    check('r32: mailbox empty afterwards', r.json.einladungen.length === 0);

    // persisted next to the friend list, mode 0600
    await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK + '&x=1', titel: 'x'.repeat(200) });
    const fp = path.join(h.dir, 'freunde.json');
    const st = fs.statSync(fp);
    const db = JSON.parse(fs.readFileSync(fp, 'utf8'));
    check('r32: stored in freunde.json (mode 0600)', Array.isArray(db.einladungen) && db.einladungen.length === 1 && (st.mode & 0o777) === 0o600);
    check('r32: title is cut to 80 chars', db.einladungen[0].titel.length === 80);

    // block: blocked sender gets "ok" but nothing is delivered
    await post(P, '/fv/freunde/blockieren', { ...B, fvid: a.id });
    await post(P, '/fv/freunde/einladung-weg', { ...B, ids: db.einladungen.map((x) => x.id) });
    r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK + '&y=2' });
    check('r32: blocked sender is not told', r.status === 200);
    r = await post(P, '/fv/freunde/einladungen', B);
    check('r32: blocked sender delivers nothing', r.json.einladungen.length === 0);
    await post(P, '/fv/freunde/blockieren', { ...B, fvid: a.id, an: false });

    // burst limit
    let last = 0;
    for (let i = 0; i < 8; i++) {
      r = await post(P, '/fv/freunde/einladen', { ...A, an: b.id, link: LINK + '&n=' + i });
      last = r.status;
    }
    check('r32: burst is limited (429)', last === 429);

    // old endpoints still work
    r = await post(P, '/fv/freunde/stand', A);
    check('r32: /stand still answers the old shape', r.status === 200 && Array.isArray(r.json.freunde) && r.json.freunde.length === 1 && !('einladungen' in r.json));
    r = await post(P, '/fv/freunde/unbekannt', A);
    check('r32: unknown path still 404', r.status === 404);

    // ================= r30: file name per brand =================
    const bearer = { authorization: 'Bearer ' + TOK };
    const mk = async (brand) => (await post(P, '/fv/setup/create', { password: 'secret-pw', name_hint: 'x', brand }, bearer)).json;
    const fvCode = await mk('freeviewer');
    const xoCode = await mk('xoffi');
    check('r30: setup codes created', !!(fvCode && fvCode.code) && !!(xoCode && xoCode.code));
    r = await request(P, { path: '/fv/setup/dl/' + fvCode.code + '?name=PC' });
    check('r30: FreeViewer link -> FreeViewer-Einrichtung.exe', r.status === 200 && /filename="FreeViewer-Einrichtung\.exe"/.test(r.headers['content-disposition'] || ''));
    check('r30: FreeViewer link delivers the FreeViewer exe', r.text.startsWith('FV-EXE') && r.text.includes('FVSETUP1:'));
    r = await request(P, { path: '/fv/setup/dl/' + xoCode.code + '?name=PC' });
    check('r30: X-Remote link -> X-Remote-Einrichtung.exe', r.status === 200 && /filename="X-Remote-Einrichtung\.exe"/.test(r.headers['content-disposition'] || ''));
    check('r30: X-Remote link delivers the X-Remote exe', r.text.startsWith('XR-EXE'));
    r = await request(P, { path: '/fv/setup/dl/' + xoCode.code + '?brand=freeviewer' });
    check('r30: ?brand= override changes file and name together', r.text.startsWith('FV-EXE') && /FreeViewer-Einrichtung/.test(r.headers['content-disposition'] || ''));
    r = await request(P, { path: '/fv/setup/dl/' + fvCode.code + '?brand=nonsense' });
    check('r30: unknown brand falls back to FreeViewer', r.status === 200 && /FreeViewer-Einrichtung/.test(r.headers['content-disposition'] || ''));
    r = await request(P, { path: '/fv/setup/dl/NOPE' });
    check('r30: unknown code -> 404', r.status === 404);
  } catch (e) {
    console.log('ERROR ' + (e && e.stack || e));
    console.log(h.getLog().split('\n').slice(-15).join('\n'));
    failed++;
  } finally {
    socks.forEach((w) => { try { w.close(); } catch (_) {} });
    stop(h);
  }
  // ================= r32 with the address book (logged-in friends) =================
  const kontakte = await startKontakte();
  const h2 = await startRelay(17292, jarvis.address().port, {
    FV_BUCH: '1',
    FV_KONTAKTE: 'http://127.0.0.1:' + kontakte.address().port,
    FV_BUCH_GERAETE: path.join(os.tmpdir(), 'fveinl-buch-' + process.pid + '.json'),
  });
  const socks2 = [];
  try {
    const P2 = 17292;
    const j = await registerHost(P2, SA, 'PC-Justin'); socks2.push(j.ws);
    const an = await registerHost(P2, SB, 'PC-Anna'); socks2.push(an.ws);
    const st = await registerHost(P2, SC, 'PC-Stranger'); socks2.push(st.ws);
    // both devices bind to their accounts the first time they talk to the book
    let r = await post(P2, '/fv/freunde/stand', { secret: SB, name: 'PC-Anna', token: TOK2 });
    check('book: Anna binds her device', r.status === 200 && r.json.buch === true);
    r = await post(P2, '/fv/freunde/stand', { secret: SA, name: 'PC-Justin', token: TOK });
    check('book: Justin sees Anna with her device', r.status === 200 && r.json.freunde.length === 1 && r.json.freunde[0].geraete[0].fvid === an.id);
    r = await post(P2, '/fv/freunde/einladen', { secret: SA, name: 'PC-Justin', token: TOK, an: an.id, link: LINK, titel: 'Mit Anna' });
    check('book: friend via address book -> delivered', r.status === 200 && r.json.zugestellt === true);
    r = await post(P2, '/fv/freunde/einladungen', { secret: SB, name: 'PC-Anna', token: TOK2 });
    check('book: Anna (logged in) gets the invitation', r.status === 200 && r.json.einladungen.length === 1 && r.json.einladungen[0].link === LINK);
    r = await post(P2, '/fv/freunde/einladen', { secret: SC, name: 'PC-Stranger', an: an.id, link: LINK });
    check('book: stranger without account -> 403', r.status === 403);
    r = await post(P2, '/fv/freunde/stand', { secret: SA, name: 'PC-Justin', token: TOK });
    check('book: /stand of an account unchanged by the mailbox', r.status === 200 && !('einladungen' in r.json));
  } catch (e) {
    console.log('ERROR ' + (e && e.stack || e));
    console.log(h2.getLog().split('\n').slice(-15).join('\n'));
    failed++;
  } finally {
    socks2.forEach((w) => { try { w.close(); } catch (_) {} });
    stop(h2);
    kontakte.close();
    try { fs.unlinkSync(path.join(os.tmpdir(), 'fveinl-buch-' + process.pid + '.json')); } catch (_) {}
    jarvis.close();
  }
  console.log(failed ? `\n${failed} FAILED` : '\nall ok');
  process.exit(failed ? 1 : 0);
})();
