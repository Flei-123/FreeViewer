# Relay: Node 18 (EOL) -> current LTS (r9) - guide, NOT executed

The relay (CT110, `/root/fvrelay`) runs on Node 18, which is end-of-life (no security fixes).
The relay only needs `http`, `fs`, `crypto` and `ws@^8` - nothing Node-18-specific.

1. **Snapshot**: on the PVE host `pct snapshot 110 pre-node-upgrade`.
2. **Check what runs**: `pct exec 110 -- node -v; systemctl cat <relay unit>` (unit name not verified here).
3. **Dry run on a copy, other port**: copy `/root/fvrelay` to `/root/fvrelay-test`, install the new Node,
   run `FV_PORT=7181 FV_DATA=/tmp/h.json node relay.js`, then `node relay/test_relay.js` and `node relay/test_publish.js`
   from this repo (they need `npm install` in the same folder for `ws`).
4. **Install Node** (current LTS - check https://nodejs.org/en/about/previous-releases first; 24.x at the time of writing, 22.x is in maintenance):
   NodeSource: `curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && apt-get install -y nodejs`
   (or `nvm`). `cd /root/fvrelay && npm install --omit=dev` (updates `ws`).
5. `systemctl restart <relay unit>`; check `curl -s localhost:7180/fv/health`, a host re-register and one test session.
6. **Rollback**: `pct rollback 110 pre-node-upgrade` (Node binary + app state go back together; `hosts.json` changes since the snapshot are lost - copy it first).
Also: the JARVIS server itself reports Node v18.20.8 - the same upgrade applies there.
