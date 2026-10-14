#!/usr/bin/env bash
# Block 3 / step 40 - switch the BROWSER to E2E v2 (= new SFU binary with the new e2e.js).
#
# Three stages, each can be run alone (--stage file|build|restart); default = all three:
#   file    replace /opt/freemeet/web/e2e.js (verified by sha) and commit it in the CT111 git
#   build   nice -n 19 cargo build --release (background job, polled; the live SFU keeps
#           running on the old inode, the old binary is saved as freemeet-sfu.bak_nonce_v1)
#   restart systemctl restart freemeet  ->  ALL RUNNING MEETINGS END (clients must rejoin)
#           + health check; auto-undo (old binary back) if the service does not come up.
# Do the restart in the maintenance window (NOTICE-wartungsfenster.txt), not before the gate
# (20-gate-native.sh) was passed.
#
# Usage: 40-apply.sh [--apply] [--stage file|build|restart] [--force]
#   --force  restart although connections exist
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,15p' "$0"; }
fv_parse_args "$@"
banner "freemeet nonce: apply"
NEWJS="$REPO_ROOT/web-patch/e2e.js"
newsha=$(sha_of "$NEWJS")
stage=$(fv_opt stage all)
[[ "$stage" =~ ^(all|file|build|restart)$ ]] || die "--stage all|file|build|restart"
do_stage() { [ "$stage" = all ] || [ "$stage" = "$1" ]; }

step "checks"
rx fm "test -e $FM_DIR/freemeet-sfu.bak_nonce_v1 && test -e $FM_DIR/web/e2e.js.bak_nonce_v1 && echo yes" | grep -q yes && ok "backups exist (30-backup.sh)" || bad "run 30-backup.sh --apply first"
rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -qx pre-nonce-v2 && ok "LXC snapshot pre-nonce-v2 exists" || bad "snapshot pre-nonce-v2 missing"
if do_stage file; then
  live=$(rx fm "sha256sum $FM_DIR/web/e2e.js | cut -d' ' -f1")
  if [ "$live" = "$FM_E2E_LIVE_SHA" ]; then ok "live e2e.js is the known v1 file"; elif [ "$live" = "$newsha" ]; then note "e2e.js on CT111 is already v2 (file stage will be a no-op)"; else bad "live e2e.js unknown ($live)"; fi
fi
if do_stage restart; then
  c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l")
  if [ "$c" = 0 ]; then ok "no connections to the SFU"; elif fv_flag force; then warn "$c connection(s), --force given"; else bad "$c connection(s) to the SFU - a restart ends these meetings. Wait or use --force."; fi
  if [ "$stage" = restart ]; then
    if rx fm "grep -c freemeet-e2e-v2 $FM_DIR/target/release/freemeet-sfu" | grep -qv '^0'; then ok "binary on disk contains the v2 e2e.js"; else bad "binary on disk has NO v2 e2e.js - run --stage build first"; fi
  fi
fi
gate

if do_stage file; then
  step "stage file"
  put fm "$NEWJS" "$FM_DIR/web/e2e.js.new"
  mx fm "verify sha, move into place, commit" "cd $FM_DIR && test \"\$(sha256sum web/e2e.js.new | cut -d' ' -f1)\" = '$newsha' && mv -f web/e2e.js.new web/e2e.js && git add web/e2e.js && { git diff --cached --quiet || git commit -q -m 'FreeMeet E2E v2: per-sender salt + unique nonces (r1)'; } && git log --oneline -1 && sha256sum web/e2e.js"
fi

if do_stage build; then
  step "stage build (CT111: 4 cores, the live SFU keeps running; niced)"
  mx fm "start background build" "cd $FM_DIR && rm -f /root/fm-build.done /root/fm-build.fail && nohup sh -c 'nice -n 19 cargo build --release --locked >/root/fm-build.log 2>&1 && touch /root/fm-build.done || touch /root/fm-build.fail' >/dev/null 2>&1 &"
  if ((APPLY)); then
    info "  polling the build (max 30 min) ..."
    for _ in $(seq 1 180); do
      sleep 10
      r=$(rx fm "if [ -e /root/fm-build.done ]; then echo done; elif [ -e /root/fm-build.fail ]; then echo fail; else tail -n1 /root/fm-build.log 2>/dev/null | cut -c1-100; fi")
      case "$r" in
        done) ok "cargo build finished"; break ;;
        fail) rx fm "tail -n 25 /root/fm-build.log"; die "cargo build FAILED (live service untouched, old binary still running; undo the file stage with 90-rollback.sh --file-only)" ;;
        *) printf '    %s\n' "$r" ;;
      esac
    done
    rx fm "grep -c freemeet-e2e-v2 $FM_DIR/target/release/freemeet-sfu" | grep -qv '^0' && ok "new binary contains e2e v2" || die "new binary has no v2 marker - do NOT restart"
    rx fm "$FM_DIR/target/release/freemeet-sfu --help >/dev/null 2>&1; echo exit=\$?" | sed 's/^/  [INFO] binary smoke (--help): /'
  fi
fi

if do_stage restart; then
  step "stage restart (meetings end now)"
  confirm "Restart FreeMeet now? All running meetings end, participants have to rejoin."
  mx fm "restart + health check + auto-undo" "set -e
cd $FM_DIR
systemctl restart $FM_UNIT
sleep 3
if ! systemctl is-active --quiet $FM_UNIT || ! curl -fsS -m5 -o /dev/null http://127.0.0.1:7200/e2e.js; then
  echo 'SERVICE NOT HEALTHY - putting the old binary back'
  cp -p freemeet-sfu.bak_nonce_v1 target/release/freemeet-sfu.restore && mv -f target/release/freemeet-sfu.restore target/release/freemeet-sfu
  systemctl restart $FM_UNIT; exit 1
fi
echo 'service healthy'"
fi
note "next: 50-verify.sh, then CHECKLIST.md with real browsers"
finish
