#!/usr/bin/env bash
# ci.sh -- everything that must be green before a merge to main.
#   ./ci.sh          no-German guard + action manifest + FreeMeet E2E vectors + Fleitec-ID adapter + cargo test
#   CI_QUICK=1 ./ci.sh   only the two cheap checks (no full cargo test)
# Heavy (cargo): run it through the server wrapper: /root/jarvis/bin/heavy ./ci.sh
set -euo pipefail
cd "$(dirname "$0")"
step() { printf '\n== %s\n' "$*"; }
step "action manifest (lint + flags exist in src/main.rs)"; python3 tools/check_actions.py
step "no German in code (shared guard of Firn, frozen counts)"; bash "${FIRN:-/root/firn}/tools/english/no_german_ci.sh" .
step "FreeMeet E2E vectors (Python reference + browser e2e.js)"; python3 tools/freemeet-e2e/ref.py --check; if command -v node >/dev/null; then node --test web-patch/ >/dev/null; else echo "node missing: browser e2e test skipped"; fi
step "Fleitec-ID vectors against src/account.rs";          cargo test --manifest-path tools/fleitec-id-adapter/Cargo.toml
if [ "${CI_QUICK:-0}" != 1 ]; then step "cargo test"; cargo test; fi
echo; echo "ci.sh: all green"
