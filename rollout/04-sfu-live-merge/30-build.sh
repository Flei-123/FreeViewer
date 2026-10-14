#!/usr/bin/env bash
# Block 4 / step 30 - build the merged SFU on CT111 into a SEPARATE target dir (live binary and its
# target/ stay untouched; the service keeps running). Result: /opt/freemeet/freemeet-sfu.w19.
# Runs niced as a background job and is polled. A cold build takes a while (rayon etc.).
# Combined run with block 3: do 03-freemeet-nonce/40-apply.sh --stage file FIRST, so the new
# web/e2e.js is compiled into this binary (include_str!); then ONE restart via 40-swap-binary.sh.
# Usage: 30-build.sh [--apply] [--tests]     (--tests also runs cargo test --release, ~36 tests)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,8p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: build"
NEWJS="$REPO_ROOT/web-patch/e2e.js"

step "checks"
br=$(rx fm "git -C $FM_DIR rev-parse --abbrev-ref HEAD" || true)
[ "$br" = "$SFU_BRANCH" ] && ok "CT111 is on branch $SFU_BRANCH" || bad "CT111 branch is '$br', expected $SFU_BRANCH - run 20-merge.sh --apply"
rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "tree clean" || bad "uncommitted changes on CT111"
rx fm "df -Pm /opt | awk 'NR==2{print \$4}'" | awk '{ if ($1+0 >= 3000) print "  [ OK ] free space on /opt: " $1 " MB"; else print "  [FAIL] only " $1 " MB free on /opt" }'
live=$(rx fm "sha256sum $FM_DIR/web/e2e.js | cut -d' ' -f1" || true)
if [ -f "$NEWJS" ] && [ "$live" = "$(sha_of "$NEWJS")" ]; then ok "web/e2e.js on CT111 is v2 -> it will be inside this binary"
else note "web/e2e.js on CT111 is NOT v2 (block 3 stage file not done) - the binary will carry the old e2e.js"; fi
gate

step "build (background, niced)"
tests=""; fv_flag tests && tests=" && CARGO_TARGET_DIR=$SFU_BUILD_TARGET nice -n 19 cargo test --release --locked"
mx fm "start background build into $SFU_BUILD_TARGET" "cd $FM_DIR && rm -f /root/fm-w19-build.done /root/fm-w19-build.fail && nohup sh -c 'CARGO_TARGET_DIR=$SFU_BUILD_TARGET nice -n 19 cargo build --release --locked$tests >/root/fm-w19-build.log 2>&1 && cp -p $SFU_BUILD_TARGET/release/freemeet-sfu $SFU_NEW_BINARY && touch /root/fm-w19-build.done || touch /root/fm-w19-build.fail' >/dev/null 2>&1 &"
if ((APPLY)); then
  info "  polling the build (max 45 min) ..."
  res=timeout
  for _ in $(seq 1 270); do
    sleep 10
    r=$(rx fm "if [ -e /root/fm-w19-build.done ]; then echo done; elif [ -e /root/fm-w19-build.fail ]; then echo fail; else tail -n1 /root/fm-w19-build.log 2>/dev/null | cut -c1-100; fi")
    case "$r" in
      done) res=done; break ;;
      fail) res=fail; break ;;
      *) printf '    %s\n' "$r" ;;
    esac
  done
  [ "$res" = done ] || { rx fm "tail -n 25 /root/fm-w19-build.log"; die "build $res (live service untouched, nothing to undo; remove $SFU_BUILD_TARGET if you want the space)"; }
  ok "cargo build finished"
  rx fm "grep -c FM_WORKERS $SFU_NEW_BINARY" | grep -qv '^0' && ok "binary knows FM_WORKERS (merged build)" || die "binary has no FM_WORKERS - wrong build, do NOT swap"
  if [ -f "$NEWJS" ]; then
    rx fm "grep -c freemeet-e2e-v2 $SFU_NEW_BINARY" | grep -qv '^0' && ok "binary contains the v2 e2e.js" || note "binary has no v2 e2e.js marker (fine without block 3)"
  fi
  rx fm "sha256sum $SFU_NEW_BINARY"
  note "next: 40-swap-binary.sh --binary $SFU_NEW_BINARY --apply   (the one restart)"
fi
finish
