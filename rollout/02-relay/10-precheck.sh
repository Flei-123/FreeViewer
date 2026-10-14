#!/usr/bin/env bash
# Block 2 / step 10 - READ-ONLY precheck for the relay patch (r2).
#  * the new relay.js in the repo = live snapshot + patch 0001 + patch 0002 (real client IP) and nothing else;
#    relay/clientip.js = what patch 0002 creates; its unit test passes
#  * live relay.js on CT110 is byte-identical to the snapshot (else STOP: drift)
#  * relay healthy, nobody connected, node syntax of the new file ok
#  * who still publishes via the public URL (this breaks in strict mode)
# Usage: 10-precheck.sh
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,8p' "$0"; }
fv_parse_args "$@"
banner "relay: precheck"
need_cmd node
need_cmd patch

step "local files"
[ -f "$RELAY_LOCAL_JS" ] || die "missing $RELAY_LOCAL_JS"
node --check "$RELAY_LOCAL_JS" && ok "node --check relay/relay.js" || bad "relay.js has a syntax error"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/relay"
if git -C "$REPO_ROOT" cat-file -e "$RELAY_BASE_COMMIT:relay/relay.js" 2>/dev/null; then
  git -C "$REPO_ROOT" show "$RELAY_BASE_COMMIT:relay/relay.js" >"$tmp/relay/relay.js"
  [ "$(sha_of "$tmp/relay/relay.js")" = "$RELAY_LIVE_SHA" ] && ok "snapshot $RELAY_BASE_COMMIT == live sha" || bad "snapshot differs from the recorded live sha"
  (cd "$tmp" && patch -s -p1 <"$RELAY_PATCH") && ok "patch 0001 applies to the snapshot" || bad "patch 0001 does not apply"
  if [ -f "$RELAY_PATCH2" ]; then
    (cd "$tmp" && patch -s -p1 <"$RELAY_PATCH2") && ok "patch 0002 (real client IP) applies on top" || bad "patch 0002 does not apply"
  else
    bad "missing $RELAY_PATCH2 (r63 not on this branch)"
  fi
  [ "$(sha_of "$tmp/relay/relay.js")" = "$(sha_of "$RELAY_LOCAL_JS")" ] && ok "repo relay.js == snapshot + patches 0001+0002 (nothing else mixed in)" || bad "repo relay.js has more changes than the patches"
  if [ -f "$tmp/relay/clientip.js" ] && [ -f "$RELAY_LOCAL_CLIENTIP" ]; then
    [ "$(sha_of "$tmp/relay/clientip.js")" = "$(sha_of "$RELAY_LOCAL_CLIENTIP")" ] && ok "repo clientip.js == what patch 0002 creates" || bad "repo clientip.js differs from patch 0002"
  else
    bad "clientip.js missing (patch or repo)"
  fi
else
  warn "commit $RELAY_BASE_COMMIT not in this clone - cannot prove 'snapshot + patch' (fetch full history)"
  bad "base commit missing"
fi
node --check "$RELAY_LOCAL_CLIENTIP" && ok "node --check relay/clientip.js" || bad "clientip.js has a syntax error"
if [ -f "$REPO_ROOT/relay/test_clientip.js" ]; then
  (cd "$REPO_ROOT/relay" && node test_clientip.js >/dev/null 2>&1) && ok "node relay/test_clientip.js passes" || bad "relay/test_clientip.js FAILED"
fi
pub=$(update_pubkey)
[[ "$pub" =~ ^[0-9a-f]{64}$ ]] && ok "update public key readable: ${pub:0:16}..." || bad "no update public key in src/update.rs"
if [ -r /root/fvwork/keys/update-ed25519.key ] && command -v python3 >/dev/null; then
  k=$(python3 "$REPO_ROOT/tools/sign-release/sign_release.py" pubkey 2>/dev/null || true)
  [ "$k" = "$pub" ] && ok "private key on this machine matches the compiled-in public key" || bad "signing key does not match src/update.rs"
else
  note "signing key not on this machine - signing (step 50) must happen where /root/fvwork/keys lives"
fi

step "live CT110"
live=$(rx relay "sha256sum $RELAY_DIR/relay.js | cut -d' ' -f1" || true)
[ "$live" = "$RELAY_LIVE_SHA" ] && ok "live relay.js == snapshot (no drift)" || bad "live relay.js sha is '$live' - the relay changed since 14.10.; diff first (pull it, compare with git)"
rx relay "test -e $RELAY_DIR/clientip.js && echo yes" | grep -q yes && note "clientip.js already on CT110 (re-run)" || ok "no clientip.js on CT110 yet (fresh)"
note "the reverse proxy in front of CT110 must send X-Forwarded-For (Nginx Proxy Manager does by default); without it hosts see the proxy address (r63 stays inactive, nothing breaks)"
rx relay "systemctl is-active $RELAY_UNIT" | grep -qx active && ok "$RELAY_UNIT active" || bad "$RELAY_UNIT not active"
h=$(relay_health)
grep -q '"ok":true' <<<"$h" && ok "health: $h" || bad "health check failed"
act=$(python3 -c 'import sys,json;d=json.loads(sys.stdin.read() or "{}");print(d.get("sessions_active","?"),d.get("hosts_online","?"))' <<<"$h" 2>/dev/null || echo "? ?")
note "sessions_active / hosts_online = $act (a restart drops running sessions; hosts reconnect on their own)"
rx relay "test \"\$(stat -c %a $RELAY_DIR/upload.token)\" = 600 && echo yes" | grep -q yes && ok "upload.token mode 600" || bad "upload.token mode is not 600"
rx relay "node --version" | grep -q '^v' && note "CT110 node: $(rx relay 'node --version')"
rx relay "ls $RELAY_DROPIN 2>/dev/null" | grep -q . && note "drop-in already exists (re-run / switch mode)" || note "no r2 drop-in yet (fresh)"

step "who publishes today? (these break when publishing becomes LAN-only/header-only)"
note "GitHub Actions mac.yml uploads X-Remote-macOS.zip to the PUBLIC url with ?token=  -> blocked in strict mode (see 02-relay/mac-ci-publish.md)"
note "the Windows publish script is not in the repo - find it BEFORE 'strict'. compat mode keeps it working."
rx relay "grep -c 'published ' $RELAY_DIR/relay.log" | sed 's/^/  [INFO] publishes ever logged: /' || true
rx relay "grep 'published ' $RELAY_DIR/relay.log | tail -n3" || true
finish
