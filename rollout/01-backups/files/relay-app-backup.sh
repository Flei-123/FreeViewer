#!/usr/bin/env bash
# Application-level backup INSIDE CT110 (relay). Hourly from cron.d.
# Small (data only, no executables/zips), mode 0600 because upload.token and
# setups.json (passwords of open setup links) are inside.
set -euo pipefail

SRC="${FV_RELAY_DIR:-/root/fvrelay}"
OUT="${FV_BACKUP_DIR:-/var/backups/fvrelay}"
KEEP="${KEEP:-72}"

umask 077
mkdir -p "$OUT"
stamp=$(date +%Y%m%d-%H%M)
tmp="$OUT/.fvrelay-$stamp.tar.gz.part"
cd "$SRC"
files=()
for f in hosts.json devices.json freunde.json buch-geraete.json setups.json upload.token \
  relay.js account.js setups.js buch.js package.json package-lock.json; do
  [ -e "$f" ] && files+=("$f")
done
[ -d accounts ] && files+=(accounts)
for f in dist/version*.json; do [ -e "$f" ] && files+=("$f"); done
tar -czf "$tmp" "${files[@]}"
mv -f "$tmp" "$OUT/fvrelay-$stamp.tar.gz"
# keep only the newest $KEEP archives
ls -1t "$OUT"/fvrelay-*.tar.gz | tail -n +"$((KEEP + 1))" | xargs -r rm -f --
echo "backup ok: $OUT/fvrelay-$stamp.tar.gz"
