#!/usr/bin/env bash
# Block 4 / step 20 - put the W19 patch series on a NEW git branch (fv-w19) on CT111. No build, no restart,
# the running service is not touched (it runs the old binary; sources on disk do not matter to it).
# Aborts and restores the old branch if `git am -3` does not apply cleanly.
# Afterwards the Rust sources equal branch live-merged (tree hashes are compared).
# Usage: 20-merge.sh [--apply]
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-sfu.sh"
usage() { sed -n '2,6p' "$0"; }
fv_parse_args "$@"
banner "sfu merge: git am on CT111"

step "checks"
mapfile -t pf < <(sfu_patches)
[ "${#pf[@]}" -ge 1 ] && ok "${#pf[@]} patch files" || bad "no patch files in $SFU_PATCH_DIR"
sfu_prove_series && ok "local proof: series == $SFU_MERGED_BRANCH" || bad "local proof failed - run 10-precheck.sh"
rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "CT111 tree clean" || bad "uncommitted changes on CT111"
rx fm "git -C $FM_DIR rev-parse -q --verify refs/heads/$SFU_BRANCH >/dev/null && echo yes" | grep -q yes && bad "branch $SFU_BRANCH exists already (merge done? 90-rollback.sh, then delete it by hand)" || ok "branch $SFU_BRANCH free"
rx pve "pct listsnapshot 111" | awk '{print $2}' | grep -q '^pre-nonce-v2$\|^fv-rollout-' && ok "LXC snapshot exists" || bad "no snapshot - run 03-freemeet-nonce/30-backup.sh --apply first"
gate

step "copy the series + checksums"
mx fm "create $SFU_REMOTE_DIR" "mkdir -p $SFU_REMOTE_DIR && rm -f $SFU_REMOTE_DIR/*.patch $SFU_REMOTE_DIR/SHA256SUMS"
sums=$(mktemp)
trap 'rm -f "$sums"' EXIT
for p in "${pf[@]}"; do
  put fm "$p" "$SFU_REMOTE_DIR/$(basename "$p")"
  printf '%s  %s\n' "$(sha_of "$p")" "$(basename "$p")" >>"$sums"
done
put fm "$sums" "$SFU_REMOTE_DIR/SHA256SUMS"

step "git am -3 on a new branch"
mx fm "verify checksums, branch $SFU_BRANCH, git am -3 (auto-abort on conflict)" "set -e
cd $SFU_REMOTE_DIR && sha256sum -c --quiet SHA256SUMS
cd $FM_DIR
git rev-parse --abbrev-ref HEAD > $SFU_REMOTE_DIR/prev-branch
git rev-parse HEAD > $SFU_REMOTE_DIR/prev-head
git switch -q -c $SFU_BRANCH
if ! git -c user.name=fv-rollout -c user.email=rollout@localhost am -3 -q $SFU_REMOTE_DIR/0*.patch; then
  echo 'git am FAILED - aborting, old branch restored'
  git am --abort || true
  git switch -q \$(cat $SFU_REMOTE_DIR/prev-branch)
  git branch -D $SFU_BRANCH
  exit 1
fi
git log --oneline -1"

if ((APPLY)); then
  step "proof: Rust sources == $SFU_MERGED_BRANCH"
  want=$(sfu_expected_hashes)
  got=$(rx fm "git -C $FM_DIR rev-parse HEAD:freemeet-sfu HEAD:Cargo.toml HEAD:Cargo.lock | tr '\n' ' '")
  [ "$got" = "$want" ] && ok "freemeet-sfu/, Cargo.toml and Cargo.lock identical to $SFU_MERGED_BRANCH" || bad "differs from $SFU_MERGED_BRANCH: want '$want' got '$got' - do NOT build; 90-rollback.sh"
  rx fm "git -C $FM_DIR status --porcelain | wc -l" | grep -qx 0 && ok "tree clean" || bad "tree not clean after am"
  note "service untouched. Next: 30-build.sh --apply"
fi
finish
