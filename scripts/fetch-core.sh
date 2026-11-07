#!/bin/sh
# Compatibility entrypoint for macOS and Linux; Windows can run fetch-core.py directly.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
exec python3 "$ROOT/scripts/fetch-core.py" "$@"
