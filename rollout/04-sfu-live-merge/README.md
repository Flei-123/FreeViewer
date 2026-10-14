# Block 4 - SFU live merge (r65, W19 package)

Package: `/root/projects/freemeet-sfu/rollout/0001..0010-*.patch` (override with `SFU_REPO=`). Built from branch
`live-merged` = `live` (read-only import of CT111 `43d84ff`) + fv-w15/w19 work. `10-precheck.sh` proves locally that
the series applied to `live` gives exactly `live-merged`. Nothing here was run against CT111.
Details/measurements: `$SFU_REPO/docs/LIVE-MERGE.md`, `PERFORMANCE.md`.

## What changes for the users
Hardened meetings (CSPRNG ids, rate limit, 0600 files), gzip assets, bounded WS queue, UDP send threads + recv batching,
parallel media loop (rayon, `FM_WORKERS`). **First start runs with `FM_WORKERS=1`** (old single-thread loop): the parallel
loop doubled total CPU and had no proven gain at 16 participants. Decide about more workers only after a bench on an idle host.

## Combined run with block 3 (ONE restart, recommended)
The e2e.js is compiled into the binary, so the nonce fix and the SFU merge ship in the same build and the same restart.
```
03-freemeet-nonce/10-precheck.sh ; 20-gate-native.sh ... ; 30-backup.sh --apply        # backups + snapshot first
03-freemeet-nonce/40-apply.sh --stage file --apply                                      # new web/e2e.js (committed on CT111)
04-sfu-live-merge/10-precheck.sh                                                        # accepts HEAD = 43d84ff + e2e.js commit
04-sfu-live-merge/20-merge.sh --apply                                                   # branch fv-w19, git am -3, sources == live-merged
04-sfu-live-merge/30-build.sh --apply [--tests]                                         # separate target dir, live untouched
   ... maintenance window, NOTICE-wartungsfenster.txt sent ...
04-sfu-live-merge/40-swap-binary.sh --binary /opt/freemeet/freemeet-sfu.w19 --apply     # THE restart, FM_WORKERS=1, auto-undo
04-sfu-live-merge/50-verify.sh ; 03-freemeet-nonce/50-verify.sh ; CHECKLIST.md          # real browsers
```
Do NOT run `03/40-apply.sh --stage build|restart` in this mode (it would build the old tree / restart a second time).
Without block 3 the same steps work (skip the `--stage file` line; the binary keeps the old e2e.js).

## Undo
`90-rollback.sh --apply`: old binary (`bak_swap`, else `bak_nonce_v1`), drop-in removed, git back, one restart.
Deeper: LXC snapshot `pre-nonce-v2` (`pct rollback 111`, see 03 rollback). Failed 40-swap undoes itself.

## Not proven
Real build on CT111 (`--locked`, offline metadata failed earlier, internet works), real meeting with the real `web/`,
disk space for the second target dir (checked by the scripts, ~3 GB), behaviour of the new defaults under real browsers.
