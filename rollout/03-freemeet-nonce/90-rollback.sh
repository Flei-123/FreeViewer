#!/usr/bin/env bash
# Block 3 / step 90 - ROLLBACK of the E2E v2 switch on CT111.
#   default     : put e2e.js (v1) back, put the OLD BINARY back, restart, check that v1 is served.
#   --file-only : only the e2e.js file + git commit (use after a failed BUILD; the old binary
#                 never stopped running, no restart needed).
#   --lxc       : full `pct rollback 111 pre-nonce-v2` (hard: state since the snapshot is lost).
#
# Native side: clients never move backwards. To undo the native release publish the previous build
# again under a HIGHER version number (02-relay/50-publish-release.sh). Never run v1 and v2 in one
# E2E room: they drop each other's frames.
# Usage: 90-rollback.sh [--apply] [--file-only | --lxc]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,12p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: rollback"

step "checks"
if fv_flag lxc; then
  rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -qx pre-nonce-v2 && ok "snapshot exists" || bad "snapshot pre-nonce-v2 missing"
else
  rx fm "test -e $FM_DIR/web/e2e.js.bak_nonce_v1 && echo yes" | grep -q yes && ok "e2e.js.bak_nonce_v1 exists" || bad "e2e.js.bak_nonce_v1 missing"
  fv_flag file-only || { rx fm "test -e $FM_DIR/freemeet-sfu.bak_nonce_v1 && echo yes" | grep -q yes && ok "binary backup exists" || bad "freemeet-sfu.bak_nonce_v1 missing"; }
fi
gate
confirm "Rollback ends running meetings (restart)."

if fv_flag lxc; then
  mx pve "pct rollback 111 pre-nonce-v2" "pct rollback 111 pre-nonce-v2 && pct start 111"
else
  mx fm "restore e2e.js v1 + git" "cd $FM_DIR && cp -p web/e2e.js.bak_nonce_v1 web/e2e.js && git add web/e2e.js && { git diff --cached --quiet || git commit -q -m 'Revert FreeMeet E2E v2 (rollback)'; } && sha256sum web/e2e.js"
  if ! fv_flag file-only; then
    mx fm "old binary back + restart + health" "set -e
cd $FM_DIR
cp -p freemeet-sfu.bak_nonce_v1 target/release/freemeet-sfu.restore
mv -f target/release/freemeet-sfu.restore target/release/freemeet-sfu
systemctl restart $FM_UNIT
sleep 3
systemctl is-active --quiet $FM_UNIT
curl -fsS -m5 -o /dev/null http://127.0.0.1:7200/e2e.js
echo 'service healthy (v1)'"
  fi
fi
if ((APPLY)) && ! fv_flag file-only; then
  step "verify"
  got=$(rx fm "curl -s -m5 http://127.0.0.1:7200/e2e.js | sha256sum | cut -d' ' -f1")
  [ "$got" = "$FM_E2E_LIVE_SHA" ] && ok "SFU serves v1 again" || bad "served sha is ${got:0:16}... (expected v1 ${FM_E2E_LIVE_SHA:0:16}...)"
fi
finish
