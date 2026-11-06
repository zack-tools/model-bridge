#!/bin/sh
# Download one pinned CLIProxyAPI archive and verify it against vendor/cliproxyapi/checksums.txt.
# This script does not install OS packages and does not start the binary.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
exec python3 - "$ROOT" "$@" << 'PY'
import hashlib, pathlib, sys, tarfile, urllib.request, zipfile

root = pathlib.Path(sys.argv[1])
arch = {"x86_64": "linux_amd64", "aarch64": "linux_aarch64", "arm64": "linux_aarch64"}.get(
    __import__("os").uname().machine
)
if arch is None:
    sys.exit(f"no additional-evaluation asset mapped for this machine")
name = f"CLIProxyAPI_8.0.57_{arch}.tar.gz"
checksums = (root / "vendor/cliproxyapi/checksums.txt").read_text().splitlines()
expected = next(line.split()[0] for line in checksums if line.split()[1] == name)
cache = root / ".cache" / "cliproxyapi"
cache.mkdir(parents=True, exist_ok=True)
dest = cache / name
if not dest.exists() or hashlib.sha256(dest.read_bytes()).hexdigest() != expected:
    url = f"https://github.com/router-for-me/CLIProxyAPI/releases/download/v8.0.57/{name}"
    urllib.request.urlretrieve(url, dest)
actual = hashlib.sha256(dest.read_bytes()).hexdigest()
if actual != expected:
    sys.exit(f"checksum mismatch for {name}: {actual} != {expected}")
binary = cache / "cli-proxy-api"
with tarfile.open(dest, "r:gz") as archive:
    member = archive.getmember("cli-proxy-api")
    extracted = archive.extractfile(member)
    binary.write_bytes(extracted.read())
binary.chmod(0o755)
print(dest)
print(binary)
PY
