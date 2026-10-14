# shellcheck shell=bash
# shellcheck disable=SC2034  # constants are used by the scripts that source this file
# Shared helpers for the FreeViewer rollout scripts. Source it, do not execute it.
#
# Rules every script follows:
#   * DRY-RUN is the default. Nothing on a live system changes without --apply.
#   * rx  = READ-ONLY remote command, always runs (also in dry-run).
#   * mx  = MUTATING remote command, only runs with --apply.
#   * put = MUTATING file copy, only runs with --apply.
#   * Every mutating remote command starts with ": FVMUT;" so tests can prove
#     that a dry-run never sent one.
set -euo pipefail

ROLLOUT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$ROLLOUT_ROOT/.." && pwd)"

# ---- targets (override with environment variables) ----
PVE_HOST="${PVE_HOST:-192.168.1.1}"      # Proxmox host (LAN); port 209 is only the public mapping
PVE_PORT="${PVE_PORT:-22}"
RELAY_HOST="${RELAY_HOST:-192.168.1.60}" # CT110 FreeViewer relay
FM_HOST="${FM_HOST:-192.168.1.61}"       # CT111 FreeMeet
SSH_USER="${SSH_USER:-root}"
FV_SSH="${FV_SSH:-ssh}"
FV_SCP="${FV_SCP:-scp}"
FV_SSH_OPTS="${FV_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=8}"
FV_STATE="${FV_STATE:-$HOME/.fv-rollout}"   # local notes (hashes, timestamps); never secrets

# ---- facts verified read-only on 14.10.2026 ----
RELAY_LIVE_SHA="7c348523f7e23f2d0775f6f7ce2059a9017b061d8f21c753ab70ef0c4f43b5e1"  # /root/fvrelay/relay.js
FM_E2E_LIVE_SHA="17a4227f10ed0b9c07cea7190ef63d1c066b94b15d03383af60f3fd542fb9b49" # /opt/freemeet/web/e2e.js
RELAY_DIR="/root/fvrelay"
RELAY_UNIT="fvrelay"
FM_DIR="/opt/freemeet"
FM_UNIT="freemeet"
DUMP_STORAGE="${DUMP_STORAGE:-wd-backup}"

APPLY=0
ASSUME_YES=0
EXTRA=()
FAILS=0

# ---- output ----
info() { printf '%s\n' "$*"; }
warn() { printf 'WARN: %s\n' "$*" >&2; }
die() { printf 'ABORT: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s ==\n' "$*"; }
ok() { printf '  [ OK ] %s\n' "$*"; }
bad() { printf '  [FAIL] %s\n' "$*"; FAILS=$((FAILS + 1)); }
note() { printf '  [INFO] %s\n' "$*"; }

# ---- arguments ----
fv_parse_args() {
  EXTRA=()
  local a
  for a in "$@"; do
    case "$a" in
      --apply) APPLY=1 ;;
      --dry-run) APPLY=0 ;;
      --yes | -y) ASSUME_YES=1 ;;
      -h | --help)
        if declare -F usage >/dev/null; then usage; else info "see rollout/README.md"; fi
        exit 0
        ;;
      *) EXTRA+=("$a") ;;
    esac
  done
}

# fv_opt NAME [DEFAULT]  ->  value of "--NAME value" or "--NAME=value" in EXTRA
fv_opt() {
  local name="$1" def="${2:-}" i
  for ((i = 0; i < ${#EXTRA[@]}; i++)); do
    case "${EXTRA[$i]}" in
      "--$name="*) printf '%s' "${EXTRA[$i]#--"$name"=}"; return 0 ;;
      "--$name") if ((i + 1 < ${#EXTRA[@]})); then printf '%s' "${EXTRA[$((i + 1))]}"; return 0; fi ;;
    esac
  done
  printf '%s' "$def"
}
# fv_flag NAME -> true if "--NAME" is present
fv_flag() {
  local a
  for a in "${EXTRA[@]+"${EXTRA[@]}"}"; do [ "$a" = "--$1" ] && return 0; done
  return 1
}

banner() {
  if ((APPLY)); then
    printf '### %s - MODE: APPLY (live changes!) ###\n' "$1"
  else
    printf '### %s - MODE: DRY-RUN (reads only, nothing is changed; add --apply to execute) ###\n' "$1"
  fi
}

# ---- remote execution ----
_target() {
  case "$1" in
    pve) T_HOST="$PVE_HOST"; T_PORT="$PVE_PORT" ;;
    relay) T_HOST="$RELAY_HOST"; T_PORT=22 ;;
    fm) T_HOST="$FM_HOST"; T_PORT=22 ;;
    *) die "unknown target '$1'" ;;
  esac
}
# rx TARGET COMMAND...   read-only, always executed
rx() {
  local t="$1"
  shift
  _target "$t"
  local -a ssh_cmd
  read -r -a ssh_cmd <<<"$FV_SSH $FV_SSH_OPTS"
  "${ssh_cmd[@]}" -p "$T_PORT" "$SSH_USER@$T_HOST" "$*"
}
# mx TARGET DESCRIPTION COMMAND...   mutating, only with --apply
mx() {
  local t="$1" desc="$2"
  shift 2
  if ((APPLY)); then
    info "  -> [$t] $desc"
    rx "$t" ": FVMUT; $*"
  else
    info "  -> [dry-run] would run on $t: $desc"
    printf '       %s\n' "$*" | head -n 6
  fi
}
# put TARGET LOCALFILE REMOTEPATH   mutating file copy
put() {
  local t="$1" src="$2" dst="$3"
  _target "$t"
  if ((APPLY)); then
    info "  -> [$t] copy $(basename "$src") -> $dst"
    local -a scp_cmd
    read -r -a scp_cmd <<<"$FV_SCP $FV_SSH_OPTS"
    "${scp_cmd[@]}" -P "$T_PORT" "$src" "$SSH_USER@$T_HOST:$dst"
  else
    info "  -> [dry-run] would copy $src -> $t:$dst"
  fi
}
# mlocal DESCRIPTION CMD...   mutating local command
mlocal() {
  local desc="$1"
  shift
  if ((APPLY)); then
    info "  -> [local] $desc"
    "$@"
  else
    info "  -> [dry-run] would run locally: $desc"
  fi
}

# ---- guards ----
confirm() {
  ((APPLY)) || return 0
  ((ASSUME_YES)) && return 0
  local ans
  printf '%s\nType "yes" to continue: ' "$1"
  read -r ans
  [ "$ans" = "yes" ] || die "not confirmed"
}
# gate: stop before mutating if a check failed (dry-run only warns)
gate() {
  if ((FAILS > 0)); then
    if ((APPLY)); then die "$FAILS check(s) failed - nothing was changed"; fi
    warn "$FAILS check(s) failed - --apply would ABORT here"
  fi
}
finish() {
  if ((FAILS > 0)); then
    printf '\nRESULT: %s check(s) FAILED\n' "$FAILS"
    exit 2
  fi
  printf '\nRESULT: all checks passed%s\n' "$( ((APPLY)) || printf ' (dry-run)')"
}
need_cmd() { command -v "$1" >/dev/null 2>&1 || die "local tool missing: $1"; }
sha_of() { sha256sum "$1" | cut -d' ' -f1; }
stamp() { date +%Y%m%d-%H%M%S; }

# the public key the clients trust (single source of truth: src/update.rs)
update_pubkey() {
  grep -o 'None => "[0-9a-f]\{64\}"' "$REPO_ROOT/src/update.rs" | grep -o '[0-9a-f]\{64\}' | head -n1
}
