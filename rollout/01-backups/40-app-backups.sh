#!/usr/bin/env bash
# Block 1 / step 40 - hourly application backups INSIDE the containers.
#   CT110: /usr/local/bin/fvrelay-app-backup.sh    -> /var/backups/fvrelay
#   CT111: /usr/local/bin/freemeet-app-backup.sh   -> /var/backups/freemeet
# Adds only files + a cron.d entry; no service is restarted, nothing existing is touched.
# Usage: 40-app-backups.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,7p' "$0"; }
fv_parse_args "$@"
banner "backups: app-level backups in CT110/CT111"
F="$ROLLOUT_ROOT/01-backups/files"
bash -n "$F/relay-app-backup.sh" && bash -n "$F/freemeet-app-backup.sh" && ok "local syntax check of both scripts" || bad "syntax error in files/*.sh"

step "checks"
rx relay "test -d $RELAY_DIR && echo yes" | grep -q yes && ok "CT110 $RELAY_DIR exists" || bad "CT110: $RELAY_DIR missing"
rx fm "test -d /var/lib/freemeet && test -d $FM_DIR/.git && echo yes" | grep -q yes && ok "CT111 data dir + git exist" || bad "CT111: data dir or git missing"
rx relay "df -BM --output=avail /var | tail -n1 | tr -dc 0-9" | awk '{exit !($1>500)}' && ok "CT110 >500 MB free" || bad "CT110 low disk"
rx fm "df -BM --output=avail /var | tail -n1 | tr -dc 0-9" | awk '{exit !($1>2000)}' && ok "CT111 >2 GB free" || bad "CT111 low disk"
gate

step "install on CT110"
put relay "$F/relay-app-backup.sh" /tmp/fvrelay-app-backup.sh
mx relay "install script + cron" "install -m 0755 /tmp/fvrelay-app-backup.sh /usr/local/bin/fvrelay-app-backup.sh && rm -f /tmp/fvrelay-app-backup.sh && printf '%s\n' '# FreeViewer rollout: hourly relay data backup' '17 * * * * root /usr/local/bin/fvrelay-app-backup.sh >>/var/log/fvrelay-app-backup.log 2>&1' >/etc/cron.d/fvrelay-app-backup && chmod 644 /etc/cron.d/fvrelay-app-backup && /usr/local/bin/fvrelay-app-backup.sh"

step "install on CT111"
put fm "$F/freemeet-app-backup.sh" /tmp/freemeet-app-backup.sh
mx fm "install script + cron" "install -m 0755 /tmp/freemeet-app-backup.sh /usr/local/bin/freemeet-app-backup.sh && rm -f /tmp/freemeet-app-backup.sh && printf '%s\n' '# FreeViewer rollout: hourly FreeMeet backup (data, config, git bundle, binary)' '7 * * * * root /usr/local/bin/freemeet-app-backup.sh >>/var/log/freemeet-app-backup.log 2>&1' >/etc/cron.d/freemeet-app-backup && chmod 644 /etc/cron.d/freemeet-app-backup && /usr/local/bin/freemeet-app-backup.sh"

if ((APPLY)); then
  step "verify"
  rx relay "ls -l /var/backups/fvrelay | tail -n2" && ok "CT110 first archive written"
  rx fm "ls -l /var/backups/freemeet /var/backups/freemeet/bin | tail -n6" && ok "CT111 first archive written"
fi
finish
