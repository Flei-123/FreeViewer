#!/usr/bin/env bash
# Block 2 / step 30 - deploy the hardened relay.js (r2) to CT110.
#
#   --mode compat  (default, STEP A) publishers keep working exactly as today
#                  (public URL, ?token=); new: signatures sent in X-Fv-Sig are verified.
#   --mode strict  (STEP B, after every publisher was moved to the LAN + header token)
#                  publish only from the LAN, token only in Authorization/X-Upload-Token.
#   Switching A -> B later: rerun with --mode strict --only-dropin (no file copy, one restart).
#
# Files put on CT110: relay.js (r2 + r63) AND clientip.js (new, relay.js requires it from the same directory).
# clientip.js is moved into place BEFORE relay.js, both before the restart.
#
# Downtime: one service restart (~3 s). Running sessions drop, hosts reconnect by themselves.
# The script restarts only if sessions_active == 0 (override: --force) and undoes itself
# automatically if the relay is not healthy afterwards.
#
# Usage: 30-apply.sh [--apply] [--mode compat|strict] [--only-dropin] [--force]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,15p' "$0"; }
fv_parse_args "$@"
banner "relay: apply"
mode=$(fv_opt mode compat)
[[ "$mode" =~ ^(compat|strict)$ ]] || die "--mode must be compat or strict"

step "checks"
rx relay "test -e $RELAY_DIR/relay.js.bak_r2 && echo yes" | grep -q yes && ok "backup relay.js.bak_r2 exists" || bad "no relay.js.bak_r2 - run 20-backup.sh --apply first"
rx pve "pct listsnapshot 110" | awk '{print $2}' | grep -qx pre-publish-hardening && ok "LXC snapshot exists" || bad "no pre-publish-hardening snapshot"
h=$(relay_health)
grep -q '"ok":true' <<<"$h" && ok "relay healthy before: $h" || bad "relay not healthy before the change"
act=$(python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("sessions_active",1))' <<<"$h" 2>/dev/null || echo 1)
if [ "$act" = 0 ]; then ok "no active session"; elif fv_flag force; then warn "sessions_active=$act, --force given"; else bad "sessions_active=$act - wait for 0 or use --force"; fi
if ! fv_flag only-dropin; then
  node --check "$RELAY_LOCAL_JS" && ok "node --check (local)" || bad "local relay.js broken"
  [ -f "$RELAY_LOCAL_CLIENTIP" ] && node --check "$RELAY_LOCAL_CLIENTIP" && ok "node --check clientip.js (local)" || bad "local clientip.js missing or broken (patch 0002)"
  grep -q "require('./clientip')" "$RELAY_LOCAL_JS" && ok "relay.js requires ./clientip" || bad "relay.js does not require ./clientip - repo not at patch 0002?"
  live=$(rx relay "sha256sum $RELAY_DIR/relay.js | cut -d' ' -f1")
  new=$(sha_of "$RELAY_LOCAL_JS")
  if [ "$live" = "$RELAY_LIVE_SHA" ]; then ok "live relay.js is the known snapshot"; elif [ "$live" = "$new" ]; then note "live relay.js already is the new file"; else bad "live relay.js is unknown ($live)"; fi
fi
if [ "$mode" = strict ]; then
  note "STRICT: confirm that the Windows publish script and the Mac CI no longer use the public URL / ?token= (see mac-ci-publish.md)"
  confirm "Strict mode breaks every publisher that still uses the public URL or ?token=. Have they been moved?"
fi
gate

if ! fv_flag only-dropin; then
  step "copy + syntax-check the new file on CT110 (service untouched so far)"
  put relay "$RELAY_LOCAL_CLIENTIP" "$RELAY_DIR/clientip.js.new"
  put relay "$RELAY_LOCAL_JS" "$RELAY_DIR/relay.js.new"
  mx relay "node --check both, clientip.js first, then relay.js into place" "cd $RELAY_DIR && node --check clientip.js.new && node --check relay.js.new && mv -f clientip.js.new clientip.js && mv -f relay.js.new relay.js && node -e \"require('./clientip')\" && sha256sum relay.js clientip.js"
fi

step "drop-in + restart (mode $mode)"
relay_write_and_restart "$mode" 0

if ((APPLY)); then
  step "result"
  rx relay "tail -n 3 $RELAY_DIR/relay.log"
  [ "$mode" = strict ] || note "next: 40-verify.sh --mode compat, then move the publishers, then 30-apply.sh --mode strict --only-dropin"
fi
finish
