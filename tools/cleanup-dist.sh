#!/bin/bash
# r15: archive old, publicly downloadable files from the relay's dist directory.
# Run ON the relay host (CT110). DRY RUN by default - prints what would move. Nothing is ever deleted:
# with --apply files are MOVED to a sibling directory that the relay does not serve.
#
#   bash cleanup-dist.sh [--apply] [--dist /root/fvrelay/dist] [--keep NAME]...
#
# Kept: version.json, version-xoffi.json, and every file they reference (file, mac_url basename),
# plus anything given via --keep (e.g. --keep freeviewer-0293.exe for a rollback build).
set -euo pipefail
DIST=/root/fvrelay/dist
APPLY=0
KEEP=()
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --dist) DIST="$2"; shift ;;
    --keep) KEEP+=("$2"); shift ;;
    -h|--help) sed -n 2,12p "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
[ -d "$DIST" ] || { echo "no such dir: $DIST" >&2; exit 1; }
DIST=$(cd "$DIST" && pwd)
ARCHIVE="$(dirname "$DIST")/dist-archive-$(date +%Y%m%d)"

keep_file() { printf '%s\n' "$1"; }
declare -A KEEPSET
for f in version.json version-xoffi.json "${KEEP[@]}"; do KEEPSET["$f"]=1; done
for j in version.json version-xoffi.json; do
  [ -f "$DIST/$j" ] || continue
  # referenced files: "file" and the basename of "mac_url"
  while read -r name; do [ -n "$name" ] && KEEPSET["$name"]=1; done < <(
    node -e '
      const j = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
      if (j.file) console.log(j.file);
      if (j.mac_url) console.log(String(j.mac_url).split("/").pop());
    ' "$DIST/$j"
  )
done
# the update feed must exist, otherwise refuse (an empty keep list would archive the live builds)
[ -f "$DIST/version.json" ] || { echo "version.json missing - refusing to continue" >&2; exit 1; }

n=0; bytes=0
for path in "$DIST"/*; do
  [ -f "$path" ] || continue
  name=$(basename "$path")
  [ -n "${KEEPSET[$name]:-}" ] && continue
  size=$(stat -c %s "$path")
  n=$((n+1)); bytes=$((bytes+size))
  if [ "$APPLY" = 1 ]; then
    mkdir -p "$ARCHIVE"
    mv -n -- "$path" "$ARCHIVE/"
    echo "moved   $name ($size B)"
  else
    echo "would move  $name ($size B)"
  fi
done
echo "--- kept: ${!KEEPSET[*]}"
if [ "$APPLY" = 1 ]; then echo "$n files ($bytes B) moved to $ARCHIVE"; else echo "$n files ($bytes B) would move to $ARCHIVE  (dry run; use --apply)"; fi
