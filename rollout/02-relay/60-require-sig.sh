#!/usr/bin/env bash
# Block 2 / step 60 - signature phase switch on the RELAY side (REQUIRE_SIGNATURE).
#
#   Phase 1 : client verifies signatures when present, accepts unsigned feeds (compiled default).
#   Phase 2 : relay + feed always signed. This script switches the relay to
#             FV_REQUIRE_SIG=1 = the relay refuses every unsigned upload.
#   Phase 3 : clients built with FV_REQUIRE_SIG=1 refuse unsigned feeds. That is a CLIENT
#             build (see README "Phase 3 client build"); this script only checks that the
#             feed is ready for it.
#
# Do this only when
#   * the first signed release (50-publish-release.sh) is live and its signature verifies,
#   * the Mac fields are signed too (or you accept that Mac stays unsigned),
#   * most installations already run the phase-1 client (an old client simply never updates
#     to a build that it cannot verify... but it ALSO never gets the phase-1 client if the
#     feed becomes unsigned-hostile - phase 3 is only safe after the phase-1 rollout).
#
# Usage: 60-require-sig.sh [--apply] [--off]      (--off: back to phase 1 relay behaviour)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,19p' "$0"; }
fv_parse_args "$@"
banner "relay: require signatures"
need_cmd python3
SIGN="$REPO_ROOT/tools/sign-release/sign_release.py"
pub=$(update_pubkey)
want=1
fv_flag off && want=0

step "checks"
tmpf=$(mktemp)
trap 'rm -f "$tmpf"' EXIT
for feed in version.json version-xoffi.json; do
  rx relay "cat $RELAY_DIR/dist/$feed" >"$tmpf"
  if python3 - "$tmpf" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
sys.exit(0 if d.get("sig") else 1)
PYEOF
  then
    python3 -I "$SIGN" verify --feed "$tmpf" --pub "$pub" >/dev/null && ok "$feed: signature verifies" || bad "$feed: signature present but INVALID"
  else
    if [ "$want" = 1 ]; then bad "$feed: no signature yet - publish a signed release first (50-publish-release.sh)"; else note "$feed unsigned"; fi
  fi
done
env=$(rx relay "systemctl show $RELAY_UNIT -p Environment")
grep -q 'FV_LAN_ONLY=0\|FV_PUBLISH_LAN_ONLY=0' <<<"$env" && note "relay is still in COMPAT mode (public publish allowed) - consider 30-apply.sh --mode strict --only-dropin"
mode=compat
grep -q 'FV_PUBLISH_LAN_ONLY=0' <<<"$env" || mode=strict
gate

step "relay drop-in (mode stays $mode, FV_REQUIRE_SIG=$want)"
confirm "From now on the relay refuses unsigned uploads. Every publish needs a signature (50-publish-release.sh does that)."
relay_write_and_restart "$mode" "$want"
if ((APPLY)); then
  note "now run: 40-verify.sh --mode $mode $( [ "$want" = 1 ] && echo --require-sig )"
fi
note "Phase 3 client build is a separate release: FV_REQUIRE_SIG=1 cargo build --release, bump the version, publish with 50-publish-release.sh."
finish
