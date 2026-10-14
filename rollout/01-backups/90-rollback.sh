#!/usr/bin/env bash
# Block 1 / step 90 - ROLLBACK of what blocks 1 added (backups are only added,
# so this is "switch the new things off again"; existing archives stay).
#   * job fv-ct110-111 removed, global job loses its exclude again
#   * cron.d entries + scripts removed (CT110, CT111, pve offsite)
#   * the data in /var/backups/* stays (delete by hand if wanted)
# Usage: 90-rollback.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,8p' "$0"; }
fv_parse_args "$@"
banner "backups: rollback"
confirm "This switches off the extra backup jobs/crons. Archives are kept."

step "vzdump jobs"
json=$(rx pve 'pvesh get /cluster/backup --output-format json')
oldid=$(python3 -c '
import sys, json
for j in json.load(sys.stdin):
    if j.get("all") and j.get("exclude"): print(j["id"]); break' <<<"$json" || true)
if grep -q 'fv-ct110-111' <<<"$json"; then
  mx pve "delete job fv-ct110-111" "pvesh delete /cluster/backup/fv-ct110-111"
else note "job fv-ct110-111 not present"; fi
if [ -n "$oldid" ]; then
  mx pve "global job $oldid: remove exclude" "pvesh set /cluster/backup/$oldid --delete exclude"
else note "no global job with an exclude list"; fi

step "cron + scripts"
mx relay "remove relay app backup" "rm -f /etc/cron.d/fvrelay-app-backup /usr/local/bin/fvrelay-app-backup.sh"
mx fm "remove FreeMeet app backup" "rm -f /etc/cron.d/freemeet-app-backup /usr/local/bin/freemeet-app-backup.sh"
mx pve "remove offsite cron" "rm -f /etc/cron.d/fv-offsite-sync /usr/local/sbin/fv-offsite-sync.sh"

step "restore test container (if left behind)"
mx pve "destroy CT 9111 only if it is the test CT" "if pct config 9111 2>/dev/null | grep -q fv-restore-test; then pct stop 9111 || true; pct destroy 9111 --purge; fi"
note "snapshots are removed with 80-cleanup-snapshots.sh (not here - they are your undo for blocks 2-4)"
finish
