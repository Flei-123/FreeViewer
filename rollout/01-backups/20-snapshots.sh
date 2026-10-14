#!/usr/bin/env bash
# Block 1 / step 20 - safety snapshots of CT110 (relay) and CT111 (FreeMeet)
# BEFORE any live change of blocks 2-4. Instant on LVM-thin; remove them after
# 3 days with 80-cleanup-snapshots.sh (snapshots grow with every write).
# Usage: 20-snapshots.sh [--apply] [--name NAME] [--cts "110 111"]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,6p' "$0"; }
fv_parse_args "$@"
banner "backups: pre-change snapshots"
name=$(fv_opt name "fv-rollout-$(date +%Y%m%d)")
cts=$(fv_opt cts "110 111")
[[ "$name" =~ ^[A-Za-z][A-Za-z0-9_-]{1,38}$ ]] || die "bad snapshot name '$name'"

step "checks"
pool=$(rx pve "lvs --noheadings -o data_percent pve/data 2>/dev/null | tr -d ' %' | cut -d. -f1" || true)
if [ -n "$pool" ] && [ "$pool" -lt 85 ]; then ok "thin pool ${pool}% used"; else bad "thin pool too full/unknown (${pool:-?}%)"; fi
for id in $cts; do
  if rx pve "pct listsnapshot $id" | awk '{print $2}' | grep -qx "$name"; then bad "CT$id already has snapshot $name"; else ok "CT$id: no snapshot named $name yet"; fi
done
gate

step "snapshots"
for id in $cts; do
  mx pve "snapshot CT$id as $name" "pct snapshot $id $name --description 'before FreeViewer rollout $(date +%F)'"
done
if ((APPLY)); then
  for id in $cts; do rx pve "pct listsnapshot $id" | grep -q " $name " && ok "CT$id snapshot $name exists" || bad "CT$id snapshot missing"; done
fi
finish
