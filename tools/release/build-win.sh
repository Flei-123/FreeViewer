#!/bin/bash
# Cross-build the Windows exe on Linux (x86_64-pc-windows-gnu), release profile.
# usage: tools/release/build-win.sh fv|xr [OUT_DIR]   (run through /root/jarvis/bin/heavy, ~8 min)
#   fv -> OUT_DIR/freeviewer.exe          (brand FreeViewer, feed /fv/version)
#   xr -> OUT_DIR/x-remote.exe            (brand X-Remote/Xoffi, --features license, feed version-xoffi.json)
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64, static Windows opus in /opt/opus-win
# (see BUILD-WINDOWS-AUF-LINUX.md). No icon (no rc.exe), no Authenticode signature (r11).
# Afterwards sign for the updater: tools/sign-release/sign_release.py sign-fields ... (docs/UPDATE-SIGNING.md)
set -euo pipefail
B=${1:?fv|xr}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${2:-$ROOT/dist}
mkdir -p "$OUT"
D=$(mktemp -d /dev/shm/fv-build.XXXX)
trap 'rm -rf "$D"' EXIT
export CARGO_TARGET_DIR=$D PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=/opt/opus-win/lib/pkgconfig
cd "$ROOT"
if [ "$B" = xr ]; then
  export FV_BRAND_NAME=X-Remote FV_BRAND_EXE=x-remote.exe FV_BRAND_DIR=X-Remote FV_BRAND_PUBLISHER=Xoffi \
    FV_BRAND_SLUG=xoffi FV_BRAND_SCHEME=xremote FV_BRAND_WEB=https://remote.fleitec.com \
    FV_BRAND_FEED=https://remote.fleitec.com/fv/dist/version-xoffi.json
  cargo build --locked --release --target x86_64-pc-windows-gnu --bin freeviewer --features license
  cp "$D/x86_64-pc-windows-gnu/release/freeviewer.exe" "$OUT/x-remote.exe"
else
  cargo build --locked --release --target x86_64-pc-windows-gnu --bin freeviewer
  cp "$D/x86_64-pc-windows-gnu/release/freeviewer.exe" "$OUT/freeviewer.exe"
fi
echo "BUILD_OK $B -> $OUT"
