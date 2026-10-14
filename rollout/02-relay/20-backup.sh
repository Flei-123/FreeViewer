#!/usr/bin/env bash
# Block 2 / step 20 - backup before touching the relay.
#   1. relay.js.bak_r2 next to the live file (never overwritten on re-runs)
#   2. data copy backup-r2-<stamp>/ (hosts.json, feeds, upload.token, drop-ins)
#   3. LXC snapshot "pre-publish-hardening" (full undo incl. node/app state)
# Usage: 20-backup.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,7p' "$0"; }
fv_parse_args "$@"
banner "relay: backup"
ts=$(stamp)

step "checks"
live=$(rx relay "sha256sum $RELAY_DIR/relay.js | cut -d' ' -f1" || true)
[ "$live" = "$RELAY_LIVE_SHA" ] && ok "live relay.js is the known snapshot" || bad "live relay.js unknown ($live) - run 10-precheck.sh"
if rx pve "pct listsnapshot 110" | awk '{print $2}' | grep -qx pre-publish-hardening; then note "snapshot pre-publish-hardening exists already"; fi
gate

step "backup"
mx relay "relay.js.bak_r2 (only if not present)" "cd $RELAY_DIR && { [ -e relay.js.bak_r2 ] && echo 'relay.js.bak_r2 exists - kept' || cp -p relay.js relay.js.bak_r2; } && mkdir -p backup-r2-$ts && cp -a hosts.json devices.json freunde.json buch-geraete.json upload.token dist/version.json dist/version-xoffi.json backup-r2-$ts/ 2>/dev/null; chmod 700 backup-r2-$ts; ls -l relay.js.bak_r2"
if ! rx pve "pct listsnapshot 110" | awk '{print $2}' | grep -qx pre-publish-hardening; then
  mx pve "snapshot CT110 as pre-publish-hardening" "pct snapshot 110 pre-publish-hardening --description 'before relay publish hardening (r2)'"
fi
if ((APPLY)); then
  rx relay "sha256sum $RELAY_DIR/relay.js.bak_r2" | grep -q "$RELAY_LIVE_SHA" && ok "relay.js.bak_r2 == live snapshot" || bad "backup does not match"
  rx pve "pct listsnapshot 110" | awk '{print $2}' | grep -qx pre-publish-hardening && ok "LXC snapshot exists" || bad "LXC snapshot missing"
fi
finish
