#!/usr/bin/env bash
# Block 2 / step 90 - ROLLBACK of the relay patch.
#   default      : relay.js.bak_r2 back in place (clientip.js moved aside), r2 drop-in removed, restart, health check.
#   --feed VER   : additionally restore the feed from dist/<feed>.pre-VER (a bad publish).
#   --lxc        : full revert with `pct rollback 110 pre-publish-hardening` (hard: node + app
#                  + hosts.json go back to the snapshot state, changes since then are LOST;
#                  a data copy is made first).
# Usage: 90-rollback.sh [--apply] [--feed X.Y.Z [--brand freeviewer|x-remote]] [--lxc]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,10p' "$0"; }
fv_parse_args "$@"
banner "relay: rollback"
ts=$(stamp)
ver=$(fv_opt feed "")
brand=$(fv_opt brand freeviewer)
[ "$brand" = freeviewer ] && feedname=version.json || feedname=version-xoffi.json

step "checks"
if fv_flag lxc; then
  rx pve "pct listsnapshot 110" | awk '{print $2}' | grep -qx pre-publish-hardening && ok "LXC snapshot exists" || bad "no pre-publish-hardening snapshot"
else
  rx relay "sha256sum $RELAY_DIR/relay.js.bak_r2 | cut -d' ' -f1" | grep -qx "$RELAY_LIVE_SHA" && ok "relay.js.bak_r2 is the original" || bad "relay.js.bak_r2 missing or not the original"
fi
[ -z "$ver" ] || { rx relay "test -e $RELAY_DIR/dist/$feedname.pre-$ver && echo yes" | grep -q yes && ok "feed backup $feedname.pre-$ver exists" || bad "feed backup .pre-$ver missing"; }
gate
confirm "Roll the relay back to the state before r2?"

if fv_flag lxc; then
  step "full LXC rollback"
  mx relay "copy data aside first" "mkdir -p /root/rollback-data-$ts && cp -a $RELAY_DIR/hosts.json $RELAY_DIR/devices.json $RELAY_DIR/freunde.json $RELAY_DIR/dist/version*.json /root/rollback-data-$ts/ 2>/dev/null; true"
  warn "the copy lives INSIDE the container and is gone after the rollback - pull it first with scp if you need it"
  mx pve "pct rollback 110 (stops and restarts the container)" "pct rollback 110 pre-publish-hardening && pct start 110"
else
  step "file rollback"
  mx relay "restore relay.js, remove drop-in, restart, health" "set -e
cd $RELAY_DIR
cp -p relay.js relay.js.failed-r2
cp -p relay.js.bak_r2 relay.js
[ ! -e buch.js.bak_r2 ] || { cp -p buch.js buch.js.failed-r2; cp -p buch.js.bak_r2 buch.js; }
[ ! -e clientip.js ] || mv -f clientip.js clientip.js.failed-r2
[ ! -e reqauth.js ] || mv -f reqauth.js reqauth.js.failed-r2
rm -f $RELAY_DROPIN $RELAY_DROPIN.prev
systemctl daemon-reload
systemctl restart $RELAY_UNIT
sleep 3
curl -fsS -m5 http://127.0.0.1:7180/fv/health
echo"
fi
if [ -n "$ver" ]; then
  step "feed rollback ($feedname -> .pre-$ver)"
  mx relay "restore the feed" "cd $RELAY_DIR/dist && cp -p $feedname $feedname.failed-$ts && cp -p $feedname.pre-$ver $feedname && cat $feedname | head -c 300"
fi
if ((APPLY)); then
  step "verify"
  relay_health | grep -q '"ok":true' && ok "relay healthy after rollback" || bad "relay NOT healthy"
  rx relay "sha256sum $RELAY_DIR/relay.js | cut -d' ' -f1" | grep -qx "$RELAY_LIVE_SHA" && ok "relay.js == original" || note "relay.js differs from the original (expected after --lxc if snapshot is older)"
fi
finish
