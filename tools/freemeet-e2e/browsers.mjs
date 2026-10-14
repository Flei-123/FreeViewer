// FreeMeet E2E browser check (roadmap r62): Chromium / Firefox / WebKit.
//
// Usage (Playwright is installed on the JARVIS server, not in this repo):
//   PW=/root/jarvis/node_modules/playwright/index.mjs \
//   node tools/freemeet-e2e/browsers.mjs inpage <webdir> [chromium|firefox|webkit ...]
//   node tools/freemeet-e2e/browsers.mjs sfu <baseUrl> <chromium|firefox> [<chromium|firefox>]
//
// inpage: no SFU needed. Serves <webdir> (e2e.js, e2e-worker.js) on 127.0.0.1 and runs in each browser:
//   * feature probe + FreeMeetE2E.moeglich()
//   * frame transform core: audio/video round trip, replay drop (r60), tamper drop, clear-VP8 drop
//   * RTCRtpScriptTransform worker path: two RTCPeerConnections in ONE page (canvas + oscillator),
//     both sides E2E -> video must decode; receiver WITHOUT transform -> must NOT decode (proves encryption)
// sfu: two browser clients join a local SFU testhost room with #k=<key> and we read E2E tag + inbound stats.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';

const PW = process.env.PW || '/root/jarvis/node_modules/playwright/index.mjs';
const pw = await import(PW);
const [mode, target, ...rest] = process.argv.slice(2);

let failed = 0;
const check = (name, ok, info = '') => {
  console.log(`${ok ? 'OK  ' : 'FAIL'}  ${name}${info ? '   [' + info + ']' : ''}`);
  if (!ok) failed++;
};

const KEY = 'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8'; // 43 chars -> 32 bytes (test key only)

// Opens a browser; Firefox uses a COPY of the profile template (FF_PROFILE_TPL) so the GMP H.264 plugin is there.
async function openBrowser(name) {
  const opts = { headless: true, ...launchOpts(name) };
  if (name === 'firefox' && process.env.FF_PROFILE_TPL) {
    const dir = fs.mkdtempSync(path.join(process.env.TMPDIR || '/tmp', 'ffp-'));
    fs.cpSync(process.env.FF_PROFILE_TPL, dir, { recursive: true });
    const ctx = await pw.firefox.launchPersistentContext(dir, opts);
    // let the plugin register
    const p0 = ctx.pages()[0] || await ctx.newPage();
    for (let i = 0; i < 15; i++) {
      const h = await p0.evaluate(() => RTCRtpSender.getCapabilities('video').codecs.some((c) => /h264/i.test(c.mimeType))).catch(() => false);
      if (h) break;
      await new Promise((r) => setTimeout(r, 2000));
    }
    return { newContext: async () => ctx, close: async () => { await ctx.close().catch(() => {}); fs.rmSync(dir, { recursive: true, force: true }); } };
  }
  const b = await pw[name].launch(opts);
  return { newContext: (o) => b.newContext(o), close: () => b.close() };
}

function launchOpts(name) {
  if (name === 'chromium') {
    return { args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
      '--autoplay-policy=no-user-gesture-required', '--disable-features=WebRtcHideLocalIpsWithMdns'] };
  }
  if (name === 'firefox') {
    return { firefoxUserPrefs: {
      'media.navigator.streams.fake': true, 'media.navigator.permission.disabled': true,
      'media.peerconnection.ice.loopback': true, 'media.autoplay.default': 0,
      'media.peerconnection.ice.obfuscate_host_addresses': false,
      // Playwright's Firefox has no H.264 until the OpenH264 GMP plugin is downloaded (~20 s, needs internet).
      // Run once with FF_PROFILE_TPL=<empty dir> to fetch it; later runs copy that profile.
      'media.gmp-manager.updateEnabled': true, 'media.gmp-gmpopenh264.enabled': true,
      'media.gmp-gmpopenh264.autoupdate': true, 'media.gmp-provider.enabled': true,
      'media.gmp-gmpopenh264.provider.enabled': true, 'media.peerconnection.video.h264_enabled': true,
    } };
  }
  return {};
}

