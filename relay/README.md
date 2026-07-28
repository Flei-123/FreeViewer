# FreeViewer Relay

A ~200 line Node.js service. It does exactly two things:

1. maps a host secret to a stable 9 digit ID (`sha256(secret) -> id`, stored in `hosts.json`)
2. pipes binary frames between a host and one viewer

It cannot read anything: the session password never reaches it, and all payload
frames are AES-256-GCM encrypted between host and viewer.

## Run

```bash
npm install ws
node relay.js                 # :7180, websocket path /fv/ws, health at /fv/health
node test_relay.js            # smoke test (9 checks)
```

Environment: `FV_PORT` (default 7180), `FV_DATA` (default `./hosts.json`).

## Behind a reverse proxy

Terminate TLS in nginx/Caddy and forward `/fv/` with websocket upgrade, e.g.:

```nginx
location /fv/ {
  proxy_pass http://127.0.0.1:7180;
  proxy_http_version 1.1;
  proxy_set_header Upgrade $http_upgrade;
  proxy_set_header Connection "upgrade";
  proxy_read_timeout 3600s;
  proxy_buffering off;
}
```

Then point clients at it: `FV_RELAY=wss://your.host/fv/ws`.

## systemd

```ini
[Unit]
Description=FreeViewer Relay
After=network.target

[Service]
ExecStart=/usr/bin/node /opt/freeviewer-relay/relay.js
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

## Protocol (client -> relay, JSON text frames)

| message                                | answer                          |
| -------------------------------------- | ------------------------------- |
| `{"t":"host_register","secret":"<hex>"}` | `{"t":"registered","id":"..."}` |
| `{"t":"connect","id":"123456789"}`       | `{"t":"paired","sid":"..."}` or `{"t":"error","msg":"offline"/"busy"}` |
| `{"t":"bye"}`                            | -                               |

The host additionally receives `{"t":"incoming"}` when a viewer pairs and
`{"t":"peer_gone"}` when it leaves. Everything else is binary and opaque.
