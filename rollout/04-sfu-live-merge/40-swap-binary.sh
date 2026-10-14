#!/usr/bin/env bash
# Block 4 / step 40 - THE ONE RESTART: put the freshly built SFU binary live (ends ALL meetings).
# Binary comes from 30-build.sh (/opt/freemeet/freemeet-sfu.w19). With block 3 this restart ALSO ships the
# v2 e2e.js (it is compiled into the binary) - so do NOT run 03-freemeet-nonce/40-apply.sh --stage build/restart.
# FIRST START WITH FM_WORKERS=1 (old single-thread loop): the parallel loop doubled total CPU without a
# proven gain (docs/PERFORMANCE.md in the SFU repo). A systemd drop-in sets it; change it later with --workers N.
# The old binary is kept as freemeet-sfu.bak_swap (never overwritten); if the service is not healthy,
# old binary + old drop-in come back automatically.
# Usage: 40-swap-binary.sh --binary /opt/freemeet/freemeet-sfu.w19 [--workers 1|N|keep] [--apply] [--force]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,10p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: swap binary + restart"
bin=$(fv_opt binary "")
workers=$(fv_opt workers 1)
re_bin='^/opt/freemeet/[A-Za-z0-9._/-]+$'
[[ "$bin" =~ $re_bin ]] || die "--binary /opt/freemeet/... required"
[[ "$workers" =~ ^([0-9]+|keep)$ ]] || die "--workers N or keep"

step "checks"
rx fm "test -x $bin && echo yes" | grep -q yes && ok "binary exists" || bad "binary not found on CT111"
rx fm "test -e $FM_DIR/freemeet-sfu.bak_swap && echo yes" | grep -q yes && bad "freemeet-sfu.bak_swap exists already (it is the pre-merge binary) - move it away on purpose first" || ok "bak_swap free"
if [ "$workers" != keep ]; then
  rx fm "grep -c FM_WORKERS $bin" | grep -qv '^0' && ok "binary knows FM_WORKERS" || bad "binary has no FM_WORKERS - not the merged build"
fi
rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -q '^pre-nonce-v2$\|^fv-rollout-' && ok "snapshot exists" || bad "no snapshot - run 03/30-backup.sh or 01/20-snapshots.sh"
c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l")
if [ "$c" = 0 ]; then ok "no connections"; elif fv_flag force; then warn "$c connections, --force"; else bad "$c connections - meetings would end"; fi
gate
confirm "Restart FreeMeet with the new binary (FM_WORKERS=$workers)? Running meetings end."

if [ "$workers" = keep ]; then
  dropin_cmd="true"
else
  dropin_cmd="mkdir -p $SFU_DROPIN_DIR && { [ -e $SFU_DROPIN ] && cp -p $SFU_DROPIN $SFU_DROPIN.prev || true; } && printf '[Service]\nEnvironment=FM_WORKERS=$workers\n' > $SFU_DROPIN && systemctl daemon-reload"
fi
mx fm "bak_swap, binary, drop-in FM_WORKERS=$workers, ONE restart, health, auto-undo" "set -e
cd $FM_DIR
cp -p target/release/freemeet-sfu freemeet-sfu.bak_swap
$dropin_cmd
cp -p $bin target/release/freemeet-sfu.swap && mv -f target/release/freemeet-sfu.swap target/release/freemeet-sfu
systemctl restart $FM_UNIT
sleep 3
if ! systemctl is-active --quiet $FM_UNIT || ! curl -fsS -m5 -o /dev/null http://127.0.0.1:7200/e2e.js; then
  echo 'NOT HEALTHY - old binary + old drop-in back'
  cp -p freemeet-sfu.bak_swap target/release/freemeet-sfu.swap && mv -f target/release/freemeet-sfu.swap target/release/freemeet-sfu
  if [ -e $SFU_DROPIN.prev ]; then mv -f $SFU_DROPIN.prev $SFU_DROPIN; else rm -f $SFU_DROPIN; fi
  systemctl daemon-reload; systemctl restart $FM_UNIT; exit 1
fi
rm -f $SFU_DROPIN.prev
echo healthy"
note "next: 50-verify.sh, then 03-freemeet-nonce/50-verify.sh + CHECKLIST.md (real browsers)"
finish
