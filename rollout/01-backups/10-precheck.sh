#!/usr/bin/env bash
# Block 1 / step 10 - READ-ONLY inventory of the backup situation.
# Finds out what already exists before anything is added (r22 text said "no
# backups", but on 14.10.2026 a nightly all-guest vzdump job was found).
# Usage: 10-precheck.sh [--dry-run]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,6p' "$0"; }
fv_parse_args "$@"
banner "backups: precheck"

step "pve reachable + storage"
rx pve 'pveversion' >/dev/null && ok "pve ssh works ($PVE_HOST)" || bad "cannot ssh to pve $PVE_HOST:$PVE_PORT"
st=$(rx pve "pvesm status" || true)
if grep -Eq "^$DUMP_STORAGE +dir +active" <<<"$st"; then ok "storage $DUMP_STORAGE active"; else bad "storage $DUMP_STORAGE not active"; fi

step "existing vzdump jobs"
rx pve 'pvesh get /cluster/backup --output-format json' | python3 -c '
import sys, json
for j in json.load(sys.stdin):
    print("  job %s: vmid=%s all=%s schedule=%s storage=%s prune=%s exclude=%s enabled=%s" % (
        j.get("id"), j.get("vmid", "-"), j.get("all", 0), j.get("schedule"), j.get("storage"),
        j.get("prune-backups"), j.get("exclude", "-"), j.get("enabled")))
' || bad "cannot read /cluster/backup"

step "newest archive per container (must be < 36 h old)"
dumpdir=$(rx pve "awk '/^dir: $DUMP_STORAGE/{f=1} f&&/path/{print \$2; exit}' /etc/pve/storage.cfg")/dump
note "dump dir: $dumpdir"
for id in 104 110 111; do
  line=$(rx pve "ls -1t $dumpdir/vzdump-lxc-$id-*.tar.zst 2>/dev/null | head -n1" || true)
  if [ -z "$line" ]; then bad "CT$id: no archive in $dumpdir"; continue; fi
  age=$(rx pve "echo \$(( (\$(date +%s) - \$(stat -c %Y $line)) / 3600 ))")
  size=$(rx pve "du -h $line | cut -f1")
  if [ "$age" -lt 36 ]; then ok "CT$id: $(basename "$line") ($size, ${age} h old)"; else bad "CT$id: newest archive is ${age} h old"; fi
done

step "free space on the backup disk"
avail=$(rx pve "df -BG --output=avail $dumpdir | tail -n1 | tr -dc 0-9")
if [ "${avail:-0}" -ge 150 ]; then ok "free: ${avail} GB"; else bad "only ${avail} GB free (CT104 archive alone is ~54 GB)"; fi

step "snapshots + thin pool"
for id in 110 111; do note "CT$id snapshots: $(rx pve "pct listsnapshot $id | grep -v current | awk '{print \$2}' | tr '\n' ' '" || true)"; done
pool=$(rx pve "lvs --noheadings -o data_percent pve/data 2>/dev/null | tr -d ' %' | cut -d. -f1" || true)
if [ -n "$pool" ] && [ "$pool" -lt 85 ]; then ok "thin pool ${pool}% used"; else bad "thin pool usage unknown/high (${pool:-?}%)"; fi

step "known gaps (information, not failures)"
note "backup disk is a USB disk mounted on the SAME pve host -> a host/power failure can take disk and data together"
note "no offsite copy  -> 50-offsite.sh (needs a target from the boss)"
note "retention is 3 daily + 2 weekly for ALL guests -> 30-vzdump-job.sh adds 7/4/3 for CT110+CT111"
note "no application-level backup (fast restore of relay hosts.json, FreeMeet binary)"
note "no restore test ever documented -> 70-restore-test.sh"
finish
