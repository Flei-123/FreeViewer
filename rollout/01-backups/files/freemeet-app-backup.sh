#!/usr/bin/env bash
# Application-level backup INSIDE CT111 (FreeMeet). Hourly from cron.d.
#  * meetings + config (/etc/freemeet/env holds the admin password -> 0600)
#  * a git bundle of /opt/freemeet (the only copy of the live source history,
#    the repo has no remote)
#  * the release binary, de-duplicated by hash, newest 3 kept (it exists
#    nowhere else; a rebuild needs the whole Rust toolchain)
set -euo pipefail

DATA_DIR="${FM_DATA_DIR:-/var/lib/freemeet}"
CONF="${FM_CONF:-/etc/freemeet}"
APP="${FM_APP:-/opt/freemeet}"
OUT="${FM_BACKUP_DIR:-/var/backups/freemeet}"
KEEP="${KEEP:-72}"

umask 077
mkdir -p "$OUT" "$OUT/bin"
stamp=$(date +%Y%m%d-%H%M)

paths=("${DATA_DIR#/}")
[ -d "$CONF" ] && paths+=("${CONF#/}")
tmp="$OUT/.freemeet-$stamp.tar.gz.part"
tar -czf "$tmp" -C / "${paths[@]}"
mv -f "$tmp" "$OUT/freemeet-$stamp.tar.gz"

if [ -d "$APP/.git" ]; then
  git -C "$APP" bundle create "$OUT/source-$stamp.bundle" --all >/dev/null 2>&1 || echo "WARN: git bundle failed" >&2
fi

bin="$APP/target/release/freemeet-sfu"
if [ -f "$bin" ]; then
  h=$(sha256sum "$bin" | cut -c1-12)
  [ -e "$OUT/bin/freemeet-sfu.$h" ] || cp -p "$bin" "$OUT/bin/freemeet-sfu.$h"
  ls -1t "$OUT"/bin/freemeet-sfu.* | tail -n +4 | xargs -r rm -f --
fi

ls -1t "$OUT"/freemeet-*.tar.gz | tail -n +"$((KEEP + 1))" | xargs -r rm -f --
ls -1t "$OUT"/source-*.bundle 2>/dev/null | tail -n +"$((KEEP + 1))" | xargs -r rm -f --
echo "backup ok: $OUT/freemeet-$stamp.tar.gz"
