#!/usr/bin/env bash
# Block 2 / step 55 - fetch the Mac zip built by GitHub Actions and publish it SIGNED over the LAN (r28 + Mac CI upload).
#
# WHY: the CI runner must NOT push to the relay (public URL + token in the URL = the hole r2 closes) and must NOT hold
# the update private key. So the trust chain is:  CI builds + attests the zip  ->  this script (LAN, where the key lives)
# downloads the artifact, checks it, signs it and publishes it with 50-publish-release.sh (platform=mac).
# The CI never gets a relay token; a hacked runner can at most produce an artifact you can inspect before it is signed.
#
# Checks before anything is signed: run is a successful `push` run of branch main (not a PR), the zip's Info.plist
# version is read from the zip itself, 50-publish-release.sh re-checks zip/version/exec bit, and (if `gh` is installed)
# the GitHub build attestation of the zip is verified (--require-attest makes it mandatory).
# Downloads go to a temp dir that is removed at the end. Dry-run downloads + checks but publishes nothing.
#
# Usage: 55-mac-pull.sh [--brand freeviewer|x-remote] [--run-id N] [--repo OWNER/REPO] [--zip PATH]
#                       [--require-attest] [--apply] [--yes]
#   --zip PATH   skip GitHub: use a zip you downloaded by hand (Actions page -> artifact -> unzip)
# Env:   GH_TOKEN  GitHub token with read access to Actions artifacts of the repo (only needed without --zip;
#        never stored in the repo; fine-grained "Actions: read" is enough)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,19p' "$0"; }
fv_parse_args "$@"
banner "relay: pull the Mac build from CI and publish it signed"
need_cmd python3
need_cmd unzip

brand=$(fv_opt brand freeviewer)
repo=$(fv_opt repo Flei-123/FreeViewer)
runid=$(fv_opt run-id "")
zipin=$(fv_opt zip "")
[[ "$brand" =~ ^(freeviewer|x-remote)$ ]] || die "--brand freeviewer|x-remote"
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo OWNER/REPO"
[ -z "$runid" ] || [[ "$runid" =~ ^[0-9]+$ ]] || die "--run-id must be a number"
if [ "$brand" = freeviewer ]; then art=freeviewer-macos-arm64; zipname=FreeViewer-macOS.zip; else art=x-remote-macos-arm64; zipname=X-Remote-macOS.zip; fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh_api() { curl -fsS -m60 -H "Authorization: Bearer ${GH_TOKEN}" -H 'Accept: application/vnd.github+json' "https://api.github.com/repos/$repo/$1"; }
json() { python3 -c "import sys,json
d=json.loads(sys.stdin.read())
$1"; }

step "get the zip"
if [ -n "$zipin" ]; then
  [ -f "$zipin" ] || die "--zip: not a file: $zipin"
  zip="$zipin"
  ok "using $zip (manual download)"
else
  [ -n "${GH_TOKEN:-}" ] || die "GH_TOKEN not set (read access to Actions artifacts) - or pass --zip PATH"
  if [ -z "$runid" ]; then
    runs=$(gh_api "actions/workflows/mac.yml/runs?branch=main&event=push&status=success&per_page=1") || die "GitHub API: cannot list runs"
    runid=$(json 'r=d.get("workflow_runs") or [{}]
print(r[0].get("id",""))' <<<"$runs")
    [ -n "$runid" ] || die "no successful push run of mac.yml on main"
    sha_head=$(json 'print((d.get("workflow_runs") or [{}])[0].get("head_sha",""))' <<<"$runs")
    ok "latest successful main run: $runid (commit ${sha_head:0:10})"
  else
    run=$(gh_api "actions/runs/$runid") || die "GitHub API: run $runid not found"
    [ "$(json 'print(d.get("head_branch",""))' <<<"$run")" = main ] || bad "run $runid is not from branch main"
    [ "$(json 'print(d.get("event",""))' <<<"$run")" = push ] || bad "run $runid is not a push run (PR build?)"
    [ "$(json 'print(d.get("conclusion",""))' <<<"$run")" = success ] || bad "run $runid did not succeed"
    sha_head=$(json 'print(d.get("head_sha",""))' <<<"$run")
    ok "run $runid checked (commit ${sha_head:0:10})"
  fi
  arts=$(gh_api "actions/runs/$runid/artifacts") || die "GitHub API: cannot list artifacts"
  aurl=$(json "for a in d.get('artifacts', []):
    if a.get('name') == '$art' and not a.get('expired'):
        print(a.get('archive_download_url', ''))
        break" <<<"$arts")
  [ -n "$aurl" ] || die "artifact '$art' not found (or expired) in run $runid"
  curl -fsSL -m300 -H "Authorization: Bearer ${GH_TOKEN}" -o "$work/artifact.zip" "$aurl" || die "artifact download failed"
  mkdir -p "$work/art"
  unzip -q -o "$work/artifact.zip" -d "$work/art" || die "artifact is not a zip"
  zip="$work/art/$zipname"
  [ -f "$zip" ] || die "artifact does not contain $zipname"
  ok "artifact $art downloaded: $(stat -c %s "$zip") bytes, sha256 $(sha_of "$zip")"
fi

step "checks"
plist_ver=$(unzip -p "$zip" '*.app/Contents/Info.plist' 2>/dev/null | tr -d '\n' | sed -n 's/.*CFBundleShortVersionString<\/key> *<string>\([^<]*\)<.*/\1/p' | head -n1)
[[ "$plist_ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && ok "zip says version $plist_ver" || die "cannot read a version from the zip's Info.plist"
if command -v gh >/dev/null 2>&1; then
  if gh attestation verify "$zip" --repo "$repo" >/dev/null 2>&1; then ok "GitHub build attestation verifies (zip was built by $repo CI)"; else bad "attestation does NOT verify"; fi
elif fv_flag require-attest; then
  bad "--require-attest given but the gh CLI is missing"
else
  note "gh CLI not installed: attestation not checked (inspect the run on GitHub, then confirm below)"
fi
gate

step "sign + publish (50-publish-release.sh)"
pass=()
((APPLY)) && pass+=(--apply)
((ASSUME_YES)) && pass+=(--yes)
bash "$(dirname "$0")/50-publish-release.sh" --platform mac --file "$zip" --version "$plist_ver" --brand "$brand" "${pass[@]+"${pass[@]}"}"
finish
