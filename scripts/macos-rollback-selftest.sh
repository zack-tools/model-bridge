#!/usr/bin/env bash
# Offline rollback check on an isolated CODEX_HOME under this repo's .handoff.
# Invoke with bash from any working directory. Does not start the gateway or OAuth.
if [ -z "${BASH_VERSION-}" ]; then
  echo "macos-rollback-selftest.sh: this file must run under bash." >&2
  return 1 2>/dev/null || exit 1
fi
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=macos-arm64-handoff.sh
source "$SCRIPT_DIR/macos-arm64-handoff.sh"

KEY="selftest-gateway-key"
BASE_URL="http://127.0.0.1:50833/v1"
failed=0

mtime_of() {
  if [ ! -e "$1" ]; then
    echo missing
    return
  fi
  if stat -c %Y "$1" >/dev/null 2>&1; then
    stat -c %Y "$1"
  else
    stat -f %m "$1"
  fi
}

snapshot_real_paths() {
  local path
  for path in \
    "$REAL_HOME/.codex" \
    "$REAL_HOME/.cli-proxy-api" \
    "$REAL_HOME/Library/Application Support/$BUNDLE_ID"
  do
    printf '%s %s\n' "$(mtime_of "$path")" "$path"
  done
}

refuse_if_codex_home_escapes() {
  local handoff resolved
  handoff=$(real_path "$ROOT/.handoff")
  resolved=$(real_path "$CODEX_HOME")
  case "$resolved" in
    "$handoff"|"$handoff"/*) ;;
    *)
      echo "FAIL isolation: CODEX_HOME resolves outside .handoff ($resolved)" >&2
      exit 1
      ;;
  esac
  assert_isolation
}

ensure_agg_poc() {
  AGG="$ROOT/target/debug/agg-poc"
  if [[ ! -x "$AGG" ]]; then
    cargo build -p agg-core --bin agg-poc --manifest-path "$ROOT/Cargo.toml"
  fi
  [[ -x "$AGG" ]] || die "agg-poc binary is missing at $AGG"
}

apply_model() {
  local home="$1" model="$2"
  "$AGG" apply \
    --config "$home/config.toml" \
    --key "$KEY" \
    --base-url "$BASE_URL" \
    --model "$model"
}

use_home() {
  local home="$1"
  rm -rf "$home"
  mkdir -p "$home"
  export CODEX_HOME="$home"
  refuse_if_codex_home_escapes
}

config_has_only_foreign_table() {
  local file="$1"
  python3 - "$file" <<'PY'
import pathlib, sys
text = pathlib.Path(sys.argv[1]).read_text()
if 'trust_level = "trusted"' not in text:
    sys.exit("missing trust_level")
if '[projects."/tmp/x"]' not in text and "[projects.'/tmp/x']" not in text:
    sys.exit("missing projects table")
for line in text.splitlines():
    stripped = line.strip()
    if stripped.startswith(("model_provider", "model_catalog_json", "model ", "model=", "[model_providers")):
        sys.exit(f"managed key remains: {stripped}")
PY
}

run_case() {
  local name="$1"
  shift
  if "$@"; then
    echo "PASS $name"
  else
    echo "FAIL $name" >&2
    failed=1
  fi
}

keep_foreign_table() {
  local home="$SCRATCH/keep"
  use_home "$home"
  apply_model "$home" gemini-3-flash
  apply_model "$home" gpt-6-sol
  printf '\n[projects."/tmp/x"]\ntrust_level = "trusted"\n' >> "$home/config.toml"
  "$AGG" rollback --config "$home/config.toml"
  [[ -f "$home/config.toml" ]] || {
    echo "config.toml was deleted" >&2
    return 1
  }
  config_has_only_foreign_table "$home/config.toml"
}

remove_created_file() {
  local home="$SCRATCH/remove"
  use_home "$home"
  apply_model "$home" gemini-3-flash
  apply_model "$home" gpt-6-sol
  "$AGG" rollback --config "$home/config.toml"
  [[ ! -e "$home/config.toml" ]]
}

real_paths_unchanged() {
  local now
  now=$(snapshot_real_paths)
  if [[ "$now" == "$BEFORE" ]]; then
    return 0
  fi
  echo "FAIL isolation: a real Codex or app-support path changed" >&2
  echo "before:" >&2
  echo "$BEFORE" >&2
  echo "after:" >&2
  echo "$now" >&2
  return 1
}

ensure_agg_poc
apply_isolation
SCRATCH=$(mktemp -d "$ROOT/.handoff/rollback-selftest.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
BEFORE=$(snapshot_real_paths)

run_case keep-foreign-table keep_foreign_table
run_case remove-created-file remove_created_file
if real_paths_unchanged; then
  echo "PASS real-paths-unchanged"
else
  echo "FAIL real-paths-unchanged" >&2
  failed=1
fi

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
exit 0
