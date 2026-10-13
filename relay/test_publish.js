#!/usr/bin/env node
// Tests for the hardened POST /fv/publish (token in header, LAN-only,
// timing-safe compare, Ed25519 signature check + pass-through).
//   node test_publish.js          (needs the `ws` module like relay.js itself)
'use strict';
const http = require('http');
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { spawn } = require('child_process');

const TOKEN = 'unit-test-token-0123456789';
const PAYLOAD = Buffer.from('MZ-fake-exe-payload');
let failed = 0;
function check(name, ok) {
  console.log((ok ? 'ok   ' : 'FAIL ') + name);
  if (!ok) failed++;
}

// test key pair (never used outside this test)
const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
/* english: ok - DER is a key encoding */ const rawPub = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex');
function sign(version, sha, size) {
  const msg = Buffer.from(['freeviewer-update-v1', 'win', version, sha, String(size)].join('\n'));
  return crypto.sign(null, msg, privateKey).toString('hex');
}

function request(port, opts, body) {
  return new Promise((resolve, reject) => {
    const r = http.request({ host: '127.0.0.1', port, ...opts }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString() }));
    });
    r.on('error', reject);
    if (body) r.write(body);
    r.end();
  });
}

async function startRelay(port, extraEnv) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fvpub-'));
  fs.writeFileSync(path.join(dir, 'tok'), TOKEN + '\n');
  const env = {
    ...process.env,
    FV_PORT: String(port),
    FV_DATA: path.join(dir, 'hosts.json'),
    FV_DIST: path.join(dir, 'dist'),
    FV_UPLOAD_TOKEN_FILE: path.join(dir, 'tok'),
    FV_UPDATE_PUBKEY: rawPub,
    ...extraEnv,
  };
  const child = spawn(process.execPath, [path.join(__dirname, 'relay.js')], { env, stdio: 'ignore' });
  for (let i = 0; i < 50; i++) {
    try {
      const r = await request(port, { path: '/fv/health', method: 'GET' });
      if (r.status === 200) return { child, dir };
    } catch (_) {}
    await new Promise((r) => setTimeout(r, 100));
  }
  child.kill();
  throw new Error('relay did not start');
}
function stop(h) {
  h.child.kill();
  fs.rmSync(h.dir, { recursive: true, force: true });
}

const pub = (port, query, headers) =>
  request(port, { path: '/fv/publish?' + query, method: 'POST', headers }, PAYLOAD);

(async () => {
  const sha = crypto.createHash('sha256').update(PAYLOAD).digest('hex');
  const q = 'version=1.2.3&name=freeviewer.exe';
  const goodSig = sign('1.2.3', sha, PAYLOAD.length);

  // ---- default config ----
  let h = await startRelay(17181, {});
  try {
    let r = await pub(17181, q + '&token=' + TOKEN, {});
    check('token in URL query is refused', r.status === 403);
    r = await pub(17181, q, {});
    check('no token is refused', r.status === 403);
    r = await pub(17181, q, { authorization: 'Bearer wrong-token' });
    check('wrong token is refused', r.status === 403);
    r = await pub(17181, q, { authorization: 'Bearer ' + TOKEN, 'x-forwarded-for': '203.0.113.9' });
    check('correct token but via reverse proxy (X-Forwarded-For) is refused', r.status === 403);
    r = await pub(17181, q, { authorization: 'Bearer ' + TOKEN, 'x-real-ip': '203.0.113.9' });
    check('correct token but X-Real-IP is refused', r.status === 403);
    r = await pub(17181, q, { authorization: 'Bearer ' + TOKEN, 'x-fv-sig': 'zz' });
    check('malformed signature header is refused', r.status === 400);
    const bad = goodSig.slice(0, 10) + (goodSig[10] === '0' ? '1' : '0') + goodSig.slice(11);
    r = await pub(17181, q, { authorization: 'Bearer ' + TOKEN, 'x-fv-sig': bad });
    check('invalid signature is refused', r.status === 400);
    r = await request(17181, { path: '/fv/version', method: 'GET' });
    check('feed untouched after refused publishes', r.status === 404);
    r = await pub(17181, q, { authorization: 'Bearer ' + TOKEN, 'x-fv-sig': sign('1.2.4', sha, PAYLOAD.length) });
    check('signature for another version is refused', r.status === 400);
    r = await pub(17181, q, { 'x-upload-token': TOKEN, 'x-fv-sig': goodSig });
    check('valid token (X-Upload-Token) + valid signature is accepted', r.status === 200);
    r = await request(17181, { path: '/fv/version', method: 'GET' });
    const feed = JSON.parse(r.body);
    check('feed carries sha256 + the passed-through signature', feed.sha256 === sha && feed.sig === goodSig && feed.version === '1.2.3');
    r = await pub(17181, 'version=1.2.5&name=freeviewer.exe', { authorization: 'Bearer ' + TOKEN });
    check('unsigned publish still works while FV_REQUIRE_SIG is off', r.status === 200);
    r = await request(17181, { path: '/fv/version', method: 'GET' });
    check('unsigned publish leaves no stale signature in the feed', JSON.parse(r.body).sig === undefined);
  } finally { stop(h); }

  // ---- signature required ----
  h = await startRelay(17182, { FV_REQUIRE_SIG: '1' });
  try {
    let r = await pub(17182, q, { authorization: 'Bearer ' + TOKEN });
    check('FV_REQUIRE_SIG=1: unsigned publish is refused', r.status === 400);
    r = await pub(17182, q, { authorization: 'Bearer ' + TOKEN, 'x-fv-sig': goodSig });
    check('FV_REQUIRE_SIG=1: signed publish is accepted', r.status === 200);
  } finally { stop(h); }

  // ---- legacy switch for old publish scripts ----
  h = await startRelay(17183, { FV_PUBLISH_QUERY_TOKEN: '1' });
  try {
    const r = await pub(17183, q + '&token=' + TOKEN, {});
    check('FV_PUBLISH_QUERY_TOKEN=1: query token accepted (migration switch)', r.status === 200);
  } finally { stop(h); }

  console.log(failed ? `\n${failed} FAILED` : '\nall passed');
  process.exit(failed ? 1 : 0);
})().catch((e) => { console.error(e); process.exit(2); });
