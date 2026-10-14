#!/usr/bin/env bash
# Block 4 / step 40 - put a freshly built SFU binary live (restart ends all meetings).
# The binary must already exist on CT111 (built from the merged branch), passed with --binary.
# Old binary is kept as freemeet-sfu.bak_swap; if the service is not healthy the old one returns.
# Usage: 40-swap-binary.sh --binary /opt/freemeet/target/release/freemeet-sfu.new [--apply] [--force]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,5p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: swap binary"
bin=$(fv_opt binary "")
re_bin='^/opt/freemeet/[A-Za-z0-9._/-]+$'
[[ "$bin" =~ $re_bin ]] || die "--binary /opt/freemeet/... required"
rx fm "test -x $bin && echo yes" | grep -q yes && ok "binary exists" || bad "binary not found on CT111"
rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -q '^pre-nonce-v2$\|^fv-rollout-' && ok "snapshot exists" || bad "no snapshot - run 03/30-backup.sh or 01/20-snapshots.sh"
c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l")
if [ "$c" = 0 ]; then ok "no connections"; elif fv_flag force; then warn "$c connections, --force"; else bad "$c connections - meetings would end"; fi
gate
confirm "Restart FreeMeet with the new binary? Running meetings end."
mx fm "swap + restart + health, auto-undo" "set -e
cd $FM_DIR
cp -p target/release/freemeet-sfu freemeet-sfu.bak_swap
cp -p $bin target/release/freemeet-sfu.swap && mv -f target/release/freemeet-sfu.swap target/release/freemeet-sfu
systemctl restart $FM_UNIT
sleep 3
if ! systemctl is-active --quiet $FM_UNIT || ! curl -fsS -m5 -o /dev/null http://127.0.0.1:7200/e2e.js; then
  echo 'NOT HEALTHY - old binary back'
  cp -p freemeet-sfu.bak_swap target/release/freemeet-sfu.swap && mv -f target/release/freemeet-sfu.swap target/release/freemeet-sfu
  systemctl restart $FM_UNIT; exit 1
fi
echo healthy"
finish
