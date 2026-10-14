# FreeViewer rollout package (14.10.2026) - NOTHING in here has been run live

Every script is **dry-run by default** (reads only). `--apply` executes. All use `set -euo pipefail`.
Run from a machine that has ssh keys for pve (192.168.1.1), CT110 (.60), CT111 (.61)
(`FV_SSH`/`FV_SCP` can point to a wrapper). Signing needs `/root/fvwork/keys/update-ed25519.key` (or `--sig`).
Test of the scripts against fakes: `bash rollout/tests/run-dryrun-tests.sh`.

## Order, time, downtime
| # | Block | Time | Downtime | Depends on |
|---|-------|------|----------|-----------|
| 1 | `01-backups` | 30 min + restore test 20 min | none | - |
| 2A | `02-relay` compat (30-apply) | 15 min | relay restart ~3 s, sessions drop | block 1 snapshots |
| 2B | first signed release (50-publish) | 20 min | none | 2A + a new client build (version bump!) |
| 3 | `03-freemeet-nonce` | gate: days; switch 20 min | **FreeMeet restart ~1 min, all meetings end** | native release out (2B), block 1 |
| 4 | `04-sfu-live-merge` | after W19 | same restart as block 3 (combine) | block 3 gate |
| 2C | relay strict + require-sig + phase-3 client | later | relay restart | publishers moved (mac-ci-publish.md) |

## Who clicks what (Boss)
1. Say yes. 2. Give an **offsite target** (`user@host:/dir`) for step 1/50. 3. Build + release the native client with the
nonce fix (bump `Cargo.toml` version, e.g. 0.30.0) - then 2B publishes it. 4. Pick the maintenance window and send
`03-freemeet-nonce/NOTICE-wartungsfenster.txt`. 5. Run `03.../CHECKLIST.md` with real browsers.
Everything else is `script --apply`.

## Steps
**Block 1** (`01-backups`): `10-precheck` -> `20-snapshots --apply` -> `30-vzdump-job --apply` -> `40-app-backups --apply`
-> `50-offsite --offsite ... --apply` (needs target) -> `60-verify` -> `70-restore-test --apply` -> later `80-cleanup-snapshots`. Undo: `90-rollback`.
**Block 2**: `10-precheck` -> `20-backup --apply` -> `30-apply --mode compat --apply` -> `40-verify --mode compat`
-> `50-publish-release --platform win --file ... --version ... --apply` (first signed release; re-signs Mac fields)
-> later: move publishers (`mac-ci-publish.md`), `30-apply --mode strict --only-dropin`, `60-require-sig`, then a client built
with `FV_REQUIRE_SIG=1` (phase 3), published with `50-publish-release`. Undo: `90-rollback` (`--feed VER`, `--lxc`).
**Block 3**: `10-precheck` -> `20-gate-native --native-min-version X` -> `30-backup --apply` -> `40-apply --apply` (file, build, restart)
-> `50-verify` -> `CHECKLIST.md`. Undo: `90-rollback` (`--file-only`, `--lxc`).
**Block 4**: see its README (combine the restart with block 3).

## Findings that change the old docs
- **Backups already exist** (nightly all-guest vzdump 02:30, 3 daily + 2 weekly, USB disk on the SAME pve). Missing: offsite, longer retention for CT110/111, app-level backup, restore test.
- **FreeMeet e2e.js is compiled into the binary** (`include_str!`): copying the file is not enough, a rebuild + restart is needed (docs/FREEMEET-E2E-NONCE.md said "no restart" - wrong).
- **LAN-only publish breaks the Mac CI upload** (public URL + `?token=`). Hence relay `compat` first, `strict` later.
- The client signs/verifies both platforms with the feed field `version` -> every Windows publish must re-sign `mac_sig` (done by `50-publish-release`).
- Live relay.js == repo snapshot (sha 7c348523...), live e2e.js == patch base (17a4227f...).

## Not proven
Nothing was applied. pvesh syntax checked with `pvesh usage` only; build on CT111 (`--locked`) unproven (offline metadata failed, internet works);
x-remote brand uses the same public key (assumed); public feed URL for the x-remote brand unverified.
