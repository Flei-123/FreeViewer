#!/usr/bin/env bash
# Block 1 / step 80 - remove the pre-change snapshots (fv-rollout-*) once the
# rollout is confirmed good (suggested: >= 3 days later). Only snapshots whose
# name starts with "fv-rollout-" are touched.
# Usage: 80-cleanup-snapshots.sh [--apply] [--cts "110 111"]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,6p' "$0"; }
fv_parse_args "$@"
banner "backups: remove pre-change snapshots"
cts=$(fv_opt cts "110 111")
for id in $cts; do
  names=$(rx pve "pct listsnapshot $id | awk '{print \$2}' | grep '^fv-rollout-' || true")
  if [ -z "$names" ]; then note "CT$id: no fv-rollout-* snapshots"; continue; fi
  for n in $names; do
    mx pve "delete snapshot $n of CT$id" "pct delsnapshot $id $n"
  done
done
finish