// ------------------------------------------------------------------ inpage
function serve(dir) {
  const srv = http.createServer((req, res) => {
    const u = req.url.split('?')[0];
    if (u === '/t.html') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><title>t</title>'); return; }
    const f = path.join(dir, path.basename(u));
    if (!fs.existsSync(f) || !fs.statSync(f).isFile()) { res.statusCode = 404; res.end('no'); return; }
    res.setHeader('content-type', 'application/javascript');
    res.end(fs.readFileSync(f));
  });
  return new Promise((r) => srv.listen(0, '127.0.0.1', () => r(srv)));
}

const IN_PAGE = async ({ key }) => {
  const out = {};
  const E = window.FreeMeetE2E;
  out.moeglich = E.moeglich();
  out.features = {
    createEncodedStreams: typeof RTCRtpSender !== 'undefined' && typeof RTCRtpSender.prototype.createEncodedStreams === 'function',
    scriptTransform: typeof RTCRtpScriptTransform !== 'undefined',
    worker: typeof Worker !== 'undefined',
    h264: (() => { try { return RTCRtpSender.getCapabilities('video').codecs.some((c) => /h264/i.test(c.mimeType)); } catch (e) { return null; } })(),
  };
  const raw = E.schluesselBytes(key);
  const handle = await E.schluesselAufbauen(raw);
  // run frames through spurSchuetzen; returns the output frame datas
  const run = async (art, richtung, frames) => {
    const outFrames = [];
    const rs = new ReadableStream({ start(c) { for (const f of frames) c.enqueue(f); c.close(); } });
    const ws = new WritableStream({ write(f) { outFrames.push(f.data); } });
    await E.spurSchuetzen(handle, art, rs, ws, richtung);
    return outFrames;
  };
  const mk = (bytes, type) => ({ data: new Uint8Array(bytes).buffer, type });
  const orig = new Map(); // keep pristine copies: spurSchuetzen rewrites frame.data in place
  // audio
  const audio = Array.from({ length: 40 }, (_, i) => mk(Array.from({ length: 60 }, (_, j) => (i + j) & 255)));
  const enc = await run('audio', 'raus', audio);
  out.audioEncLen = enc.length;
  out.audioEncDiffers = enc.every((d, i) => new Uint8Array(d)[0] === E.E2E_KENNUNG && d.byteLength > 60);
  const dec = await run('audio', 'rein', enc.map((d) => ({ data: d.slice(0) })));
  out.audioRoundTrip = dec.length === 40 && dec.every((d, i) => new Uint8Array(d).every((b, j) => b === ((i + j) & 255)));
  // replay: same encrypted frames twice -> second copy dropped
  const rep = await run('audio', 'rein', [...enc, ...enc].map((d) => ({ data: d.slice(0) })));
  out.audioReplayKept = rep.length;           // expect 40
  // tamper
  const bad = enc.map((d) => { const u = new Uint8Array(d.slice(0)); u[u.length - 1] ^= 1; return { data: u.buffer }; });
  out.audioTamperKept = (await run('audio', 'rein', bad)).length;   // expect 0
  // video H.264 (Annex-B: SPS, PPS, IDR slice)
  const nal = (hdr, n, seed) => [0, 0, 0, 1, hdr, ...Array.from({ length: n }, (_, i) => ((i * 7 + seed) & 0xff) | 0x10)];
  const vbytes = Array.from({ length: 10 }, (_, i) => [...nal(0x67, 8, 1), ...nal(0x68, 4, 2), ...nal(0x65, 200, i)]);
  const vclone = () => vbytes.map((b) => mk(b, 'key'));
  const vframes = vbytes.map((b) => ({ data: new Uint8Array(b) }));
  const venc = await run('video', 'raus', vclone());
  out.videoEncLen = venc.length;
  out.videoSliceChanged = venc.every((d, i) => { const a = new Uint8Array(d), b = vframes[i].data; return a.length > b.length && a[4] === 0x67; });
  const vdec = await run('video', 'rein', venc.map((d) => ({ data: d.slice(0) })));
  out.videoRoundTrip = vdec.length === 10 && vdec.every((d, i) => { const a = new Uint8Array(d), b = vframes[i].data; return a.length === b.length && a.every((x, j) => x === b[j]); });
  out.videoReplayKept = (await run('video', 'rein', [...venc, ...venc].map((d) => ({ data: d.slice(0) })))).length; // expect 10
  // clear VP8-like frame (no start code) must not leave the sender
  out.vp8SentKept = (await run('video', 'raus', [mk(Array.from({ length: 80 }, (_, i) => (i * 3 + 5) & 0xff))])).length; // expect 0
  // clear frame from the network must be dropped by the receiver
  out.clearVideoKept = (await run('video', 'rein', vclone())).length; // expect 0

  // ---- worker + RTCRtpScriptTransform loopback ----
  out.loop = null;
  if (out.features.scriptTransform && out.features.worker && typeof RTCPeerConnection !== 'undefined') {
    const loop = async (receiverTransform) => {
      const res = { error: null };
      try {
        const worker = new Worker('/e2e-worker.js');
        worker.postMessage({ key: handle });
        const cv = document.createElement('canvas'); cv.width = 320; cv.height = 240;
        const g = cv.getContext('2d');
        const vtrack = cv.captureStream(15).getVideoTracks()[0];
        const ac = new AudioContext(); const osc = ac.createOscillator(); const dst = ac.createMediaStreamDestination();
        osc.connect(dst); osc.start();
        const atrack = dst.stream.getAudioTracks()[0];
        const pc1 = new RTCPeerConnection({ encodedInsertableStreams: true });
        const pc2 = new RTCPeerConnection({ encodedInsertableStreams: true });
        pc1.onicecandidate = (e) => e.candidate && pc2.addIceCandidate(e.candidate);
        pc2.onicecandidate = (e) => e.candidate && pc1.addIceCandidate(e.candidate);
        const tv = pc1.addTransceiver(vtrack, { direction: 'sendonly' });
        const ta = pc1.addTransceiver(atrack, { direction: 'sendonly' });
        try {
          const caps = RTCRtpSender.getCapabilities('video').codecs.filter((c) => /^video\/(h264|rtx)$/i.test(c.mimeType));
          if (caps.some((c) => /h264/i.test(c.mimeType))) { tv.setCodecPreferences(caps); res.codec = 'h264'; } else res.codec = 'default(no h264)';
        } catch (e) { res.codecErr = e.message; }
        tv.sender.transform = new RTCRtpScriptTransform(worker, { art: 'video', richtung: 'raus' });
        ta.sender.transform = new RTCRtpScriptTransform(worker, { art: 'audio', richtung: 'raus' });
        pc2.ontrack = (ev) => {
          if (receiverTransform) ev.receiver.transform = new RTCRtpScriptTransform(worker, { art: ev.track.kind, richtung: 'rein' });
        };
        await pc1.setLocalDescription(await pc1.createOffer());
        await pc2.setRemoteDescription(pc1.localDescription);
        await pc2.setLocalDescription(await pc2.createAnswer());
        await pc1.setRemoteDescription(pc2.localDescription);
        const t0 = Date.now();
        let n = 0;
        const timer = setInterval(() => { g.fillStyle = `hsl(${(n++ * 20) % 360},80%,50%)`; g.fillRect(0, 0, 320, 240); g.fillStyle = '#fff'; g.fillRect(n % 300, n % 200, 20, 20); }, 50);
        let dec = 0, pkts = 0, apk = 0, tot = 0, conc = 0;
        while (Date.now() - t0 < 9000) {
          await new Promise((r) => setTimeout(r, 500));
          const st = await pc2.getStats();
          st.forEach((r) => {
            if (r.type === 'inbound-rtp' && r.kind === 'video') { dec = r.framesDecoded || 0; pkts = r.packetsReceived || 0; }
            if (r.type === 'inbound-rtp' && r.kind === 'audio') { apk = r.packetsReceived || 0; tot = r.totalSamplesReceived || 0; conc = r.concealedSamples || 0; }
          });
          if (dec > 5 && Date.now() - t0 > 4000) break;
        }
        clearInterval(timer);
        res.framesDecoded = dec; res.videoPackets = pkts; res.audioPackets = apk; res.audioConcealedPct = tot ? Math.round(1000 * conc / tot) / 10 : null;
        pc1.close(); pc2.close(); worker.terminate(); ac.close();
      } catch (e) { res.error = String(e && e.message || e); }
      return res;
    };
    out.loop = { both: await loop(true), senderOnly: await loop(false) };
  }
  return out;
};

