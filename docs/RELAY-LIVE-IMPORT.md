# Relay code in git (r23)

Before: `relay/relay.js` had 178 lines (the first version); the live relay was ~870 lines and only existed on the server.

## What is in the repo now
`relay/{relay.js, account.js, setups.js, buch.js, package.json}` = working copy `/root/work/fvrelay-r7/` on the JARVIS server
(24.09.2026, 874 lines; next to it `relay.orig.js`, 841 lines, presumably the copy pulled from live before the r7 edits - not proven), commit `cc41c25`.
Routes: `/fv/ws`, `/fv/health`, `/fv/version`, `/fv/dist/`, `/fv/publish`, `/fv/online`, `/fv/account*`, `/fv/freunde/*`, `/fv/setup*`.
The patch on top (publish hardening, `relay/patches/0001-publish-hardening.patch`) is a second commit - NOT deployed.

## NOT verified
SSH to CT110 was not possible from this session (PVE `109.69.172.199:209` -> "No route to host"; `root@192.168.1.60` -> publickey denied).
So it is **unproven** that the snapshot equals what runs now (the live relay was restarted ~14 days ago, after 24.09.).
To check on the PVE host:
```
pct exec 110 -- sha256sum /root/fvrelay/relay.js /root/fvrelay/account.js /root/fvrelay/setups.js /root/fvrelay/buch.js
git show cc41c25:relay/relay.js | sha256sum      # snapshot relay.js (7c348523f7e2...)
```
If they differ, `pct pull 110 /root/fvrelay/<file> .` and commit the real files (diff = the drift).

## Never commit (live-only, in `.gitignore`)
`hosts.json`, `upload.token`, `setups.json` (contains passwords of open setup links), `accounts/`, `dist/` (builds, feeds), `*.log`.
The old July copy `/root/fvrelay.moved-to-ct110/` (254 lines) is obsolete.
