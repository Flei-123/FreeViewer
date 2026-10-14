#!/usr/bin/env bash
# Runs every rollout script in DRY-RUN against fake ssh/scp and proves that
# no mutating command (marker FVMUT) and no scp was ever sent. Also: bash -n.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
export FV_FAKE_LOG; FV_FAKE_LOG="$(mktemp)"
trap 'rm -f "$FV_FAKE_LOG"' EXIT
export FV_SSH="$here/fakes/ssh" FV_SCP="$here/fakes/scp"
fail=0
while IFS= read -r f; do
  bash -n "$f" || { echo "SYNTAX FAIL $f"; fail=1; }
done < <(find "$root" -name '*.sh' -type f)
for s in "$root"/0*/[1-9]0-*.sh "$root"/02-relay/55-mac-pull.sh; do
  : >"$FV_FAKE_LOG"
  args=()
  case "$s" in *20-gate-native*) args=(--native-min-version 0.0.1 --yes) ;; esac
  case "$s" in *50-offsite*) args=(--offsite u@h:/d) ;; esac
  case "$s" in *50-publish-release*) args=(--platform win --file "$root/README.md" --version 9.9.9 --sig "$(printf 'a%.0s' $(seq 128))") ;; esac
  case "$s" in *55-mac-pull*) args=(--zip "$root/README.md") ;; esac
  case "$s" in *40-swap-binary*) args=(--binary /opt/freemeet/x) ;; esac
  out=$(bash "$s" "${args[@]}" 2>&1 </dev/null); rc=$?
  if grep -q FVMUT "$FV_FAKE_LOG"; then echo "FAIL (mutation in dry-run): $s"; fail=1; else echo "ok (no mutation, rc=$rc): ${s#"$root"/}"; fi
  grep -q 'MODE: DRY-RUN' <<<"$out" || { echo "FAIL (no dry-run banner): $s"; fail=1; }
done
exit $fail
