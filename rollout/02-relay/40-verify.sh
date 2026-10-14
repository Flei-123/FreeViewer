#!/usr/bin/env bash
# Block 2 / step 40 - smoke tests of the relay (curl, run on CT110 over ssh;
# the token is read on CT110 and NEVER printed). None of the requests can
# publish anything: they are all refused before the feed is written. The script
# proves that by comparing the feed hash before and after.
#
# Usage: 40-verify.sh [--mode compat|strict] [--require-sig]
#   --mode        must match what 30-apply.sh set (default compat)
#   --require-sig also expect "unsigned upload refused" (after 60-require-sig.sh)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,10p' "$0"; }
fv_parse_args "$@"
banner "relay: verify (read-only requests)"
mode=$(fv_opt mode compat)
[[ "$mode" =~ ^(compat|strict)$ ]] || die "--mode must be compat or strict"

# expect DESCRIPTION EXPECTED_CODE REMOTE_COMMAND   (command prints the http code)
expect() {
  local d="$1" want="$2" cmd="$3" got
  got=$(rx relay "$cmd" 2>/dev/null || true)
  if [ "$got" = "$want" ]; then ok "$d -> $got"; else bad "$d -> got '$got', expected $want"; fi
}
CURL='curl -s -o /dev/null -m10 -w %{http_code} -X POST'
BASE='http://127.0.0.1:7180/fv/publish'
TOKCMD='TOK=$(cat /root/fvrelay/upload.token)'

step "state"
feed_before=$(rx relay "sha256sum $RELAY_DIR/dist/version.json | cut -d' ' -f1")
rx relay "systemctl is-active $RELAY_UNIT" | grep -qx active && ok "service active" || bad "service not active"
rx relay "systemctl show $RELAY_UNIT -p Environment" | grep -q FV_UPDATE_PUBKEY && ok "FV_UPDATE_PUBKEY set" || bad "FV_UPDATE_PUBKEY missing -> signature check inactive"
env=$(rx relay "systemctl show $RELAY_UNIT -p Environment")
if [ "$mode" = strict ]; then
  grep -q 'FV_PUBLISH_LAN_ONLY=0' <<<"$env" && bad "LAN_ONLY is switched off but mode strict expected" || ok "LAN-only active"
  grep -q 'FV_PUBLISH_QUERY_TOKEN=1' <<<"$env" && bad "query token still allowed" || ok "query token refused"
fi
step "files (r2 + r63)"
if [ -f "$RELAY_LOCAL_JS" ]; then
  [ "$(rx relay "sha256sum $RELAY_DIR/relay.js | cut -d' ' -f1" || true)" = "$(sha_of "$RELAY_LOCAL_JS")" ] && ok "live relay.js == repo relay.js" || bad "live relay.js differs from the repo file"
fi
if [ -f "$RELAY_LOCAL_CLIENTIP" ]; then
  [ "$(rx relay "sha256sum $RELAY_DIR/clientip.js | cut -d' ' -f1" || true)" = "$(sha_of "$RELAY_LOCAL_CLIENTIP")" ] && ok "live clientip.js == repo clientip.js" || bad "live clientip.js missing or different"
fi
if [ -f "$RELAY_LOCAL_REQAUTH" ]; then
  [ "$(rx relay "sha256sum $RELAY_DIR/reqauth.js | cut -d' ' -f1" || true)" = "$(sha_of "$RELAY_LOCAL_REQAUTH")" ] && ok "live reqauth.js == repo reqauth.js" || bad "live reqauth.js missing or different"
fi
rx relay "grep -c client_ip $RELAY_DIR/relay.js || true" | grep -qv '^0$' && ok "relay.js sends client_ip to hosts" || bad "relay.js has no client_ip"
rx relay "tail -n 40 $RELAY_DIR/relay.log | grep -ci 'Cannot find module' || true" | grep -qx 0 && ok "no 'Cannot find module' in the log" || bad "module error in the relay log"
expect "health" 200 "curl -s -o /dev/null -m5 -w %{http_code} http://127.0.0.1:7180/fv/health"
expect "feed readable" 200 "curl -s -o /dev/null -m5 -w %{http_code} http://127.0.0.1:7180/fv/version"
rx relay "tail -n 5 $RELAY_DIR/relay.log | grep -ci 'error\\|exception' || true" | awk '{ if ($1>0) print "  [WARN] error words in the last log lines"; }'

