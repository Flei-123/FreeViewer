#!/usr/bin/env bash
# Block 1 / step 60 - READ-ONLY proof that the backups are real and readable.
#  * newest archive of CT104/110/111 younger than 36 h and its zstd stream intact
#    (zstd -t on 110 and 111; CT104 only with --deep, it is ~54 GB)
#  * jobs present, app-level backups fresh, offsite log/cron if installed
# Usage: 60-verify.sh [--deep]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,7p' "$0"; }
fv_parse_args "$@"
banner "backups: verify"

dumpdir=$(rx pve "awk '/^dir: $DUMP_STORAGE/{f=1} f&&/path/{print \$2; exit}' /etc/pve/storage.cfg")/dump
step "archives"
for id in 110 111 104; do
  f=$(rx pve "ls -1t $dumpdir/vzdump-lxc-$id-*.tar.zst 2>/dev/null | head -n1" || true)
  [ -n "$f" ] || { bad "CT$id: no archive"; continue; }
  age=$(rx pve "echo \$(( (\$(date +%s) - \$(stat -c %Y $f)) / 3600 ))")
  [ "$age" -lt 36 ] && ok "CT$id: $(basename "$f") is ${age} h old" || bad "CT$id: archive ${age} h old"
  if [ "$id" != 104 ] || fv_flag deep; then
    rx pve "zstd -tq $f && echo intact" | grep -q intact && ok "CT$id: zstd stream intact" || bad "CT$id: zstd test FAILED"
  else
    note "CT104: zstd test skipped (use --deep)"
  fi
done

step "jobs"
rx pve 'pvesh get /cluster/backup --output-format json' | python3 -c '
import sys, json
ids = [j["id"] for j in json.load(sys.stdin)]
print("  jobs:", ", ".join(ids))
sys.exit(0 if "fv-ct110-111" in ids else 1)' && ok "job fv-ct110-111 present" || bad "job fv-ct110-111 missing (30-vzdump-job.sh)"

step "application backups"
rx relay "find /var/backups/fvrelay -name 'fvrelay-*.tar.gz' -mmin -90 2>/dev/null | head -n1" | grep -q . && ok "CT110: archive < 90 min old" || bad "CT110: no fresh app backup (40-app-backups.sh)"
rx fm "find /var/backups/freemeet -name 'freemeet-*.tar.gz' -mmin -90 2>/dev/null | head -n1" | grep -q . && ok "CT111: archive < 90 min old" || bad "CT111: no fresh app backup"
rx fm "ls /var/backups/freemeet/bin 2>/dev/null | head -n1" | grep -q . && ok "CT111: release binary stored" || bad "CT111: binary not stored"
rx fm "ls /var/backups/freemeet/source-*.bundle 2>/dev/null | tail -n1" | grep -q . && ok "CT111: git bundle stored" || bad "CT111: git bundle missing"

step "offsite (optional)"
if rx pve "test -f /etc/cron.d/fv-offsite-sync && echo yes" | grep -q yes; then
  rx pve "tail -n3 /var/log/fv-offsite-sync.log 2>/dev/null" || true
  rx pve "tail -n3 /var/log/fv-offsite-sync.log 2>/dev/null | grep -c '^OK'" | awk '{exit !($1>=1)}' && ok "offsite log shows OK lines" || bad "offsite log has no OK line"
else
  note "offsite not installed - the single largest remaining risk (see README)"
fi
finish
