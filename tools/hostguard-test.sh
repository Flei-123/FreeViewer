#!/usr/bin/env bash
# hostguard-test.sh -- live check of two host/viewer hardening features against
# a LOCAL relay + the synthetic test host (FV_TESTHOST=1, see orientos-testhost.sh):
#   1. brute-force guard: repeated wrong passwords lock the sign-in, a correct
#      password is refused while locked (r4)
#   2. viewer auto-reconnect: relay dies and comes back, the viewer session
#      resumes by itself (r25)
#
#   tools/hostguard-test.sh [--bin PATH] [--port N]   (default port: a free one)
#
# Everything lives in one mktemp directory that is removed on exit.
set -uo pipefail

PORT=""
BIN="${FV_BIN:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# default: a free port picked by the kernel (a fixed port can collide with another test run on the same machine)
if [ -z "$PORT" ]; then
  PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])' 2>/dev/null || echo 17190)"
fi
[ -n "$BIN" ] || BIN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/freeviewer"
[ -x "$BIN" ] || { echo "binary not found: $BIN" >&2; exit 1; }

D="$(mktemp -d)"
PIDS=()
cleanup() {
  for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$D"
}
trap cleanup EXIT INT TERM

PW=guardtest-pw-1
mkdir -p "$D/home" "$D/vhome"
cp "$ROOT/relay/relay.js" "$D/relay.cjs"
cp "$ROOT/relay/clientip.js" "$D/clientip.js"

start_relay() {
  FV_PORT="$PORT" FV_DATA="$D/hosts.json" node "$D/relay.cjs" >>"$D/relay.log" 2>&1 &
  RELAY_PID=$!
  PIDS+=("$RELAY_PID")
  for _ in $(seq 1 50); do
    curl -fsS "http://127.0.0.1:$PORT/fv/health" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  echo "relay did not start" >&2; return 1
}

export FV_RELAY="ws://127.0.0.1:$PORT/fv/ws" FV_ALLOW_INSECURE_RELAY=1
start_relay || exit 1

HOME="$D/home" FV_TESTHOST=1 FV_AUTOCONFIRM=1 FV_PASSWORD="$PW" "$BIN" --headless >"$D/host.log" 2>&1 &
PIDS+=($!)
ID=""
for _ in $(seq 1 100); do
  ID="$(grep -oE '^ID [0-9]{9}' "$D/host.log" | head -1 | cut -d' ' -f2 || true)"
  [ -n "$ID" ] && break
  sleep 0.2
done
[ -n "$ID" ] || { echo "host did not register"; cat "$D/host.log"; exit 1; }
echo "host id $ID"

viewer() { HOME="$D/vhome" FV_TESTHOST=0 timeout "${VT:-60}" "$BIN" "$@" 2> >(grep -v '^ALSA lib' >&2); }
rc=0

# ---------------------------------------------------------------- 1. guard
echo "-- guard: correct password works first"
viewer --connect "$ID" "$PW" 2 >"$D/ok1.log" 2>&1 && echo "ok" || { echo "FAIL: first login"; tail -2 "$D/ok1.log"; rc=1; }

echo "-- guard: 5 wrong passwords"
for i in 1 2 3 4 5; do
  viewer --connect "$ID" "wrong-password-$i" 2 >"$D/w$i.log" 2>&1 && { echo "FAIL: wrong pw accepted"; rc=1; }
  sleep 0.3
done
sleep 1
echo "-- guard: correct password must be refused while locked"
if viewer --connect "$ID" "$PW" 2 >"$D/locked.log" 2>&1; then
  echo "FAIL: login worked during lockout"; rc=1
else
  echo "ok (refused): $(tail -1 "$D/locked.log")"
fi
HLOG="$(find "$D/home" -name capture.log | head -1)"
if [ -n "$HLOG" ] && grep -q "too many wrong passwords" "$HLOG"; then
  echo "ok: host reports the lockout"
else
  echo "FAIL: host capture.log has no lockout message"; rc=1
fi

# ------------------------------------------------------------ 2. reconnect
# fresh host: the first one is locked now (30 s)
kill "${PIDS[1]}" 2>/dev/null || true
sleep 1
HOME="$D/home2" FV_TESTHOST=1 FV_AUTOCONFIRM=1 FV_PASSWORD="$PW" "$BIN" --headless >"$D/host2.log" 2>&1 &
PIDS+=($!)
ID2=""
for _ in $(seq 1 100); do
  ID2="$(grep -oE '^ID [0-9]{9}' "$D/host2.log" | head -1 | cut -d' ' -f2 || true)"
  [ -n "$ID2" ] && break
  sleep 0.2
done
[ -n "$ID2" ] || { echo "FAIL: second host did not register"; exit 1; }
echo "-- reconnect: viewer runs, relay is killed and restarted (host $ID2)"
VT=100 viewer --connect "$ID2" "$PW" 14 >"$D/rc.log" 2>&1 &
VPID=$!
sleep 6
kill "$RELAY_PID" 2>/dev/null || true
sleep 3
start_relay || exit 1
wait "$VPID"; vrc=$?
tail -2 "$D/rc.log"
[ "${KEEP_LOGS:-}" ] && { mkdir -p "$KEEP_LOGS"; cp -r "$D"/*.log "$D/vhome" "$KEEP_LOGS"/ 2>/dev/null || true; }
LOGF="$(find "$D/vhome" -name capture.log | head -1)"
if [ "$vrc" = 0 ] && [ -n "$LOGF" ] && grep -q "viewer ended" "$LOGF"; then
  echo "ok: viewer ended once and came back (see: $(grep -c 'viewer ended' "$LOGF") end events)"
else
  echo "FAIL: viewer did not resume (rc=$vrc)"; rc=1
fi

[ "$rc" = 0 ] && echo "HOSTGUARD TEST OK" || echo "HOSTGUARD TEST FAILED"
exit "$rc"
