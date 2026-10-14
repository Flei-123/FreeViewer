# Hardening proposals (r13, r15) - NOTHING here is applied

Files:
- `freeviewer-relay.service` - systemd sandbox for the relay (CT110). Variant A keeps root, Variant B moves to an own user.
- `freemeet-sfu.service` - systemd sandbox for the SFU (CT111, `DynamicUser`). Check `systemctl cat` of the live unit first.
- `nginx-security-headers.conf` - HSTS + CSP (Report-Only first) + nosniff etc. for the proxy hosts.
- `../../tools/cleanup-dist.sh` - r15: archive old files from `/fv/dist` (dry run by default).

## Rollout order (needs Boss-Ja, live systems)
1. `systemd-analyze security <unit>` before/after; install the unit as a drop-in copy, `systemctl daemon-reload`, restart in a quiet minute, check `/fv/version` and a test session. Roll back by removing the unit file.
2. Relay as own user (Variant B): create user, move `/root/fvrelay` -> `/opt/freeviewer-relay` + `/var/lib/freeviewer-relay`, copy `hosts.json`, `dist/`, upload token (mode 0600). Do this together with the relay import (docs/RELAY-LIVE-IMPORT.md).
3. Headers: add `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`, HSTS `max-age=86400`; CSP as Report-Only for a week, then enforce. Raise HSTS to one year after a week.
4. `/fv/dist` cleanup: run `cleanup-dist.sh` on CT110 (dry run), review, then `--apply` (moves, never deletes).

## r15: files publicly reachable in `/fv/dist` (read-only check 2026-10-14)
Verified by `curl -r 0-99 https://freeviewer.fleitec.com/fv/dist/<name>` -> HTTP 200 for a sample of every group; the relay serves ANY file in the directory by name (`safeName` only blocks `/` and odd characters). Directory listing itself is not available, but names are guessable (`v0270.patch`, `fvfull.zip`, ...). 145 MB total, 55 files on CT110 `/root/fvrelay/dist`.

| Group | Files | Needed? |
|---|---|---|
| Live builds | `freeviewer.exe`, `x-remote.exe`, `FreeViewer-macOS.zip`, `X-Remote-macOS.zip` | yes (referenced by the feeds) |
| Feeds | `version.json`, `version-xoffi.json` | yes |
| Old Windows builds | `freeviewer-0292.exe`, `freeviewer-0293.exe`, `x-remote-0292.exe`, `x-remote-0293.exe` (~22 MB each) | rollback only - keep one pair at most (`--keep`) |
| Source patches / bundles / zips | `v0234..v0270*.patch`, `maczip.patch`, `fv-native*.bundle`, `fv022.zip`, `fvfix*.zip`, `fvfull.zip`, `fvmac*.zip`, `fvstub.zip` | no - leaks old source diffs; ARCHIVE |
| Proof screenshots | `beweis-0282.jpg`, `beweis-0292-*.png` | no - ARCHIVE |
| Feed backups | `version*.json.bak*` (11 files) | no - ARCHIVE |

Result of a dry run with default keep list: everything except the 6 "yes" files (and the 4 old exes unless `--keep`) would move. Nothing was moved.

## Also open (not part of this step)
- The relay should not serve files outside a whitelist at all (feed-referenced names + `*.exe|*.zip` of the current version); proposal for a later relay patch.
