#!/usr/bin/env bash
# Block 1 / step 50 - offsite copy (needs a target from the boss).
# The WD backup disk hangs on the same pve host: fire, theft, power surge or a
# host failure takes disk AND backups. This installs offsite-sync.sh on pve and
# a cron at 05:10 (after the 02:30 / 03:40 jobs).
#
# Usage: 50-offsite.sh --offsite user@host:/dir [--key /root/.ssh/id_offsite] [--vmids "110 111"] [--encrypt-passfile /root/.offsite-pass] [--apply]
# Prepare once on pve:  ssh-keygen -t ed25519 -N '' -f /root/.ssh/id_offsite
#                       and put id_offsite.pub on the target, restricted (command="rsync --server ...",no-pty).
# Without --offsite the script stops (there is no default target).
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,11p' "$0"; }
fv_parse_args "$@"
banner "backups: offsite copy"
off=$(fv_opt offsite "")
key=$(fv_opt key "/root/.ssh/id_offsite")
vmids=$(fv_opt vmids "110 111")
passfile=$(fv_opt encrypt-passfile "")
[ -n "$off" ] || die "no target. Ask the boss for one (other site / storage box), then rerun with --offsite user@host:/dir"
re_target='^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+:/[A-Za-z0-9._/-]*$'
[[ "$off" =~ $re_target ]] || die "--offsite must look like user@host:/dir"
re_ids='^[0-9 ]+$'
[[ "$vmids" =~ $re_ids ]] || die "bad --vmids"
host="${off%%:*}"

step "checks"
rx pve "command -v rsync >/dev/null && command -v openssl >/dev/null && echo yes" | grep -q yes && ok "rsync + openssl on pve" || bad "rsync/openssl missing on pve (apt install rsync)"
rx pve "test -r $key && echo yes" | grep -q yes && ok "key $key exists" || bad "key $key missing on pve - create it first (see header)"
if [ -n "$passfile" ]; then
  rx pve "test -r $passfile && test \"\$(stat -c %a $passfile)\" = 600 && echo yes" | grep -q yes && ok "passfile ok (mode 600)" || bad "passfile missing or not mode 600"
else
  warn "no --encrypt-passfile: archives contain upload token + passwords; the target volume MUST be encrypted"
fi
rx pve "ssh -i $key -o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=accept-new $host true && echo yes" | grep -q yes && ok "pve can reach $host with the key" || bad "pve cannot log in to $host with $key"
gate

step "install on pve"
put pve "$ROLLOUT_ROOT/01-backups/files/offsite-sync.sh" /tmp/fv-offsite-sync.sh
env_line="OFFSITE='$off' SSH_KEY='$key' VMIDS='$vmids'"
[ -n "$passfile" ] && env_line="$env_line OFFSITE_PASSFILE='$passfile'"
mx pve "install script + cron" "install -m 0750 /tmp/fv-offsite-sync.sh /usr/local/sbin/fv-offsite-sync.sh && rm -f /tmp/fv-offsite-sync.sh && printf '%s\n' '# FreeViewer rollout: offsite copy of the newest CT archives' '10 5 * * * root env $env_line /usr/local/sbin/fv-offsite-sync.sh >>/var/log/fv-offsite-sync.log 2>&1' >/etc/cron.d/fv-offsite-sync && chmod 644 /etc/cron.d/fv-offsite-sync"
mx pve "run once now" "env $env_line /usr/local/sbin/fv-offsite-sync.sh"
finish
