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
expect "health" 200 "curl -s -o /dev/null -m5 -w %{http_code} http://127.0.0.1:7180/fv/health"
expect "feed readable" 200 "curl -s -o /dev/null -m5 -w %{http_code} http://127.0.0.1:7180/fv/version"
rx relay "tail -n 5 $RELAY_DIR/relay.log | grep -ci 'error\\|exception' || true" | awk '{ if ($1>0) print "  [WARN] error words in the last log lines"; }'

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
