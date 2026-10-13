// Node test for web-patch/e2e.js (WebCrypto) against the shared vectors
// written by tools/freemeet-e2e/ref.py.   Run:  node --test web-patch/
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { webcrypto } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
if (!globalThis.crypto) globalThis.crypto = webcrypto;
globalThis.window = globalThis;
vm.runInThisContext(fs.readFileSync(path.join(here, 'e2e.js'), 'utf8'), { filename: 'e2e.js' });
const E2E = globalThis.FreeMeetE2E;
const V = JSON.parse(fs.readFileSync(path.join(here, '..', 'tests', 'freemeet-e2e-vectors.json'), 'utf8'));

const hex = (h) => Uint8Array.from(Buffer.from(h, 'hex'));
const toHex = (u8) => Buffer.from(u8).toString('hex');
const root = hex(V.root_hex);

/** Independent raw key derivation (the CryptoKey of the module is not extractable). */
async function rawKey(kind, salt) {
  const enc = new TextEncoder();
  const base = await crypto.subtle.importKey('raw', root, 'HKDF', false, ['deriveBits']);
  const info = new Uint8Array(kind.length + 1 + salt.length);
  info.set(enc.encode(kind), 0);
  info.set(salt, kind.length + 1);
  const bits = await crypto.subtle.deriveBits(
    { name: 'HKDF', hash: 'SHA-256', salt: enc.encode('freemeet-e2e-v2'), info }, base, 256);
  return toHex(new Uint8Array(bits));
}

const handle = () => E2E.schluesselAufbauen(root);

/** Run frames through spurSchuetzen and collect the output. */
async function run(h, art, richtung, frames) {
  const out = [];
  const readable = new ReadableStream({
    start(c) { for (const f of frames) c.enqueue(f); c.close(); },
  });
  const writable = new WritableStream({ write(f) { out.push(f); } });
  await E2E.spurSchuetzen(h, art, readable, writable, richtung);
  return out;
}
const frame = (u8) => ({ data: u8.slice().buffer });

const sample = hex(V.h264[0].plain);

test('key derivation matches the reference', async () => {
  for (const e of V.keys) assert.equal(await rawKey(e.spur, hex(e.salt)), e.key);
});

test('nonce layout matches the reference', () => {
  for (const e of V.nonces) {
    const n = E2E.nonce(e.spur === 'ton' ? 1 : 2, e.nal, hex(e.salt), e.counter);
    assert.equal(toHex(n), e.nonce);
  }
});

test('audio: sealing reproduces the reference frames, opening returns the plaintext', async () => {
  for (const e of V.audio) {
    const h = await handle();
    const got = await E2E.schuetzenTon(h, hex(e.salt), e.counter, e.keyframe, hex(e.plain));
    assert.equal(toHex(got), e.frame);
    const fresh = await handle(); // empty key cache
    const open = await E2E.oeffnenTon(fresh, hex(e.frame));
    assert.equal(toHex(open.klar), e.plain);
    assert.equal(open.zaehler, e.counter);
  }
});

test('h264: sealing reproduces the reference frames, opening returns the plaintext', async () => {
  for (const e of V.h264) {
    const h = await handle();
    const got = await E2E.schuetzenH264(h, hex(e.salt), e.counter, hex(e.plain));
    assert.equal(toHex(got), e.frame);
    const fresh = await handle();
    assert.equal(toHex(await E2E.oeffnenH264(fresh, hex(e.frame))), e.plain);
  }
});

test('tampering with the clear header or ciphertext is detected', async () => {
  const e = V.audio[1];
  const h = await handle();
  for (const pos of [1, 2, 9, 10, 13, 14, 20, hex(e.frame).length - 1]) {
    const bad = hex(e.frame);
    bad[pos] ^= 1;
    assert.equal(await E2E.oeffnenTon(h, bad), null, 'position ' + pos);
  }
  const g = hex(V.h264[0].frame);
  g[g.length - 40] ^= 1;
  assert.equal(await E2E.oeffnenH264(h, g), null);
});

test('a wrong room key opens nothing', async () => {
  const other = await E2E.schluesselAufbauen(new Uint8Array(32).fill(7));
  assert.equal(await E2E.oeffnenTon(other, hex(V.audio[0].frame)), null);
  assert.equal(await E2E.oeffnenH264(other, hex(V.h264[0].frame)), null);
});

