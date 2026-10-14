# shellcheck shell=bash
# shellcheck disable=SC2034  # constants are used by the scripts that source this file
# Helpers shared by the SFU merge scripts (block 4). Source after lib/common.sh.
# The patch series is the finished W19 package: $SFU_REPO/rollout/0001..0013-*.patch,
# built from branch `live-merged` (= `live` import of CT111 43d84ff + fv-w15/w19 work).

SFU_REPO="${SFU_REPO:-/root/projects/freemeet-sfu}"
SFU_PATCH_DIR="$SFU_REPO/rollout"
SFU_LIVE_HEAD="43d84ff"             # CT111 /opt/freemeet HEAD the series was proven against
SFU_BASE_BRANCH="live"              # branch in $SFU_REPO that mirrors CT111 43d84ff
SFU_MERGED_BRANCH="live-merged"     # expected result of the series
SFU_BRANCH="fv-w19"                 # branch name created on CT111
SFU_REMOTE_DIR="/root/fv-w19-rollout"
SFU_BUILD_TARGET="/opt/freemeet-build-w19"   # OUTSIDE the git tree on purpose (keeps `git status` clean)
SFU_NEW_BINARY="$FM_DIR/freemeet-sfu.w19"
SFU_DROPIN_DIR="/etc/systemd/system/${FM_UNIT}.service.d"
SFU_DROPIN="$SFU_DROPIN_DIR/fv-w19.conf"

# sfu_patches -> prints the patch files, sorted, one per line
sfu_patches() { find "$SFU_PATCH_DIR" -maxdepth 1 -name '[0-9][0-9][0-9][0-9]-*.patch' -type f | sort; }

# sfu_prove_series -> 0 if (live branch + series) has exactly the tree of live-merged.
# Works in a throw-away clone, nothing in $SFU_REPO is touched.
sfu_prove_series() {
  local tmp rc=0
  tmp=$(mktemp -d)
  (
    set -e
    git clone -q --no-hardlinks -b "$SFU_BASE_BRANCH" "$SFU_REPO" "$tmp/c" 2>/dev/null
    cd "$tmp/c"
    git fetch -q origin "$SFU_MERGED_BRANCH"
    mapfile -t pf < <(sfu_patches)
    git -c user.name=fv-rollout -c user.email=rollout@localhost am -3 -q "${pf[@]}"
    git diff --quiet HEAD FETCH_HEAD
  ) >/dev/null 2>&1 || rc=1
  rm -rf "$tmp"
  return $rc
}

# sfu_expected_hashes -> "<freemeet-sfu tree> <Cargo.toml blob> <Cargo.lock blob>" of live-merged
sfu_expected_hashes() {
  git -C "$SFU_REPO" rev-parse "$SFU_MERGED_BRANCH:freemeet-sfu" "$SFU_MERGED_BRANCH:Cargo.toml" "$SFU_MERGED_BRANCH:Cargo.lock" | tr '\n' ' '
}