step "r6 + r7: /fv/online and the login token (read-only requests, no token is used)"
ONE_ID=$(rx relay "node -e \"const d=JSON.parse(require('fs').readFileSync('$RELAY_DIR/devices.json','utf8'));const k=Object.keys(d).filter(x=>d[x]&&d[x].name)[0]||'';console.log(k)\"" 2>/dev/null || true)
if [[ "$ONE_ID" =~ ^[0-9]{9,10}$ ]]; then
  n=$(rx relay "curl -s -m5 'http://127.0.0.1:7180/fv/online?ids=$ONE_ID' | grep -c '\"name\"' || true")
  [ "$n" = 0 ] && ok "anonymous /fv/online shows no device name" || bad "anonymous /fv/online still leaks a name (r6)"
  n=$(curl -s -m10 "https://freeviewer.fleitec.com/fv/online?ids=$ONE_ID" | grep -c '"name"' || true)
  [ "$n" = 0 ] && ok "public /fv/online shows no device name" || bad "public /fv/online still leaks a name (r6)"
else
  note "no device with a name in devices.json - name check skipped"
fi
expect "/fv/online answers" 200 "curl -s -o /dev/null -m5 -w %{http_code} 'http://127.0.0.1:7180/fv/online?ids=111111111'"
expect "bogus Authorization token is refused" 401 "curl -s -o /dev/null -m10 -w %{http_code} -H 'Authorization: Bearer bogus-token-0123456789' http://127.0.0.1:7180/fv/account/me"
rx relay "curl -s -m5 http://127.0.0.1:7180/fv/health" | grep -q account_query_token_hits && ok "health shows account_query_token_hits (deprecated ?token= users; 0 = safe to set FV_ACCOUNT_QUERY_TOKEN=0)" || bad "health lacks account_query_token_hits - old relay.js?"

step "publish endpoint (all must be refused, nothing may be written)"
expect "no token" 403 "$CURL '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
expect "bad token (bearer)" 403 "$CURL -H 'Authorization: Bearer wrong' '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
expect "valid token, malformed signature" 400 "$TOKCMD; $CURL -H \"Authorization: Bearer \$TOK\" -H 'X-Fv-Sig: zz' '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
expect "valid token, wrong signature" 400 "$TOKCMD; $CURL -H \"Authorization: Bearer \$TOK\" -H 'X-Fv-Sig: $(printf '0%.0s' $(seq 128))' '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
if [ "$mode" = strict ]; then
  expect "valid token + X-Forwarded-For (= from the internet)" 403 "$TOKCMD; $CURL -H \"Authorization: Bearer \$TOK\" -H 'X-Forwarded-For: 203.0.113.9' '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
  expect "valid token in the URL (?token=)" 403 "$TOKCMD; $CURL \"$BASE?token=\$TOK&version=9.9.9&name=smoketest.exe\" --data-binary x"
fi
if fv_flag require-sig; then
  expect "valid token, NO signature (require_sig)" 400 "$TOKCMD; $CURL -H \"Authorization: Bearer \$TOK\" '$BASE?version=9.9.9&name=smoketest.exe' --data-binary x"
fi

step "from the internet (this machine, public URL)"
code=$(curl -s -o /dev/null -m10 -w '%{http_code}' -X POST 'https://freeviewer.fleitec.com/fv/publish?version=9.9.9&name=smoketest.exe' --data-binary x || true)
[ "$code" = 403 ] && ok "public publish without token -> 403" || bad "public publish -> $code (expected 403)"
code=$(curl -s -o /dev/null -m10 -w '%{http_code}' 'https://freeviewer.fleitec.com/fv/version' || true)
[ "$code" = 200 ] && ok "public feed -> 200" || bad "public feed -> $code"

step "nothing was published"
feed_after=$(rx relay "sha256sum $RELAY_DIR/dist/version.json | cut -d' ' -f1")
[ "$feed_before" = "$feed_after" ] && ok "version.json unchanged" || bad "version.json CHANGED during the smoke test!"
rx relay "ls $RELAY_DIR/dist | grep -c smoketest || true" | grep -qx 0 && ok "no smoketest file in dist/" || bad "smoketest file found in dist/ - delete it"
finish