test('round trip through spurSchuetzen (raus -> rein), replay is dropped', async () => {
  const h = await handle();
  const sent = await run(h, 'audio', 'raus', [frame(hex('aabbccdd11223344'))]);
  assert.equal(sent.length, 1);
  const wire = new Uint8Array(sent[0].data);
  assert.equal(wire[0], 0xe3);
  // receiver gets the same frame twice: the second one is a replay
  const got = await run(await handle(), 'audio', 'rein', [frame(wire), frame(wire)]);
  assert.equal(got.length, 1, 'replayed frame must be dropped');
  assert.equal(toHex(new Uint8Array(got[0].data)), 'aabbccdd11223344');

  const vsent = await run(h, 'video', 'raus', [frame(sample)]);
  const vgot = await run(await handle(), 'video', 'rein', vsent);
  assert.equal(toHex(new Uint8Array(vgot[0].data)), toHex(sample));
});

test('old (v1) frames and unencrypted frames are dropped, not shown', async () => {
  const h = await handle();
  const v1 = new Uint8Array(60); v1[0] = 0xe2; v1[1] = 1;
  const plain = new Uint8Array(60).fill(0x55);
  assert.equal((await run(h, 'audio', 'rein', [frame(v1), frame(plain)])).length, 0);
  // v1 video blob: starts with the NAL index, not 0xe3
  const v1video = new Uint8Array([0, 0, 0, 1, 0x65, 0, 0, 0, 0, 5, ...Array.from({ length: 40 }, (_, i) => (i * 7 + 9) & 0xff)]);
  assert.equal((await run(h, 'video', 'rein', [frame(v1video)])).length, 0);
  assert.equal(await E2E.oeffnenH264(h, v1video), null);
});

test('(key, nonce) never repeats over 2 senders x 3 streams x rejoin', async () => {
  const seen = new Set();
  const nonces = new Set();
  let pairs = 0;
  const note = async (kind, salt, nal, counter) => {
    const k = (await rawKey(kind, salt)) + ':' + toHex(E2E.nonce(kind === 'ton' ? 1 : 2, nal, salt, counter));
    pairs++;
    nonces.add(toHex(E2E.nonce(kind === 'ton' ? 1 : 2, nal, salt, counter)));
    assert.ok(!seen.has(k), '(key, nonce) used twice');
    seen.add(k);
  };
  for (let join = 0; join < 2; join++) {              // first join + rejoin
    for (let person = 0; person < 2; person++) {      // two participants, same room key
      const h = await handle();
      const mic = await run(h, 'audio', 'raus', Array.from({ length: 50 }, () => frame(hex('0102030405060708'))));
      const cam = await run(h, 'video', 'raus', Array.from({ length: 50 }, () => frame(sample)));
      const scr = await run(h, 'video', 'raus', Array.from({ length: 50 }, () => frame(sample)));
      for (const f of mic) {
        const d = new Uint8Array(f.data);
        const dv = new DataView(d.buffer, d.byteOffset);
        await note('ton', d.slice(2, 10), 0, dv.getUint32(10, false));
      }
      for (const f of [...cam, ...scr]) {
        for (const [, db, end] of E2E.nalEinheiten(new Uint8Array(f.data))) {
          const d = new Uint8Array(f.data);
          if ([7, 8, 9].includes(d[db] & 0x1f) || end === db + 1) continue;
          const raw = E2E.epbRaus(d.slice(db + 1, end));
          const dv = new DataView(raw.buffer);
          await note('bild', raw.slice(1, 9), dv.getUint16(13, false), dv.getUint32(9, false));
        }
      }
    }
  }
  assert.equal(pairs, 2 * 2 * 50 * 5);
  assert.equal(seen.size, pairs);
  assert.equal(nonces.size, pairs, 'even the bare nonce must not repeat');
});

test('a sender rotates to a new salt before its counter wraps', () => {
  const s = E2E.neuerSender(hex('0102030405060708'));
  s.setzeNaechsten(0xffffffff);
  const last = s.nimm();
  assert.equal(last.zaehler, 0xffffffff);
  const next = s.nimm();
  assert.equal(next.zaehler, 0);
  assert.notEqual(toHex(next.salz), toHex(last.salz));
});

test('replay guard keeps one window per sender salt', () => {
  const w = E2E.neueWache();
  const a = hex('a1a2a3a4a5a6a7a8'), b = hex('0102030405060708');
  assert.ok(w(a, 5)); assert.ok(w(b, 5));
  assert.ok(!w(a, 5)); assert.ok(!w(b, 5));
  assert.ok(w(a, 13)); assert.ok(w(a, 12)); assert.ok(!w(a, 12));
  assert.ok(w(a, 200)); assert.ok(!w(a, 13));
});

test('forged salts do not fill the key cache', async () => {
  const h = await handle();
  const real = hex(V.audio[0].frame);
  const rx = await handle();
  for (let i = 0; i < 100; i++) {
    const f = real.slice();
    f[2] = i; f[3] = i >> 8;
    await E2E.oeffnenTon(rx, f);
  }
  assert.equal(rx.cache.size, 0);
  assert.ok(await E2E.oeffnenTon(rx, real));
  assert.equal(rx.cache.size, 1);
  void h;
});
