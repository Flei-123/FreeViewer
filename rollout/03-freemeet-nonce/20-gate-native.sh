#!/usr/bin/env bash
# Block 3 / step 20 - GATE: the native clients must be out BEFORE the browser switch.
#
# Why this order (v1 and v2 cannot talk to each other, on purpose):
#   * Browser = the e2e.js inside the SFU binary: switches for EVERYBODY at the next page reload
#     after the SFU restart. You control this moment.
#   * Native (Windows/Mac) = only via auto-update; you do NOT control when each machine updates.
#   Native first  -> short mixed window you control: new natives only see new natives; old browser
#                    tabs and old natives get dropped (natives show "outdated end-to-end format").
#   Browser first -> long mixed window you do NOT control: every old native is cut off from every
#                    browser until the person happens to update. That is why it is the wrong order.
# Normal (non-E2E) rooms are not affected at any point.
#
# The gate checks the feed (win version >= --native-min-version) and asks you to confirm that the
# E2E users have updated (there is no telemetry for client versions).
# Usage: 20-gate-native.sh --native-min-version 0.30.0 [--apply]   (the gate needs no --apply)
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,20p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: native gate"
minv=$(fv_opt native-min-version "")
[[ "$minv" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--native-min-version X.Y.Z required (the release that contains src/meete2e.rs v2; Cargo.toml on main is still $(grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | cut -d'"' -f2) - bump it before the release)"

step "feed"
cur=$(rx relay "python3 -c \"import json;print(json.load(open('$RELAY_DIR/dist/version.json'))['version'])\"")
if [ "$(printf '%s\n%s\n' "$cur" "$minv" | sort -V | head -n1)" = "$minv" ]; then ok "Windows feed version $cur >= $minv"; else bad "Windows feed is at $cur, needs $minv - publish the native release first (02-relay/50-publish-release.sh)"; fi
mver=$(rx relay "python3 -c \"import json;d=json.load(open('$RELAY_DIR/dist/version.json'));print(d.get('mac_version',''))\"")
note "Mac package version in the feed: ${mver:-none} (Mac clients compare with 'version' but fetch mac_url, see roadmap r28) - make sure the Mac zip is the build with v2"
gate

step "adoption (human check)"
echo "Ask / check: do the people who use E2E rooms run version >= $minv (Windows AND Mac)?"
echo "Suggestion: wait at least 3 days after the release (auto-update installs on next restart of the app/service)."
if ((ASSUME_YES)); then
  note "--yes given: adoption assumed confirmed"
else
  read -r -p 'All E2E users updated? type "yes": ' a
  [ "$a" = yes ] || die "not confirmed - do not switch the browser yet"
fi
ok "gate passed - continue with 30-backup.sh"
finish
