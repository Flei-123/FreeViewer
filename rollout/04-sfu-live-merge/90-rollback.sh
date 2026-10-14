#!/usr/bin/env bash
# Block 4 / step 90 - old binary back (freemeet-sfu.bak_swap, else freemeet-sfu.bak_nonce_v1), FM_WORKERS drop-in removed,
# git back to the branch it was on before 20-merge.sh (branch fv-w19 stays for inspection). ONE restart, meetings end.
# Note: with block 3 done, web/e2e.js on disk is v2 but an old binary embeds v1 - see 03-freemeet-nonce/90-rollback.sh.
# Usage: 90-rollback.sh [--apply] [--keep-git]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,5p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: rollback"
rx fm "ls $FM_DIR/freemeet-sfu.bak_swap $FM_DIR/freemeet-sfu.bak_nonce_v1 2>/dev/null | head -n1" | grep -q . && ok "a backup binary exists" || bad "no backup binary on CT111"
gate
confirm "Roll back the SFU binary (meetings end)?"
keepgit=0; fv_flag keep-git && keepgit=1
mx fm "restore binary, remove drop-in, restart, switch git back" "set -e
cd $FM_DIR
b=freemeet-sfu.bak_swap; [ -e \$b ] || b=freemeet-sfu.bak_nonce_v1
cp -p \$b target/release/freemeet-sfu.restore && mv -f target/release/freemeet-sfu.restore target/release/freemeet-sfu
rm -f $SFU_DROPIN $SFU_DROPIN.prev
systemctl daemon-reload
systemctl restart $FM_UNIT; sleep 3; systemctl is-active --quiet $FM_UNIT && echo healthy
if [ $keepgit = 0 ] && [ -r $SFU_REMOTE_DIR/prev-branch ] && [ \"\$(git rev-parse --abbrev-ref HEAD)\" = $SFU_BRANCH ]; then git switch -q \$(cat $SFU_REMOTE_DIR/prev-branch) && echo \"git back on \$(git rev-parse --abbrev-ref HEAD)\"; fi"
finish
