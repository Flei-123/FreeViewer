#!/usr/bin/env bash
# Block 1 / step 30 - vzdump job for the small, critical containers.
#
# Situation found 14.10.2026: ONE job "all guests, 02:30, 3 daily + 2 weekly" on
# storage wd-backup. CT104 (JARVIS) alone is ~54 GB per archive, so the global
# retention must stay short. CT110 (relay, 0.7 GB) and CT111 (FreeMeet, 1.8 GB)
# are tiny and hold the data that is hardest to rebuild, so they get a SECOND
# job with 7 daily + 4 weekly + 3 monthly (~35 GB total) and are excluded from
# the global job (otherwise its shorter prune would delete the long history).
#
# Usage: 30-vzdump-job.sh [--apply] [--mailto you@example.com] [--time 03:40]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,14p' "$0"; }
fv_parse_args "$@"
banner "backups: vzdump job for CT110+CT111"
jobid="fv-ct110-111"
when=$(fv_opt time "03:40")
mailto=$(fv_opt mailto "")
[[ "$when" =~ ^[0-2][0-9]:[0-5][0-9]$ ]] || die "bad --time"

step "current jobs"
json=$(rx pve 'pvesh get /cluster/backup --output-format json')
oldid=$(python3 -c '
import sys, json
for j in json.load(sys.stdin):
    if j.get("all") and j.get("storage") == sys.argv[1]:
        print(j["id"]); break
' "$DUMP_STORAGE" <<<"$json" || true)
has_new=$(python3 -c '
import sys, json
print(any(j.get("id") == sys.argv[1] for j in json.load(sys.stdin)))
' "$jobid" <<<"$json")
[ -n "$oldid" ] && ok "global job found: $oldid" || bad "no global all-guests job on $DUMP_STORAGE found"
[ "$has_new" = "False" ] && ok "job $jobid does not exist yet" || note "job $jobid already exists - creating it will be skipped"
already_excl=$(python3 -c '
import sys, json
for j in json.load(sys.stdin):
    if j["id"] == sys.argv[1]: print(j.get("exclude", ""))
' "$oldid" <<<"$json" || true)
note "global job exclude list now: '${already_excl:-}'"
gate

step "create job $jobid (CT110, CT111; $when daily; keep 7d/4w/3m)"
mail_args="--mailnotification failure"
[ -n "$mailto" ] && mail_args="$mail_args --mailto $mailto"
if [ "$has_new" = "False" ]; then
  mx pve "create job $jobid" "pvesh create /cluster/backup --id $jobid --vmid 110,111 --storage $DUMP_STORAGE --schedule '$when' --mode snapshot --compress zstd --prune-backups 'keep-daily=7,keep-weekly=4,keep-monthly=3' --enabled 1 $mail_args --notes-template '{{guestname}} {{vmid}}'"
fi

step "exclude CT110+CT111 from the global job $oldid"
if grep -q '110' <<<"$already_excl" && grep -q '111' <<<"$already_excl"; then
  note "already excluded"
else
  mx pve "global job: exclude 110,111" "pvesh set /cluster/backup/$oldid --exclude 110,111"
fi

step "first run now (so there is no gap until tonight)"
mx pve "vzdump CT110 + CT111 once" "vzdump 110 111 --storage $DUMP_STORAGE --mode snapshot --compress zstd --notes-template '{{guestname}} {{vmid}} manual-first-run'"

if ((APPLY)); then
  step "verify"
  rx pve 'pvesh get /cluster/backup --output-format json' | grep -q "$jobid" && ok "job $jobid present" || bad "job missing"
fi
note "Roll back with 90-rollback.sh (removes the job, restores the global job)."
finish
