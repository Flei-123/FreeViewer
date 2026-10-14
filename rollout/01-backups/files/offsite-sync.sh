#!/usr/bin/env bash
# Offsite copy of the newest vzdump archive per container. Runs on the PVE host
# from cron AFTER the nightly vzdump job. Never deletes anything on the source.
#
# Environment:
#   OFFSITE=user@host:/dir          target (required)
#   SSH_KEY=/root/.ssh/id_offsite   key without passphrase, restricted on the target
#   DUMPDIR=...                     default: dump dir of storage wd-backup
#   VMIDS="110 111"                 CT104 (JARVIS, ~54 GB/night) only if the link can take it
#   OFFSITE_PASSFILE=/root/.offsite-pass  if set: encrypt each archive (AES-256, pbkdf2)
#                                   before upload - the archives contain tokens/passwords
set -euo pipefail

: "${OFFSITE:?set OFFSITE=user@host:/dir}"
DUMPDIR="${DUMPDIR:-/mnt/wd4tb/proxmox-backup/dump}"
VMIDS="${VMIDS:-110 111}"
SSH_KEY="${SSH_KEY:-/root/.ssh/id_offsite}"
PASSFILE="${OFFSITE_PASSFILE:-}"
rsh="ssh -i $SSH_KEY -o BatchMode=yes -o StrictHostKeyChecking=yes"

fail=0
for id in $VMIDS; do
  newest=$(ls -1t "$DUMPDIR"/vzdump-lxc-"$id"-*.tar.zst 2>/dev/null | head -n1 || true)
  if [ -z "$newest" ]; then echo "ERROR: no archive for CT$id in $DUMPDIR" >&2; fail=1; continue; fi
  src="$newest"
  tmp=""
  if [ -n "$PASSFILE" ]; then
    tmp=$(mktemp -p /var/tmp "fvoffsite.XXXXXX")
    openssl enc -aes-256-cbc -pbkdf2 -salt -pass "file:$PASSFILE" -in "$newest" -out "$tmp"
    src="$tmp"
    dest="$OFFSITE/$(basename "$newest").enc"
  else
    dest="$OFFSITE/$(basename "$newest")"
  fi
  if rsync -a --partial --chmod=F600 -e "$rsh" "$src" "$dest"; then
    echo "OK: CT$id -> $(basename "$dest")"
  else
    echo "ERROR: rsync failed for CT$id" >&2; fail=1
  fi
  [ -n "$tmp" ] && rm -f "$tmp"
done
exit "$fail"
