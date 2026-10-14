'use strict';
// Real client address behind a reverse proxy (r63).
//
// The relay sits behind NPM/nginx on the LAN, so `req.socket.remoteAddress` is
// the proxy (192.168.x.x) for every viewer. The host's brute-force guard needs
// the real viewer address. Rules:
//  - X-Forwarded-For is trusted ONLY when the direct peer is a private/loopback
//    address (our own proxy). A client talking to the relay directly could send
//    any header it likes.
//  - Only the LAST entry counts: the proxy appends the address it saw, anything
//    before it was typed by the client and can be forged.
//  - The result must be a valid IP literal, otherwise fall back to the peer.

const net = require('net');

function normalize(addr) {
  let a = String(addr || '').trim().toLowerCase();
  if (a.startsWith('::ffff:')) a = a.slice(7);
  return a;
}

function isPrivateAddr(addr) {
  const a = normalize(addr);
  if (a === '::1' || a === '127.0.0.1') return true;
  if (/^127\./.test(a) || /^10\./.test(a) || /^192\.168\./.test(a)) return true;
  const m = /^172\.(\d+)\./.exec(a);
  if (m && +m[1] >= 16 && +m[1] <= 31) return true;
  return /^f[cd][0-9a-f]{2}:/.test(a); // IPv6 unique-local
}

function clientIp(req) {
  const peer = normalize(req.socket && req.socket.remoteAddress);
  if (!isPrivateAddr(peer)) return peer;
  const xff = req.headers && req.headers['x-forwarded-for'];
  if (!xff) return peer;
  const parts = String(Array.isArray(xff) ? xff.join(',') : xff).split(',');
  const last = normalize(parts[parts.length - 1]);
  return net.isIP(last) ? last : peer;
}

module.exports = { clientIp, isPrivateAddr, normalize };
