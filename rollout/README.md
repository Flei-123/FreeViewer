# FreeViewer rollout package (14.10.2026) - NOTHING in here has been run live

Every script is **dry-run by default** (reads only). `--apply` executes. All use `set -euo pipefail`.
Run from a machine that has ssh keys for pve (192.168.1.1), CT110 (.60), CT111 (.61)
(`FV_SSH`/`FV_SCP` can point to a wrapper). Signing needs `/root/fvwork/keys/update-ed25519.key` (or `--sig`).
Test of the scripts against fakes: `bash rollout/tests/run-dryrun-tests.sh`.

## Order, time, downtime
| # | Block | Time | Downtime | Depends on |
|---|-------|------|----------|-----------|
| 1 | `01-backups` | 30 min + restore test 20 min | none | - |
| 2A | `02-relay` compat (30-apply: relay.js r2/r63/r28/r6/r7 + clientip.js + reqauth.js) | 15 min | relay restart ~3 s, sessions drop | block 1 snapshots |
| 2B | first signed release (50-publish) | 20 min | none | 2A + a new client build (version bump!) |
| 3 | `03-freemeet-nonce` | gate: days; switch 20 min | **FreeMeet restart ~1 min, all meetings end** | native release out (2B), block 1 |
| 4 | `04-sfu-live-merge` (W19 package, ready) | merge+build 20-45 min (no downtime) | **the same single restart as block 3** (`40-swap-binary`, first start `FM_WORKERS=1`) | block 3 gate + stage `file` |
| 2C | relay strict + require-sig + phase-3 client | later | relay restart | publishers moved (mac-ci-publish.md) |

## Who clicks what (Boss)
1. Say yes. 2. Give an **offsite target** (`user@host:/dir`) for step 1/50. 3. Build + release the native client with the
nonce fix (bump `Cargo.toml` version, e.g. 0.30.0) - then 2B publishes it. 4. Pick the maintenance window and send
`03-freemeet-nonce/NOTICE-wartungsfenster.txt`. 5. Run `03.../CHECKLIST.md` with real browsers.
Everything else is `script --apply`.

## Steps
**Block 1** (`01-backups`): `10-precheck` -> `20-snapshots --apply` -> `30-vzdump-job --apply` -> `40-app-backups --apply`
-> `50-offsite --offsite ... --apply` (needs target) -> `60-verify` -> `70-restore-test --apply` -> later `80-cleanup-snapshots`. Undo: `90-rollback`.
**Block 2**: `10-precheck` -> `20-backup --apply` -> `30-apply --mode compat --apply` -> `40-verify --mode compat` (also proves clientip.js + reqauth.js are live, relay.js sends `client_ip`, anonymous `/fv/online` shows no names)
-> `50-publish-release --platform win --file ... --version ... --apply` (first signed release; Mac fields stay as they are)
-> Mac: `55-mac-pull.sh --brand freeviewer|x-remote [--apply]` (CI artifact -> checked -> signed -> published, `mac-ci-publish.md`)
-> later: move publishers (`mac-ci-publish.md`), `30-apply --mode strict --only-dropin`, `60-require-sig`, then a client built
with `FV_REQUIRE_SIG=1` (phase 3), published with `50-publish-release`. Undo: `90-rollback` (`--feed VER`, `--lxc`).
**Block 3**: `10-precheck` -> `20-gate-native --native-min-version X` -> `30-backup --apply` -> `40-apply --apply` (file, build, restart)
-> `50-verify` -> `CHECKLIST.md`. Undo: `90-rollback` (`--file-only`, `--lxc`).
**Block 4** (combined with block 3, ONE restart): `03/40-apply --stage file` -> `04/10-precheck` -> `20-merge --apply` -> `30-build --apply` -> maintenance window -> `40-swap-binary --binary /opt/freemeet/freemeet-sfu.w19 --apply` -> `50-verify` + `03/50-verify`. Undo: `04/90-rollback`. Details: `04-sfu-live-merge/README.md`.

## Findings that change the old docs
- **Backups already exist** (nightly all-guest vzdump 02:30, 3 daily + 2 weekly, USB disk on the SAME pve). Missing: offsite, longer retention for CT110/111, app-level backup, restore test.
- **FreeMeet e2e.js is compiled into the binary** (`include_str!`): copying the file is not enough, a rebuild + restart is needed (docs/FREEMEET-E2E-NONCE.md said "no restart" - wrong).
- **LAN-only publish broke the Mac CI upload** (public URL + `?token=`): solved - `mac.yml` only builds + attests, `55-mac-pull.sh` publishes over the LAN (`mac-ci-publish.md`). Still: relay `compat` first, `strict` later.
- The Mac client verifies `mac_sig` over the feed field `mac_version` (not `version`) -> Windows and Mac publishes are independent (r28, relay patch 0003 + `update::feed_version`). Needs the new client build (version bump) and relay patch 0003.
- Live relay.js == repo snapshot (sha 7c348523...), live e2e.js == patch base (17a4227f...).

## Not proven
Nothing was applied. pvesh syntax checked with `pvesh usage` only; build on CT111 (`--locked`) unproven (offline metadata failed, internet works);
x-remote brand uses the same public key (assumed); public feed URL for the x-remote brand unverified.
- **Relay patch 0004 (r6 + r7, `relay/reqauth.js`)**: the JARVIS login token for `/fv/account/*` and `/fv/setup/*` now comes as `Authorization: Bearer` (or `X-Fv-Token`). `?token=` still works for installed clients but the answer carries `Deprecation: true`, a hit is counted in `/fv/health` (`account_query_token_hits`) and logged once per IP and 10 min (never with the token). Once the counter stays 0 after the new client is out: env `FV_ACCOUNT_QUERY_TOKEN=0` refuses it. The new client sends the header first and falls back to `?token=` only after a 401 (old relay), so client and relay can be rolled out in any order.
- **Relay patch 0005 (r32 + r30, `relay/test_einladung.js`)**: new mailbox for meeting invitations between friends (`/fv/freunde/einladen`, `/einladungen`, `/einladung-weg`; stored in `freunde.json` as `einladungen`, 24 h, only https links without key fragment, only to friends incl. address-book friends) and the setup download now carries the brand's file name (`X-Remote-Einrichtung.exe` instead of always `FreeViewer-Einrichtung.exe`). `relay.js` only (no new file on CT110). The client uses the mailbox when it exists and falls back to the clipboard link on an old relay, so client and relay can be rolled out in any order.
- **`/fv/online` (r6)**: rate limited (`FV_ONLINE_RATE`, default 120/min per real client IP, 429 + Retry-After). Device name + last-seen only for a signed-in caller and only for IDs in that account's address book; everybody else gets `{online}`. Visible effect: installed clients WITHOUT account no longer get names/"last seen" from the relay for partners without a local name (they show the ID). Transitional escape hatch: `FV_ONLINE_PUBLIC_DETAILS=1` restores the old output (keeps r6 open).
