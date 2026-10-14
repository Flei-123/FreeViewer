#!/usr/bin/env bash
# Block 3 / step 30 - backup before touching CT111.
#   * e2e.js.bak_nonce_v1  (the live JS file)
#   * freemeet-sfu.bak_nonce_v1  (the live BINARY - it is the only copy of what runs now)
#   * git tag pre-nonce-v2 on the current HEAD of /opt/freemeet
#   * LXC snapshot (rollback of everything)
# Usage: 30-backup.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,8p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: backup"
snap="pre-nonce-v2"

step "checks"
rx fm "test -x $FM_DIR/target/release/freemeet-sfu && echo yes" | grep -q yes && ok "binary present" || bad "binary missing"
rx fm "sha256sum $FM_DIR/web/e2e.js | cut -d' ' -f1" | grep -qx "$FM_E2E_LIVE_SHA" && ok "live e2e.js is the known file" || bad "live e2e.js unknown - run 10-precheck.sh"
rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -qx "$snap" && note "snapshot $snap already exists" || ok "snapshot name $snap free"
gate

step "backup"
mx fm "copy e2e.js + binary, tag git (never overwrites an existing backup)" "cd $FM_DIR && { [ -e web/e2e.js.bak_nonce_v1 ] || cp -p web/e2e.js web/e2e.js.bak_nonce_v1; } && { [ -e freemeet-sfu.bak_nonce_v1 ] || cp -p target/release/freemeet-sfu freemeet-sfu.bak_nonce_v1; } && { git rev-parse -q --verify refs/tags/pre-nonce-v2 >/dev/null || git tag pre-nonce-v2; } && sha256sum web/e2e.js.bak_nonce_v1 freemeet-sfu.bak_nonce_v1 && { [ ! -x /usr/local/bin/freemeet-app-backup.sh ] || /usr/local/bin/freemeet-app-backup.sh; }"
if ! rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -qx "$snap"; then
  mx pve "snapshot CT111 as $snap" "pct snapshot 111 $snap --description 'before FreeMeet E2E nonce fix (r1)'"
fi
if ((APPLY)); then
  rx fm "test -e $FM_DIR/freemeet-sfu.bak_nonce_v1 && test -e $FM_DIR/web/e2e.js.bak_nonce_v1 && echo yes" | grep -q yes && ok "file backups present" || bad "file backups missing"
  rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -qx "$snap" && ok "LXC snapshot present" || bad "LXC snapshot missing"
fi
finish
