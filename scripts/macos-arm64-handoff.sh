#!/usr/bin/env bash
# Apple Silicon acceptance handoff.
# Every command forces CODEX_HOME, HOME, and the app-data directory under
# this working copy and exits if that isolation is not in effect.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BUNDLE_ID="io.github.zacktools.modelbridge"
HANDOFF_ROOT="${HANDOFF_ROOT:-$ROOT/.handoff}"

die() {
  echo "macos-arm64-handoff: $*" >&2
  exit 1
}

real_path() {
  python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$1"
}

require_darwin_arm64() {
  local os arch
  os=$(uname -s)
  arch=$(uname -m)
  if [[ "$os" != "Darwin" || "$arch" != "arm64" ]]; then
    die "this handoff runs only on Darwin arm64 (found ${os} ${arch})"
  fi
}

rustc_at_least() {
  local need="$1"
  local found
  found=$(rustc --version | awk '{print $2}')
  RV="$found" python3 - "$need" <<'PY'
import os, sys
found = tuple(int(part) for part in os.environ["RV"].split(".")[:3])
need = tuple(int(part) for part in sys.argv[1].split("."))
sys.exit(0 if found >= need else 1)
PY
}

assert_isolation() {
  : "${REAL_HOME:?REAL_HOME is unset}"
  : "${HANDOFF_ROOT:?HANDOFF_ROOT is unset}"
  : "${HOME:?HOME is unset}"
  : "${CODEX_HOME:?CODEX_HOME is unset}"
  : "${APP_AGG_DATA_DIR:?APP_AGG_DATA_DIR is unset}"
  : "${APP_AGG_CORE_DIR:?APP_AGG_CORE_DIR is unset}"
  local root real name path resolved
  local -a forbidden
  root=$(real_path "$HANDOFF_ROOT")
  real=$(real_path "$REAL_HOME")
  if [[ "$HOME" == "$real" || "$(real_path "$HOME")" == "$real" ]]; then
    die "isolation is not in effect: HOME is the real home"
  fi
  forbidden=(
    "$real/.codex"
    "$real/.cli-proxy-api"
    "$real/Library/Application Support/$BUNDLE_ID"
  )
  for name in HOME CODEX_HOME APP_AGG_DATA_DIR APP_AGG_CORE_DIR; do
    path="${!name}"
    [[ -n "$path" ]] || die "isolation is not in effect: $name is empty"
    [[ -d "$path" ]] || die "isolation is not in effect: $name does not exist ($path)"
    resolved=$(real_path "$path")
    case "$resolved" in
      "$root"|"$root"/*) ;;
      *) die "isolation is not in effect: $name is outside the handoff root ($resolved)" ;;
    esac
    for bad in "${forbidden[@]}"; do
      if [[ "$resolved" == "$bad" || "$resolved" == "$bad"/* ]]; then
        die "isolation is not in effect: $name points at $bad"
      fi
    done
  done
  if [[ "$(real_path "$CODEX_HOME")" == "$(real_path "$real/.codex")" ]]; then
    die "isolation is not in effect: CODEX_HOME is the real ~/.codex"
  fi
}

apply_isolation() {
  REAL_HOME="${REAL_HOME:-${HOME:?cannot see the real HOME}}"
  HANDOFF_ROOT="$ROOT/.handoff"
  mkdir -p \
    "$HANDOFF_ROOT/home" \
    "$HANDOFF_ROOT/codex-home" \
    "$HANDOFF_ROOT/app-data" \
    "$HANDOFF_ROOT/core" \
    "$HANDOFF_ROOT/home/.config" \
    "$HANDOFF_ROOT/home/.local/share" \
    "$HANDOFF_ROOT/home/.cache"
  chmod 700 "$HANDOFF_ROOT" "$HANDOFF_ROOT/home" "$HANDOFF_ROOT/codex-home" "$HANDOFF_ROOT/app-data" "$HANDOFF_ROOT/core"
  export APP_AGG_HANDOFF_ROOT="$HANDOFF_ROOT"
  export APP_AGG_REAL_HOME="$REAL_HOME"
  export APP_AGG_REQUIRE_ISOLATION=1
  export HOME="$HANDOFF_ROOT/home"
  export CODEX_HOME="$HANDOFF_ROOT/codex-home"
  export APP_AGG_DATA_DIR="$HANDOFF_ROOT/app-data"
  export APP_AGG_CORE_DIR="$HANDOFF_ROOT/core"
  export USERPROFILE="$HANDOFF_ROOT/home"
  export XDG_CONFIG_HOME="$HANDOFF_ROOT/home/.config"
  export XDG_DATA_HOME="$HANDOFF_ROOT/home/.local/share"
  export XDG_CACHE_HOME="$HANDOFF_ROOT/home/.cache"
  unset APPDATA || true
  assert_isolation
}

agg_poc() {
  local bin="$ROOT/target/debug/agg-poc"
  [[ -x "$bin" ]] || die "missing $bin; run: $0 build"
  "$bin" "$@"
}

desktop_bin() {
  echo "$ROOT/apps/desktop/src-tauri/target/debug/model-bridge-desktop"
}

usage() {
  cat <<EOF
usage: scripts/macos-arm64-handoff.sh <command>

  prepare
  build
  window
  isolate
  core
  oauth codex|antigravity [--accept-file-tokens] [--open]
  codex-preview [--model <id>]
  codex-apply --model <id>
  codex-rollback
  stop

Isolation is under \$REPO/.handoff and is checked before every command.
EOF
}

cmd_prepare() {
  require_darwin_arm64
  command -v rustc >/dev/null || die "rustc is not installed"
  command -v python3 >/dev/null || die "python3 is not installed"
  if ! rustc_at_least 1.88; then
    die "rustc >= 1.88 is required (Cargo.toml rust-version). Found: $(rustc --version)"
  fi
  if ! rustc_at_least 1.90; then
    die "the desktop shell needs rustc >= 1.90 because tauri 2.12.5 declares rust-version 1.90. Found: $(rustc --version). Install with: rustup toolchain install stable"
  fi
  apply_isolation
  echo "isolation root: $HANDOFF_ROOT"
  echo "rustc: $(rustc --version)"
  echo "next: $0 core && $0 build && $0 isolate && $0 window"
}

cmd_build() {
  require_darwin_arm64
  if ! rustc_at_least 1.90; then
    die "build needs rustc >= 1.90 (tauri 2.12.5 rust-version). Found: $(rustc --version)"
  fi
  apply_isolation
  cargo build -p agg-core --bin agg-poc
  cargo build --manifest-path "$ROOT/apps/desktop/src-tauri/Cargo.toml"
}

cmd_window() {
  require_darwin_arm64
  apply_isolation
  local bin
  bin=$(desktop_bin)
  [[ -x "$bin" ]] || die "missing $bin; run: $0 build"
  export APP_AGG_OPEN_UI=1
  exec "$bin"
}

cmd_isolate() {
  require_darwin_arm64
  apply_isolation
  cat <<EOF
HOME=$HOME
CODEX_HOME=$CODEX_HOME
APP_AGG_DATA_DIR=$APP_AGG_DATA_DIR
APP_AGG_CORE_DIR=$APP_AGG_CORE_DIR
APP_AGG_HANDOFF_ROOT=$APP_AGG_HANDOFF_ROOT
APP_AGG_REAL_HOME=$APP_AGG_REAL_HOME
untouched:
  $APP_AGG_REAL_HOME/.codex
  $APP_AGG_REAL_HOME/.cli-proxy-api
  $APP_AGG_REAL_HOME/Library/Application Support/$BUNDLE_ID
EOF
}

cmd_core() {
  require_darwin_arm64
  apply_isolation
  "$ROOT/scripts/fetch-core.sh" --target darwin-aarch64 --cache-dir "$APP_AGG_CORE_DIR"
}

read_gateway_key() {
  python3 - "$APP_AGG_DATA_DIR/local_keys.json" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
if not path.is_file():
    sys.exit("local keys are missing; start the window and the gateway once")
print(json.loads(path.read_text())["gateway_key"], end="")
PY
}

cmd_oauth() {
  require_darwin_arm64
  apply_isolation
  local provider="${1:-}"
  shift || true
  case "$provider" in
    codex|antigravity) ;;
    *) die "usage: $0 oauth codex|antigravity [--accept-file-tokens] [--open]" ;;
  esac
  agg_poc oauth-begin --provider "$provider" "$@"
}

cmd_codex_preview() {
  require_darwin_arm64
  apply_isolation
  local key
  key=$(read_gateway_key)
  agg_poc preview --key "$key" --base-url "http://127.0.0.1:${APP_AGG_PORT:-50833}/v1" "$@"
}

cmd_codex_apply() {
  require_darwin_arm64
  apply_isolation
  local key
  key=$(read_gateway_key)
  agg_poc apply --key "$key" --base-url "http://127.0.0.1:${APP_AGG_PORT:-50833}/v1" "$@"
}

cmd_codex_rollback() {
  require_darwin_arm64
  apply_isolation
  agg_poc rollback "$@"
}

cmd_stop() {
  require_darwin_arm64
  apply_isolation
  agg_poc stop
}

main() {
  local cmd="${1:-}"
  if [[ -z "$cmd" ]]; then
    usage
    exit 2
  fi
  shift
  case "$cmd" in
    prepare) cmd_prepare "$@" ;;
    build) cmd_build "$@" ;;
    window) cmd_window "$@" ;;
    isolate) cmd_isolate "$@" ;;
    core) cmd_core "$@" ;;
    oauth) cmd_oauth "$@" ;;
    codex-preview) cmd_codex_preview "$@" ;;
    codex-apply) cmd_codex_apply "$@" ;;
    codex-rollback) cmd_codex_rollback "$@" ;;
    stop) cmd_stop "$@" ;;
    *) usage; die "unknown command $cmd" ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
