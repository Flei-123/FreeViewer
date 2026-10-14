#!/usr/bin/env bash
# Block 4 / step 90 - old binary back (freemeet-sfu.bak_swap, else freemeet-sfu.bak_nonce_v1).
# Usage: 90-rollback.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,3p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: rollback"
rx fm "ls $FM_DIR/freemeet-sfu.bak_swap $FM_DIR/freemeet-sfu.bak_nonce_v1 2>/dev/null | head -n1" | grep -q . && ok "a backup binary exists" || bad "no backup binary on CT111"
gate
confirm "Roll back the SFU binary (meetings end)?"
mx fm "restore binary + restart" "set -e
cd $FM_DIR
b=freemeet-sfu.bak_swap; [ -e \$b ] || b=freemeet-sfu.bak_nonce_v1
cp -p \$b target/release/freemeet-sfu.restore && mv -f target/release/freemeet-sfu.restore target/release/freemeet-sfu
systemctl restart $FM_UNIT; sleep 3; systemctl is-active --quiet $FM_UNIT && echo healthy"
finish
