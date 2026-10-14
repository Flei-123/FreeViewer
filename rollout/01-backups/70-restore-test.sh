#!/usr/bin/env bash
# Block 1 / step 70 - restore test: a backup that was never restored is a hope.
# Restores the newest archive of CT110 or CT111 into a NEW throw-away container
# (id 9111) WITHOUT network, checks its content, destroys it again.
# The live container is not touched. The test CT never gets a network card, so
# it cannot clash with the live IP.
#
# Usage: 70-restore-test.sh [--ct 111|110] [--apply] [--keep]
#   --keep  leave CT 9111 in place (then destroy by hand: pct destroy 9111 --purge)
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,11p' "$0"; }
fv_parse_args "$@"
banner "backups: restore test"
ct=$(fv_opt ct 111)
[[ "$ct" =~ ^(110|111)$ ]] || die "--ct must be 110 or 111"
tid=9111
dumpdir=$(rx pve "awk '/^dir: $DUMP_STORAGE/{f=1} f&&/path/{print \$2; exit}' /etc/pve/storage.cfg")/dump
archive=$(rx pve "ls -1t $dumpdir/vzdump-lxc-$ct-*.tar.zst | head -n1")

step "checks"
[ -n "$archive" ] && ok "archive: $(basename "$archive")" || bad "no archive for CT$ct"
if rx pve "pct status $tid 2>&1" | grep -q status; then bad "CT $tid already exists - refuse to touch it"; else ok "CT $tid is free"; fi
avail=$(rx pve "pvesh get /nodes/pve/storage/local-lvm/status --output-format json | python3 -c 'import sys,json;print(int(json.load(sys.stdin)[\"avail\"]/2**30))'" || echo 0)
[ "${avail:-0}" -ge 30 ] && ok "local-lvm: ${avail} GB free" || bad "local-lvm: only ${avail} GB free"
gate

step "restore (no network, not started automatically)"
mx pve "restore $ct -> $tid" "pct restore $tid '$archive' --storage local-lvm --hostname fv-restore-test && pct set $tid --delete net0 --description fv-restore-test --onboot 0 && pct start $tid"

if ((APPLY)); then
  step "check content"
  sleep 5
  if [ "$ct" = 111 ]; then
    rx pve "pct exec $tid -- sh -c 'test -x /opt/freemeet/target/release/freemeet-sfu && python3 -c \"import json;json.load(open(\\\"/var/lib/freemeet/meetings.json\\\"));print(\\\"json ok\\\")\" && cd /opt/freemeet && git log --oneline | wc -l'" && ok "CT111 content readable (binary, meetings.json, git history)" || bad "CT111 content check failed"
  else
    rx pve "pct exec $tid -- sh -c 'cd /root/fvrelay && sha256sum relay.js && python3 -c \"import json;print(len(json.load(open(\\\"hosts.json\\\"))),\\\"hosts\\\")\"'" && ok "CT110 content readable (relay.js, hosts.json)" || bad "CT110 content check failed"
  fi
fi

step "destroy the test container"
if fv_flag keep; then
  note "--keep given: CT $tid stays. Destroy with: pct stop $tid; pct destroy $tid --purge"
else
  mx pve "destroy CT $tid (only if it is the test CT)" "test \"\$(pct config $tid | grep -c fv-restore-test)\" -ge 1 && pct stop $tid; test \"\$(pct config $tid | grep -c fv-restore-test)\" -ge 1 && pct destroy $tid --purge"
fi
finish
