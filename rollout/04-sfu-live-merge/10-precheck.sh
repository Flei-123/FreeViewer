#!/usr/bin/env bash
# Block 4 / step 10 - READ-ONLY: is the W19 package sound and is CT111 still at the state it was proven for?
#  * local: series 0001..N applies to the `live` import and gives exactly branch `live-merged`
#  * CT111: HEAD is 43d84ff (or a child that only changed web/e2e.js = block 3 stage `file`), tree clean, cargo present
# Usage: 10-precheck.sh
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,5p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: precheck"

step "local W19 package ($SFU_PATCH_DIR)"
n=$(sfu_patches | wc -l)
if [ -d "$SFU_REPO/.git" ]; then
  [ "$n" -ge 1 ] && ok "$n patch files" || bad "no patch files in $SFU_PATCH_DIR"
  if git -C "$SFU_REPO" rev-parse -q --verify "$SFU_BASE_BRANCH" >/dev/null && git -C "$SFU_REPO" rev-parse -q --verify "$SFU_MERGED_BRANCH" >/dev/null; then
    sfu_prove_series && ok "series applies with git am -3 to '$SFU_BASE_BRANCH' and equals '$SFU_MERGED_BRANCH' (empty diff)" || bad "series does NOT reproduce '$SFU_MERGED_BRANCH'"
  else
    bad "branches '$SFU_BASE_BRANCH' / '$SFU_MERGED_BRANCH' missing in $SFU_REPO"
  fi
  if sfu_patches | xargs -r grep -l '^+++ b/web/' >/dev/null 2>&1; then bad "a patch touches web/ - the series must be server-only"; else ok "no patch touches web/"; fi
else
  bad "$SFU_REPO is not a git repo"
fi

step "live CT111"
head=$(rx fm "git -C $FM_DIR rev-parse --short HEAD" || true)
note "CT111 HEAD: $head (series proven against $SFU_LIVE_HEAD)"
if [ "$head" = "$SFU_LIVE_HEAD" ]; then
  ok "HEAD as documented"
else
  rx fm "git -C $FM_DIR merge-base --is-ancestor $SFU_LIVE_HEAD HEAD && echo yes" | grep -q yes || bad "HEAD is not a descendant of $SFU_LIVE_HEAD - redo the comparison in docs/LIVE-MERGE.md"
  ch=$(rx fm "git -C $FM_DIR diff --name-only $SFU_LIVE_HEAD HEAD | grep -vc '^web/e2e.js\$' || true")
  [ "$ch" = 0 ] && ok "only web/e2e.js changed since $SFU_LIVE_HEAD (block 3 stage file)" || bad "other files changed since $SFU_LIVE_HEAD ($ch) - patches may not apply"
fi
rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "tree clean" || bad "uncommitted changes on CT111"
rx fm "git -C $FM_DIR rev-parse -q --verify refs/heads/$SFU_BRANCH >/dev/null && echo yes" | grep -q yes && note "branch $SFU_BRANCH already exists (20-merge.sh will skip)" || ok "branch $SFU_BRANCH free"
rx fm "command -v cargo >/dev/null && echo yes" | grep -q yes && ok "cargo present" || bad "cargo missing"
rx fm "df -Pm /opt | awk 'NR==2{print \$4}'" | awk '{ if ($1+0 >= 3000) print "  [ OK ] free space on /opt: " $1 " MB"; else print "  [FAIL] only " $1 " MB free on /opt (build needs ~3 GB)" }'
rx fm "systemctl cat $FM_UNIT 2>/dev/null | grep -E '^(ExecStart|Environment)'" | sed 's/^/  [INFO] unit: /' || true
rx fm "ls $SFU_DROPIN 2>/dev/null" | grep -q . && note "drop-in $SFU_DROPIN exists already" || true
c=$(rx fm "ss -Htn state established '( sport = :7200 )' | wc -l"); note "connections now: $c"
finish