async function inpage(dir, names) {
  const srv = await serve(dir);
  const base = `http://127.0.0.1:${srv.address().port}`;
  for (const name of names) {
    console.log(`\n== ${name} (inpage) ==`);
    let browser;
    try {
      browser = await openBrowser(name);
      const page = await (await browser.newContext()).newPage();
      page.on('pageerror', (e) => console.log('  pageerror:', String(e).slice(0, 200)));
      await page.goto(base + '/t.html');
      await page.addScriptTag({ url: base + '/e2e.js' });
      const o = await page.evaluate(IN_PAGE, { key: KEY });
      console.log('  features', JSON.stringify(o.features));
      const exp = o.features.createEncodedStreams || (o.features.scriptTransform && o.features.worker);
      check(`${name}: moeglich() matches feature set`, o.moeglich === !!exp, `moeglich=${o.moeglich}`);
      check(`${name}: audio encrypted + round trip`, o.audioEncLen === 40 && o.audioEncDiffers && o.audioRoundTrip);
      check(`${name}: audio replay dropped (r60)`, o.audioReplayKept === 40, `kept=${o.audioReplayKept}/80`);
      check(`${name}: audio tamper dropped`, o.audioTamperKept === 0, `kept=${o.audioTamperKept}`);
      check(`${name}: H.264 encrypted + round trip`, o.videoEncLen === 10 && o.videoSliceChanged && o.videoRoundTrip);
      check(`${name}: H.264 replay dropped (r60)`, o.videoReplayKept === 10, `kept=${o.videoReplayKept}/20`);
      check(`${name}: clear VP8 never leaves the sender`, o.vp8SentKept === 0, `kept=${o.vp8SentKept}`);
      check(`${name}: clear video from network dropped`, o.clearVideoKept === 0, `kept=${o.clearVideoKept}`);
      if (o.loop) {
        console.log('  loop both      ', JSON.stringify(o.loop.both));
        console.log('  loop senderOnly', JSON.stringify(o.loop.senderOnly));
        check(`${name}: worker path: video decodes with transform on both sides`, !o.loop.both.error && o.loop.both.framesDecoded > 5, `framesDecoded=${o.loop.both.framesDecoded}`);
        check(`${name}: worker path: audio packets flow`, o.loop.both.audioPackets > 10, `audioPackets=${o.loop.both.audioPackets}`);
        check(`${name}: worker path: encrypted video NOT decodable without receiver transform`, !o.loop.senderOnly.error && o.loop.senderOnly.framesDecoded === 0, `framesDecoded=${o.loop.senderOnly.framesDecoded}`);
      } else {
        console.log('  (no RTCRtpScriptTransform/worker here - worker loopback skipped)');
      }
    } catch (e) {
      check(`${name}: run`, false, String(e).slice(0, 200));
    } finally {
      if (browser) await browser.close().catch(() => {});
    }
  }
  srv.close();
}

