// Dev proxy for the SFU browser check: serves the web files from <webdir> and forwards everything else
// (API + WebSocket upgrade) to the SFU. Needed because the SFU binary embeds its web files at build time.
// Usage: node webproxy.mjs <listenPort> <sfuHttpPort> <webdir>
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';

const [listen, upstream, dir] = process.argv.slice(2);
const types = { js: 'application/javascript', css: 'text/css', html: 'text/html' };

const srv = http.createServer((req, res) => {
  const u = new URL(req.url, 'http://x').pathname;
  const name = u === '/' ? 'index.html' : path.basename(u);
  const f = path.join(dir, name);
  if (/^\/([a-z0-9-]+\.(js|css))?$/.test(u) && fs.existsSync(f) && fs.statSync(f).isFile()) {
    res.writeHead(200, { 'content-type': types[name.split('.').pop()] || 'text/plain', 'cache-control': 'no-store' });
    res.end(fs.readFileSync(f));
    return;
  }
  const up = http.request({ host: '127.0.0.1', port: upstream, path: req.url, method: req.method, headers: req.headers }, (r) => {
    res.writeHead(r.statusCode, r.headers);
    r.pipe(res);
  });
  up.on('error', () => { res.statusCode = 502; res.end('bad gateway'); });
  req.pipe(up);
});

srv.on('upgrade', (req, sock, head) => {
  const up = net.connect(Number(upstream), '127.0.0.1', () => {
    let raw = `${req.method} ${req.url} HTTP/1.1\r\n`;
    for (let i = 0; i < req.rawHeaders.length; i += 2) raw += `${req.rawHeaders[i]}: ${req.rawHeaders[i + 1]}\r\n`;
    up.write(raw + '\r\n');
    if (head && head.length) up.write(head);
    sock.pipe(up);
    up.pipe(sock);
  });
  up.on('error', () => sock.destroy());
  sock.on('error', () => up.destroy());
});

srv.listen(Number(listen), '127.0.0.1');
