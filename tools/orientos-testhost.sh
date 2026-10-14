#!/usr/bin/env bash
# orientos-testhost.sh -- local FreeViewer test host for the OrientOS viewer.
#
# Starts a LOCAL relay (relay/relay.js, plain ws://) and the Rust host in
# synthetic test mode (FV_TESTHOST=1: fixed 640x480 picture, input is only
# logged, no display needed). Prints ID + password, then keeps running until
# Ctrl-C. Everything (config, relay data, logs) lives in one mktemp directory
# that is removed on exit.
#
#   tools/orientos-testhost.sh [--port N] [--password PW] [--bin PATH] [--selftest]
#
#   --port N       relay port (default 17180)
#   --password PW  session password (default: testpass123)
#   --bin PATH     freeviewer binary (default: $FV_BIN, else $CARGO_TARGET_DIR/debug/freeviewer,
#                  else ./target/debug/freeviewer)
#   --selftest     run the Rust viewer twice (password mode, ask mode), print the result, exit
#
# From a QEMU guest (user networking) the host machine is 10.0.2.2:
#   relay URL  ws://10.0.2.2:PORT/fv/ws     (the viewer must allow plain ws:// for this)
#   handshake  password mode with the printed ID + password, or ask mode (the test
#              host auto-confirms, FV_AUTOCONFIRM=1) -- 4-digit session code is shown on both sides.
# Input sent by the viewer shows up in the host log lines "TESTHOST input: ..."
# (also in $D/input.log, printed at exit with --selftest).
set -euo pipefail

PORT=17180
PW=testpass123
SELFTEST=0
BIN="${FV_BIN:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --password) PW="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --selftest) SELFTEST=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ -z "$BIN" ]; then
  if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -x "$CARGO_TARGET_DIR/debug/freeviewer" ]; then
    BIN="$CARGO_TARGET_DIR/debug/freeviewer"
  else
    BIN="$ROOT/target/debug/freeviewer"
  fi
fi
[ -x "$BIN" ] || { echo "binary not found: $BIN (build it via /root/jarvis/bin/heavy cargo build, or pass --bin)" >&2; exit 1; }
command -v node >/dev/null || { echo "node is required for the relay" >&2; exit 1; }

D="$(mktemp -d)"
PIDS=()
cleanup() {
  for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$D"
}
trap cleanup EXIT INT TERM

mkdir -p "$D/home"
export HOME="$D/home"
unset APPDATA ProgramData XDG_CONFIG_HOME || true

# 1. local relay
# copied as .cjs: a package.json with "type": "module" in a parent directory would
# otherwise make node treat relay.js as an ES module. Needs the 'ws' package
# (resolved via NODE_PATH or the JARVIS fallback inside relay.js).
cp "$ROOT/relay/relay.js" "$D/relay.cjs"
for m in clientip.js reqauth.js; do cp "$ROOT/relay/$m" "$D/$m"; done
FV_PORT="$PORT" FV_DATA="$D/hosts.json" node "$D/relay.cjs" >"$D/relay.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$PORT/fv/health" >/dev/null 2>&1 && break
  sleep 0.1
done
curl -fsS "http://127.0.0.1:$PORT/fv/health" >/dev/null || { echo "relay did not start:" >&2; cat "$D/relay.log" >&2; exit 1; }

# 2. synthetic test host
RELAY_URL="ws://127.0.0.1:$PORT/fv/ws"
export FV_RELAY="$RELAY_URL" FV_ALLOW_INSECURE_RELAY=1
FV_TESTHOST=1 FV_AUTOCONFIRM=1 FV_PASSWORD="$PW" FV_TESTHOST_LOG="$D/input.log" \
  "$BIN" --headless >"$D/host.log" 2>&1 &
PIDS+=($!)

ID=""
for _ in $(seq 1 100); do
  ID="$(grep -oE '^ID [0-9]{9,10}' "$D/host.log" | head -1 | cut -d' ' -f2 || true)"
  [ -n "$ID" ] && break
  sleep 0.2
done
[ -n "$ID" ] || { echo "host did not register:" >&2; cat "$D/host.log" >&2; exit 1; }

echo "== FreeViewer test host (synthetic 640x480, JPEG only) =="
echo "ID          $ID"
echo "password    $PW"
echo "relay local ws://127.0.0.1:$PORT/fv/ws"
echo "relay QEMU  ws://10.0.2.2:$PORT/fv/ws"
echo "ask mode    works (auto-confirmed by the test host)"

if [ "$SELFTEST" = 1 ]; then
  rc=0
  # the Rust viewer prints ALSA noise on machines without a sound card
  viewer() { FV_TESTHOST=0 timeout 60 "$BIN" "$@" 2> >(grep -v '^ALSA lib' >&2); }
  echo "-- selftest: password mode"
  viewer --connect "$ID" "$PW" 3 || rc=1
  echo "-- selftest: wrong password (must fail)"
  if viewer --connect "$ID" "wrong-password" 3 >"$D/wrong.log" 2>&1; then
    echo "FAIL: wrong password was accepted"; rc=1
  else
    tail -1 "$D/wrong.log"
  fi
  sleep 1
  echo "-- selftest: ask mode"
  viewer --ask "$ID" 3 || rc=1
  sleep 1
  echo "-- selftest: input (mouse, keys, clipboard; takes ~20 s)"
  viewer --inputtest "$ID" "$PW" | grep -E "^OK|FAIL|FEHLER" || true
  for pat in 'MouseMove x=5000 y=5000' 'MouseDelta dx=10 dy=0' 'KeyVk vk=144' 'Special code=6' 'Clipboard len=.* text=FV-CLIP'; do
    if grep -qE "$pat" "$D/input.log" 2>/dev/null; then echo "input logged: $pat"; else echo "FAIL: input not logged: $pat"; rc=1; fi
  done
  [ "$rc" = 0 ] && echo "SELFTEST OK" || echo "SELFTEST FAILED"
  exit "$rc"
fi

echo "running -- Ctrl-C to stop (host log: lines 'TESTHOST input: ...')"
tail -n +1 -f "$D/host.log" &
PIDS+=($!)
wait "${PIDS[1]}"
