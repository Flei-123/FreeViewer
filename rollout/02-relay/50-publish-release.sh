#!/usr/bin/env bash
# Block 2 / step 50 - publish a SIGNED release (the first one = signature phase 1 -> 2).
#
# Runs where the update private key is (or pass --sig), uploads over ssh to CT110
# and talks to the relay on 127.0.0.1 (= "from the LAN", works in strict mode).
#
#   win : uploads the exe with header X-Fv-Sig (the relay verifies it and writes "sig" into the feed).
#         The Mac fields (mac_*) stay exactly as they are - they have their own version and signature.
#   mac : uploads the zip with platform=mac + X-Fv-Sig (signature over platform "mac" + the Mac version);
#         the relay (patch 0003) checks it and writes mac_version/mac_url/mac_sha256/mac_size/mac_sig itself.
#         Before the upload the zip is checked: it is a zip, its Info.plist version == --version,
#         the app binary keeps its executable bit.
#
# WHY the two platforms are independent (roadmap r28): the Mac client verifies "mac_sig" over the feed field
# "mac_version" (src/update.rs feed_version), the Windows client over "version". A Windows publish therefore
# cannot invalidate mac_sig, and a Mac publish cannot touch the Windows fields. Feeds that carry no mac_version
# (very old) fall back to "version" in the client - for those, publish Windows and Mac with the same version.
# Mac packages come from CI: fetch + check them with 55-mac-pull.sh, which calls this script.
#
# Usage: 50-publish-release.sh --platform win|mac --file PATH --version X.Y.Z
#          [--brand freeviewer|x-remote] [--notes TEXT] [--sig HEX] [--apply]
# Env:   FV_SIGN_KEY (default /root/fvwork/keys/update-ed25519.key)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,24p' "$0"; }
fv_parse_args "$@"
banner "relay: publish a signed release"
need_cmd python3
SIGN="$REPO_ROOT/tools/sign-release/sign_release.py"
KEY="${FV_SIGN_KEY:-/root/fvwork/keys/update-ed25519.key}"

platform=$(fv_opt platform "")
file=$(fv_opt file "")
version=$(fv_opt version "")
brand=$(fv_opt brand freeviewer)
notes=$(fv_opt notes "")
sig=$(fv_opt sig "")
[[ "$platform" =~ ^(win|mac)$ ]] || die "--platform win|mac required"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z required"
[[ "$brand" =~ ^(freeviewer|x-remote)$ ]] || die "--brand freeviewer|x-remote"
[ -f "$file" ] || die "--file missing or not a file: '$file'"
re_sig='^[0-9a-f]{128}$'
[ -z "$sig" ] || [[ "$sig" =~ $re_sig ]] || die "--sig must be 128 hex chars"
# shellcheck disable=SC2100
if [ "$brand" = freeviewer ]; then feed=version.json; else feed=version-xoffi.json; fi
if [ "$platform" = win ]; then
  if [ "$brand" = freeviewer ]; then name=freeviewer.exe; else name=x-remote.exe; fi
else
  if [ "$brand" = freeviewer ]; then name=FreeViewer-macOS.zip; else name=X-Remote-macOS.zip; fi
fi
sha=$(sha_of "$file")
size=$(stat -c %s "$file")

# sign_fields PLATFORM VERSION SHA SIZE -> hex signature
sign_fields() {
  python3 -I "$SIGN" sign-fields --platform "$1" --version "$2" --sha256 "$3" --size "$4" --key "$KEY"
}

step "checks"
ok "file $name: sha ${sha:0:16}... size $size"
cur=$(rx relay "python3 -c \"import json;print(json.load(open('$RELAY_DIR/dist/$feed'))['version'])\"" || true)
if [ "$platform" = mac ]; then
  command -v unzip >/dev/null 2>&1 || bad "unzip missing (needed to check the Mac zip)"
  if [ "$(head -c4 "$file" | od -An -tx1 | tr -d ' \n')" = 504b0304 ]; then ok "file is a zip"; else bad "file is not a zip (no PK header)"; fi
  plist_ver=$(unzip -p "$file" '*.app/Contents/Info.plist' 2>/dev/null | tr -d '\n' | sed -n 's/.*CFBundleShortVersionString<\/key> *<string>\([^<]*\)<.*/\1/p' | head -n1)
  if [ "$plist_ver" = "$version" ]; then ok "Info.plist version == $version"; else bad "Info.plist version is '${plist_ver:-none}', --version says $version (wrong build?)"; fi
  if zipinfo "$file" 2>/dev/null | grep -Eq '^-rwx.*\.app/Contents/MacOS/'; then ok "app binary keeps its executable bit"; else bad "no executable file in .app/Contents/MacOS (zip re-packed on Windows? use ditto)"; fi
  curmac=$(rx relay "python3 -c \"import json;print(json.load(open('$RELAY_DIR/dist/$feed')).get('mac_version',''))\"" || true)
  if [ -n "$curmac" ] && [ "$(printf '%s\n%s\n' "$curmac" "$version" | sort -V | tail -n1)" != "$version" ]; then bad "version $version is older than mac_version $curmac in the feed (relay would answer 409)"; else ok "version $version >= mac_version ${curmac:-none}"; fi
