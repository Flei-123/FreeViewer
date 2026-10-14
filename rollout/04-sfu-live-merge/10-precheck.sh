#!/usr/bin/env bash
# Block 4 / step 10 - READ-ONLY: is CT111 still at the state LIVE-MERGE.md was written for?
# Usage: 10-precheck.sh
. "$(dirname "$0")/../lib/common.sh"
usage() { sed -n '2,3p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: precheck"
head=$(rx fm "git -C $FM_DIR rev-parse --short HEAD")
note "CT111 HEAD: $head (LIVE-MERGE.md was written for 43d84ff)"
[ "$head" = 43d84ff ] && ok "HEAD as documented" || bad "HEAD moved - redo the comparison in LIVE-MERGE.md"
rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "tree clean" || bad "uncommitted changes on CT111"
rx fm "command -v cargo >/dev/null && echo yes" | grep -q yes && ok "cargo present" || bad "cargo missing"
[ -d /root/projects/freemeet-sfu/rollout ] && ok "W19 rollout dir exists - use it for merge/build" || note "W19 rollout dir missing - follow LIVE-MERGE.md by hand"
c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l"); note "connections now: $c"
finish