// --------------------------------------------------------------------- sfu
async function sfu(base, names) {
  const r = await fetch(base + '/api/meeting', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ titel: 'e2e browsers' }) });
  const m = await r.json();
  console.log('meeting', m.id, 'clients', names.join('+'));
  const browsers = [], pages = [];
  try {
    for (let i = 0; i < names.length; i++) {
      const b = await openBrowser(names[i]);
      browsers.push(b);
      const ctx = await b.newContext({ permissions: names[i] === 'chromium' ? ['camera', 'microphone'] : [] });
      const p = await ctx.newPage();
      p.logs = [];
      p.on('console', (c) => p.logs.push(c.text()));
      p.on('pageerror', (e) => p.logs.push('PAGEERROR ' + e));
      p.ws = [];
      p.on('websocket', (ws) => {
        ws.on('framesent', (f) => { const t = String(f.payload); if (t.includes('"join"')) { const c = JSON.parse(t).caps || {}; p.ws.push(`join e2e=${c.e2e} e2e_ver=${c.e2e_ver}`); } });
        ws.on('framereceived', (f) => { const t = String(f.payload); if (t.includes('"denied"')) p.ws.push('DENIED ' + (JSON.parse(t).msg || '').slice(0, 90)); });
      });
      pages.push(p);
    }
    const link = `${base}/?room=${m.id}&pass=${encodeURIComponent(m.passwort)}&auto=1`;
    for (let i = 0; i < pages.length; i++) {
      await pages[i].goto(`${link}&name=U${i}${process.env.NOKEY ? "" : "#k=" + KEY}`);
      await pages[i].waitForSelector('#meet:not(.hidden)', { timeout: 30000 }).catch(() => {});
      await pages[i].waitForTimeout(1500);
    }
    await pages[0].waitForTimeout(12000);
    for (let i = 0; i < pages.length; i++) {
      const p = pages[i];
      const info = await p.evaluate(async () => {
        const tag = document.getElementById('tag-e2e') || [...document.querySelectorAll('[id*=e2e],.tag')].find((e) => /E2E/.test(e.textContent));
        const stats = { video: 0, audio: 0, vpk: 0, apk: 0 };
        if (typeof state !== 'undefined' && state.pc) {
          (await state.pc.getStats()).forEach((r) => {
            if (r.type === 'inbound-rtp' && r.kind === 'video') { stats.video += r.framesDecoded || 0; stats.vpk += r.packetsReceived || 0; }
            if (r.type === 'inbound-rtp' && r.kind === 'audio') stats.apk += r.packetsReceived || 0;
          });
        }
        const wstats = await new Promise((res) => {
          if (!(typeof state !== 'undefined' && state.e2eWorker)) return res(null);
          state.e2eWorker.addEventListener('message', (m) => res(m.data));
          state.e2eWorker.postMessage({ ping: 1 });
          setTimeout(() => res('timeout'), 2000);
        });
        return { tag: tag ? tag.textContent.trim() : null, stats, wstats, meet: !document.getElementById('meet').classList.contains('hidden'), log: (document.getElementById('log') || {}).textContent };
      }).catch((e) => ({ err: String(e) }));
      console.log(`  client ${i} (${names[i]}):`, JSON.stringify(info).slice(0, process.env.WSTATS ? 2500 : 600));
      console.log('   ws:', p.ws.join(' | '));
      console.log('   console tail:', p.logs.filter((l) => /E2E|Ende|ICE|abgewiesen|rror/i.test(l)).slice(-6).join(' | ').slice(0, 500));
      pages[i].info = info;
    }
    return pages.map((p) => p.info);
  } finally {
    for (const b of browsers) await b.close().catch(() => {});
    await fetch(`${base}/api/meeting/${m.id}?pass=${encodeURIComponent(m.passwort)}`, { method: 'DELETE' }).catch(() => {});
  }
}

if (mode === 'inpage') {
  await inpage(target, rest.length ? rest : ['chromium', 'firefox', 'webkit']);
} else if (mode === 'sfu') {
  await sfu(target, rest);
} else {
  console.log('usage: see header');
  process.exit(2);
}
console.log(failed ? `\n${failed} FAILED` : '\nall ok');
process.exit(failed ? 1 : 0);
