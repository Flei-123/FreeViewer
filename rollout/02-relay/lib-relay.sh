# shellcheck shell=bash
# shellcheck disable=SC2034  # constants are used by the scripts that source this file
# Helpers shared by the relay scripts (block 2). Source after lib/common.sh.

RELAY_DROPIN_DIR="/etc/systemd/system/${RELAY_UNIT}.service.d"
RELAY_DROPIN="$RELAY_DROPIN_DIR/r2-publish.conf"
RELAY_LOCAL_JS="$REPO_ROOT/relay/relay.js"
RELAY_PATCH="$REPO_ROOT/relay/patches/0001-publish-hardening.patch"
RELAY_PATCH2="$REPO_ROOT/relay/patches/0002-real-client-ip.patch"   # r63: real viewer IP (X-Forwarded-For) for the host guard
RELAY_LOCAL_CLIENTIP="$REPO_ROOT/relay/clientip.js"                # required by relay.js after patch 0002 (same directory)
RELAY_BASE_COMMIT="cc41c25"   # commit that holds the imported live snapshot (docs/RELAY-LIVE-IMPORT.md)

# relay_dropin_text MODE REQUIRE_SIG
#   MODE compat : same behaviour as today for publishers (public URL + ?token= still work),
#                 PLUS signature checking when a client sends X-Fv-Sig.
#   MODE strict : LAN-only publish, token only in a header.
relay_dropin_text() {
  local mode="$1" require="$2" pub
  pub=$(update_pubkey)
  [ -n "$pub" ] || die "cannot read the update public key from src/update.rs"
  printf '%s\n' "# FreeViewer rollout block 2 (r2): publish hardening. Mode: $mode. Managed by rollout/02-relay."
  printf '%s\n' "[Service]"
  printf '%s\n' "Environment=FV_UPDATE_PUBKEY=$pub"
  if [ "$mode" = compat ]; then
    printf '%s\n' "Environment=FV_PUBLISH_LAN_ONLY=0" "Environment=FV_PUBLISH_QUERY_TOKEN=1"
  fi
  if [ "$require" = 1 ]; then printf '%s\n' "Environment=FV_REQUIRE_SIG=1"; fi
}

# relay_health -> prints the health json from CT110 (empty on failure)
relay_health() { rx relay "curl -fsS -m5 http://127.0.0.1:7180/fv/health" 2>/dev/null || true; }

# relay_write_and_restart MODE REQUIRE_SIG
# Writes the drop-in, restarts the relay, checks health, and rolls back the
# drop-in automatically when the relay does not come back healthy.
relay_write_and_restart() {
  local mode="$1" require="$2" text
  text=$(relay_dropin_text "$mode" "$require")
  mx relay "write drop-in ($mode, require_sig=$require), restart, health check, auto-undo on failure" "set -e
mkdir -p $RELAY_DROPIN_DIR
[ -e $RELAY_DROPIN ] && cp -p $RELAY_DROPIN $RELAY_DROPIN.prev || true
cat > $RELAY_DROPIN <<'FVEOF'
$text
FVEOF
systemctl daemon-reload
systemctl restart $RELAY_UNIT
sleep 3
if ! curl -fsS -m5 http://127.0.0.1:7180/fv/health | grep -q '\"ok\":true'; then
  echo 'HEALTH FAILED - undoing the drop-in'
  if [ -e $RELAY_DROPIN.prev ]; then mv -f $RELAY_DROPIN.prev $RELAY_DROPIN; else rm -f $RELAY_DROPIN; fi
  systemctl daemon-reload; systemctl restart $RELAY_UNIT; exit 1
fi
rm -f $RELAY_DROPIN.prev
echo 'relay healthy'"
}
