#!/usr/bin/env bash
# Block 4 / step 50 - READ-ONLY checks after the restart: merged binary running, FM_WORKERS as set, no panics.
# Usage: 50-verify.sh [--workers 1|N]     (expected FM_WORKERS value, default 1)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,4p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: verify"
workers=$(fv_opt workers 1)
[[ "$workers" =~ ^[0-9]+$ ]] || die "--workers N"

step "service"
rx fm "systemctl is-active $FM_UNIT" | grep -qx active && ok "$FM_UNIT active" || bad "$FM_UNIT not active"
pid=$(rx fm "systemctl show $FM_UNIT -p MainPID --value" || true)
[[ "$pid" =~ ^[1-9][0-9]*$ ]] && ok "MainPID $pid" || bad "no MainPID"
if [[ "$pid" =~ ^[1-9][0-9]*$ ]]; then
  rx fm "tr '\\0' '\\n' < /proc/$pid/environ | grep '^FM_WORKERS=' || echo FM_WORKERS-unset" | grep -qx "FM_WORKERS=$workers" && ok "process runs with FM_WORKERS=$workers" || bad "process FM_WORKERS differs from $workers"
  run=$(rx fm "sha256sum /proc/$pid/exe | cut -d' ' -f1" || true)
  want=$(rx fm "sha256sum $SFU_NEW_BINARY | cut -d' ' -f1" || true)
  [ -n "$run" ] && [ "$run" = "$want" ] && ok "running binary == freemeet-sfu.w19 (${run:0:16}...)" || bad "running binary is not the w19 build"
fi
rx fm "ss -lun | grep -c ':7201'" | grep -qx 1 && ok "UDP 7201 listening" || bad "UDP 7201 not listening"
rx fm "curl -s -o /dev/null -m5 -w %{http_code} http://127.0.0.1:7200/e2e.js" | grep -qx 200 && ok "e2e.js served (200)" || bad "e2e.js not served"
rx fm "journalctl -u $FM_UNIT --since '10 min ago' --no-pager | grep -ci 'panic' || true" | awk '{ if ($1>0) print "  [FAIL] " $1 " panic lines (journalctl -u freemeet)"; else print "  [ OK ] no panic in the log"}'

step "git"
rx fm "git -C $FM_DIR rev-parse --abbrev-ref HEAD" | grep -qx "$SFU_BRANCH" && ok "CT111 on branch $SFU_BRANCH" || bad "CT111 is not on $SFU_BRANCH"
want=$(sfu_expected_hashes)
got=$(rx fm "git -C $FM_DIR rev-parse HEAD:freemeet-sfu HEAD:Cargo.toml HEAD:Cargo.lock | tr '\n' ' '" || true)
[ "$got" = "$want" ] && ok "sources == $SFU_MERGED_BRANCH" || bad "sources differ from $SFU_MERGED_BRANCH"

note "load test: a real meeting with 2+ browsers (03-freemeet-nonce/CHECKLIST.md). Raising FM_WORKERS only after a bench on an idle host (docs/PERFORMANCE.md)."
finish
