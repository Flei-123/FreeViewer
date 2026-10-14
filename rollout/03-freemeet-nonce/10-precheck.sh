#!/usr/bin/env bash
# Block 3 / step 10 - READ-ONLY precheck for the FreeMeet E2E nonce fix (r1, format v2).
#
# IMPORTANT CORRECTION to docs/FREEMEET-E2E-NONCE.md ("static file, no restart"):
# on CT111 the browser files are compiled INTO the SFU binary
# (signal.rs: include_str!("../../web/e2e.js")). Copying e2e.js to /opt/freemeet/web/
# changes nothing until the SFU is rebuilt AND restarted. The restart ends all running
# meetings. This block therefore does: replace file -> cargo build -> restart.
#
# Checks: tests of the new e2e.js, live file == known sha, source tree clean and the binary
# is not older than the sources (a rebuild would ship OTHER pending changes too), build
# tooling present, who is connected right now, how many E2E rooms exist.
# Usage: 10-precheck.sh
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,13p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: precheck"
NEWJS="$REPO_ROOT/web-patch/e2e.js"

step "local: the new e2e.js"
[ -f "$NEWJS" ] || die "missing $NEWJS"
need_cmd node
node --check "$NEWJS" && ok "node --check e2e.js" || bad "syntax error"
(cd "$REPO_ROOT" && node --test web-patch/ >/tmp/fv-e2e-test.$$ 2>&1) && ok "node --test web-patch: $(grep -E '^# pass' /tmp/fv-e2e-test.$$ | head -n1)" || bad "web-patch tests FAILED (see output of: node --test web-patch/)"
rm -f /tmp/fv-e2e-test.$$
grep -q 'freemeet-e2e-v2' "$NEWJS" && ok "contains the v2 key-derivation label" || bad "v2 label not found in e2e.js"
note "new e2e.js sha256: $(sha_of "$NEWJS")"

step "live: CT111"
live=$(rx fm "sha256sum $FM_DIR/web/e2e.js | cut -d' ' -f1" || true)
[ "$live" = "$FM_E2E_LIVE_SHA" ] && ok "live web/e2e.js == the file the patch was built on" || { [ "$live" = "$(sha_of "$NEWJS")" ] && note "live e2e.js is ALREADY the new one" || bad "live e2e.js differs from the base of the patch ($live)"; }
served=$(rx fm "curl -s -m5 http://127.0.0.1:7200/e2e.js | sha256sum | cut -d' ' -f1" || true)
note "e2e.js currently SERVED: ${served:0:16}... (differs from the file on disk = binary is stale vs sources)"
[ "$served" = "$live" ] && ok "served == file on disk" || bad "served e2e.js != file on disk: the binary was built from different sources than the tree"
rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "git tree clean" || bad "git tree has uncommitted changes"
n=$(rx fm "cd $FM_DIR && find freemeet-sfu/src web Cargo.toml freemeet-sfu/Cargo.toml -type f -newer target/release/freemeet-sfu ! -name '*.bak*' ! -name '*.orig' | wc -l")
[ "$n" = 0 ] && ok "binary is newer than all sources (a rebuild ships only the e2e.js change)" || bad "$n source file(s) newer than the binary - a rebuild would ship more than e2e.js. List: find ... -newer target/release/freemeet-sfu"
rx fm "command -v cargo >/dev/null && cargo --version" && ok "cargo present" || bad "cargo missing on CT111"
rx fm "curl -s -m5 -o /dev/null -w %{http_code} https://index.crates.io/config.json" | grep -q 200 && ok "crates.io reachable (build can fetch crates)" || bad "no internet for cargo"
av=$(rx fm "df -BM --output=avail / | tail -n1 | tr -dc 0-9")
[ "$av" -ge 3000 ] && ok "${av} MB free" || bad "only ${av} MB free (build needs ~1 GB)"
rx fm "systemctl is-active $FM_UNIT" | grep -qx active && ok "$FM_UNIT active" || bad "$FM_UNIT not active"

step "who is in a meeting right now?"
c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l")
note "established TCP connections to the SFU (browsers via proxy + websocket signalling): $c"
rx fm "python3 -c \"import json;d=json.load(open('/var/lib/freemeet/meetings.json'));v=list(d.values()) if isinstance(d,dict) else d;print(len(v),'meetings in meetings.json;',sum(1 for m in v if isinstance(m,dict) and m.get('e2e')),'with E2E flag')\"" | sed 's/^/  [INFO] /'
note "FreeMeet has no admin API for live rooms in this checkout: ask the people / look at the SFU log before the restart"
rx fm "journalctl -u $FM_UNIT --since '30 min ago' --no-pager 2>/dev/null | grep -ci 'join' || true" | sed 's/^/  [INFO] join log lines in the last 30 min: /'
finish
