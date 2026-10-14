#!/usr/bin/env bash
# Block 3 / step 50 - automatic checks after the browser switch (read-only).
# The real proof is the manual CHECKLIST.md (2 browsers + 1 native in one E2E room).
# Usage: 50-verify.sh
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,4p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: verify"
NEWJS="$REPO_ROOT/web-patch/e2e.js"
want=$(sha_of "$NEWJS")

step "served file"
inner=$(rx fm "curl -s -m5 http://127.0.0.1:7200/e2e.js | sha256sum | cut -d' ' -f1")
[ "$inner" = "$want" ] && ok "SFU serves the v2 e2e.js (sha ${want:0:16}...)" || bad "SFU still serves another e2e.js (${inner:0:16}...) - not rebuilt/restarted?"
pub=$(curl -s -m10 https://meet.fleitec.com/e2e.js | sha256sum | cut -d' ' -f1 || true)
[ "$pub" = "$want" ] && ok "public https://meet.fleitec.com/e2e.js is v2" || bad "public file differs (${pub:0:16}...) - proxy cache?"
hdr=$(curl -sI -m10 https://meet.fleitec.com/e2e.js | tr -d '\r' | grep -i '^cache-control' || true)
grep -qi 'no-cache' <<<"$hdr" && ok "cache-control: no-cache (browsers fetch the new file on reload)" || bad "cache-control header: '${hdr}' - users may keep v1 in the cache"
for f in index.html app.js; do
  code=$(curl -s -o /dev/null -m10 -w '%{http_code}' "https://meet.fleitec.com/$f" || true)
  [ "$code" = 200 ] || [ "$code" = 304 ] && ok "$f -> $code" || bad "$f -> $code"
done

step "service"
rx fm "systemctl is-active $FM_UNIT" | grep -qx active && ok "$FM_UNIT active" || bad "$FM_UNIT not active"
rx fm "ss -lun | grep -c ':7201'" | grep -qx 1 && ok "UDP 7201 listening" || bad "UDP 7201 not listening"
rx fm "journalctl -u $FM_UNIT --since '10 min ago' --no-pager | grep -ci 'panic\\|error' || true" | awk '{ if ($1>0) print "  [WARN] " $1 " panic/error lines in the last 10 min (check journalctl -u freemeet)"; else print "  [ OK ] no panic/error in the log"}'
rx fm "git -C $FM_DIR log --oneline -1"
note "now run CHECKLIST.md with real browsers + one native client"
finish