fi
if [ "$platform" = win ]; then
  if [ -n "$cur" ] && [ "$(printf '%s\n%s\n' "$cur" "$version" | sort -V | tail -n1)" = "$version" ] && [ "$cur" != "$version" ]; then ok "version $version > current $cur"; else bad "version $version is not newer than the feed ($cur)"; fi
fi
pub=$(update_pubkey)
if [ -z "$sig" ]; then
  if [ -r "$KEY" ]; then
    k=$(python3 -I "$SIGN" pubkey --key "$KEY")
    if [ "$k" = "$pub" ]; then ok "signing key matches the key compiled into the client"; elif fv_flag allow-key-mismatch; then warn "key mismatch ignored (--allow-key-mismatch, tests only)"; else bad "signing key does not match src/update.rs - the clients would reject the signature"; fi
  else
    bad "no signing key at $KEY and no --sig given"
  fi
fi
state=$(relay_health)
grep -q '"ok":true' <<<"$state" && ok "relay healthy" || bad "relay not healthy"
rx relay "systemctl show $RELAY_UNIT -p Environment" | grep -q FV_UPDATE_PUBKEY && ok "relay verifies signatures (pubkey set)" || bad "relay has no FV_UPDATE_PUBKEY - run 30-apply.sh first"
gate

if [ "$platform" = win ]; then
  [ -n "$sig" ] || sig=$(sign_fields win "$version" "$sha" "$size")
  ok "signature (win, $version): ${sig:0:16}..."
  enc_notes=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1], safe=""))' "$notes")
  step "upload + publish via 127.0.0.1 on CT110"
  confirm "Publishing $name $version to ALL clients of the $brand brand (auto-update will offer it). Continue?"
  put relay "$file" "$RELAY_DIR/staging-$name"
  mx relay "feed backup + signed publish" "cd $RELAY_DIR && cp -p dist/$feed dist/$feed.pre-$version && TOK=\$(cat upload.token) && curl -fsS -m600 -X POST -H \"Authorization: Bearer \$TOK\" -H 'X-Fv-Sig: $sig' --data-binary @staging-$name 'http://127.0.0.1:7180/fv/publish?version=$version&name=$name&notes=$enc_notes'; rc=\$?; rm -f staging-$name; echo; exit \$rc"
else
  [ -n "$sig" ] || sig=$(sign_fields mac "$version" "$sha" "$size")
  ok "signature (mac, $version): ${sig:0:16}..."
  step "upload the Mac zip (platform=mac) - the relay writes the mac_* fields"
  confirm "Publishing $name $version (sha ${sha:0:16}...) for macOS clients of the $brand brand. Continue?"
  put relay "$file" "$RELAY_DIR/staging-$name"
  mx relay "feed backup + signed mac publish (win fields untouched)" "cd $RELAY_DIR && cp -p dist/$feed dist/$feed.pre-mac-$version && TOK=\$(cat upload.token) && curl -fsS -m600 -X POST -H \"Authorization: Bearer \$TOK\" -H 'X-Fv-Sig: $sig' --data-binary @staging-$name 'http://127.0.0.1:7180/fv/publish?platform=mac&version=$version&name=$name&notes=macOS'; rc=\$?; rm -f staging-$name; echo; exit \$rc"
fi

# ---- state of the other platform (information only; nothing is re-signed any more) ----
step "Mac state in $feed"
fj=$(rx relay "cat $RELAY_DIR/dist/$feed" || true)
mstate=$(python3 -c 'import sys,json
try:
    d=json.loads(sys.stdin.read())
except Exception:
    d={}
print((d.get("mac_version") or "none")+" "+("signed" if d.get("mac_sig") else "UNSIGNED"))' <<<"$fj")
note "feed: version $(python3 -c 'import sys,json;print(json.loads(sys.stdin.read() or "{}").get("version","?"))' <<<"$fj" 2>/dev/null || echo '?'), mac_version/mac_sig: $mstate"
case "$mstate" in
  none*) note "no Mac package in the feed yet - publish one with 55-mac-pull.sh" ;;
  *UNSIGNED) note "Mac package is unsigned: fine while the relay is in compat; publish a signed one (55-mac-pull.sh) BEFORE 60-require-sig / phase 3" ;;
esac

if ((APPLY)); then
  step "verify the live feed"
  tmpf=$(mktemp)
  trap 'rm -f "$tmpf"' EXIT
  rx relay "cat $RELAY_DIR/dist/$feed" >"$tmpf"
  python3 -I "$SIGN" verify --feed "$tmpf" --pub "$pub" && ok "feed signatures verify against the compiled-in public key" || bad "FEED SIGNATURE DOES NOT VERIFY - roll back the feed: cp dist/$feed.pre-$version dist/$feed"
  pubfeed=$(curl -fsS -m10 "https://$([ "$brand" = freeviewer ] && echo freeviewer || echo remote).fleitec.com/fv/version" || true)
  [ -n "$pubfeed" ] && ok "public feed reachable" || note "public feed not reachable from here (check by hand)"
fi
bkver="$version"; [ "$platform" = mac ] && bkver="mac-$version"
note "Undo: 90-rollback.sh --feed $bkver ($feed.pre-$bkver). Clients only move forward: to undo for real, publish the old build under a HIGHER version."
finish
