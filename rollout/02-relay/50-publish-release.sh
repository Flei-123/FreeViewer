#!/usr/bin/env bash
# Block 2 / step 50 - publish a SIGNED release (the first one = signature phase 1 -> 2).
#
# Runs where the update private key is (or pass --sig), uploads over ssh to CT110
# and talks to the relay on 127.0.0.1 (= "from the LAN", works in strict mode).
#
#   win : uploads the exe with header X-Fv-Sig (the relay verifies it and writes "sig" into the feed),
#         then RE-SIGNS the Mac fields of the same feed (see "why" below).
#   mac : uploads the zip with noversion=1 (the feed stays), then writes mac_version/mac_sha256/
#         mac_size/mac_sig into the feed itself - the relay has no code path for Mac fields.
#
# WHY the Mac re-sign: the client signs/verifies with the feed field "version" for BOTH
# platforms (src/update.rs). A new Windows version therefore invalidates the old mac_sig.
# Without the re-sign, signed Mac clients would refuse every update after a Windows publish.
# KNOWN QUIRK (roadmap r28): the feed carries mac_version 0.26.10 but version 0.29.4 - a Mac
# client compares against "version" and fetches mac_url. Publish Windows and Mac together.
#
# Usage: 50-publish-release.sh --platform win|mac --file PATH --version X.Y.Z
#          [--brand freeviewer|x-remote] [--notes TEXT] [--sig HEX] [--no-mac-resign] [--apply]
# Env:   FV_SIGN_KEY (default /root/fvwork/keys/update-ed25519.key)
. "$(dirname "$0")/../lib/common.sh"
. "$(dirname "$0")/lib-relay.sh"
usage() { sed -n '2,22p' "$0"; }
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
  step "upload the Mac zip (noversion=1) and write the Mac fields"
  confirm "Publishing $name $version for macOS clients. Continue?"
  put relay "$file" "$RELAY_DIR/staging-$name"
  mx relay "signed-less zip upload (noversion), feed untouched" "cd $RELAY_DIR && TOK=\$(cat upload.token) && curl -fsS -m600 -X POST -H \"Authorization: Bearer \$TOK\" --data-binary @staging-$name 'http://127.0.0.1:7180/fv/publish?version=$version&name=$name&noversion=1&notes=macOS'; rc=\$?; rm -f staging-$name; echo; exit \$rc"
fi

# ---- (re-)sign the Mac fields of the feed with the feed's CURRENT version ----
resign_mac() {
  local fj fver msha msize msig mver
  fj=$(rx relay "cat $RELAY_DIR/dist/$feed")
  # after a (dry-run) win publish the feed still has the OLD version; use the new one in that case
  fver=$(python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("version",""))' <<<"$fj")
  [ "$platform" = win ] && fver="$version"
  msha=$(python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("mac_sha256",""))' <<<"$fj")
  msize=$(python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("mac_size",0))' <<<"$fj")
  mver=$(python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("mac_version",""))' <<<"$fj")
  if [ "$platform" = mac ]; then msha="$sha"; msize="$size"; mver="$version"; fi
  re_hex64='^[0-9a-f]{64}$'
  if ! [[ "$msha" =~ $re_hex64 ]]; then note "no Mac package in $feed - nothing to sign"; return 0; fi
  if [ -r "$KEY" ]; then
    msig=$(sign_fields mac "$fver" "$msha" "$msize")
  elif [ -n "$(fv_opt mac-sig)" ]; then
    msig=$(fv_opt mac-sig)
  else
    warn "no signing key here - give --mac-sig HEX (made with sign-fields --platform mac --version $fver --sha256 $msha --size $msize)"
    return 0
  fi
  ok "mac signature over version $fver, sha ${msha:0:16}...: ${msig:0:16}..."
  mx relay "write mac fields into $feed (atomic, backup .pre-mac)" "cd $RELAY_DIR/dist && cp -p $feed $feed.pre-mac-$(stamp) && python3 - <<'PYEOF'
import json, os
p = '$feed'
d = json.load(open(p))
d['mac_version'] = '$mver'
d['mac_sha256'] = '$msha'
d['mac_size'] = $msize
d['mac_sig'] = '$msig'
json.dump(d, open(p + '.tmp', 'w'), indent=1)
os.replace(p + '.tmp', p)
print('mac fields written')
PYEOF"
}
if [ "$platform" = mac ] || { [ "$platform" = win ] && ! fv_flag no-mac-resign; }; then
  step "Mac fields"
  resign_mac
fi

if ((APPLY)); then
  step "verify the live feed"
  tmpf=$(mktemp)
  trap 'rm -f "$tmpf"' EXIT
  rx relay "cat $RELAY_DIR/dist/$feed" >"$tmpf"
  python3 -I "$SIGN" verify --feed "$tmpf" --pub "$pub" && ok "feed signatures verify against the compiled-in public key" || bad "FEED SIGNATURE DOES NOT VERIFY - roll back the feed: cp dist/$feed.pre-$version dist/$feed"
  pubfeed=$(curl -fsS -m10 "https://$([ "$brand" = freeviewer ] && echo freeviewer || echo remote).fleitec.com/fv/version" || true)
  [ -n "$pubfeed" ] && ok "public feed reachable" || note "public feed not reachable from here (check by hand)"
fi
note "Undo: 90-rollback.sh --feed ($feed.pre-$version). Clients only move forward: to undo for real, publish the old build under a HIGHER version."
finish
