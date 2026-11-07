#!/usr/bin/env python3
"""Download and extract one checksum-pinned CLIProxyAPI binary; never execute it."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VERSION = "8.0.57"


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=[f"{os_name}-{arch}" for os_name in ("darwin", "linux", "windows") for arch in ("amd64", "aarch64")])
    parser.add_argument("--cache-dir", type=Path, default=ROOT / ".cache" / "cliproxyapi")
    args = parser.parse_args()
    if args.target:
        system, arch = args.target.split("-", 1)
    else:
        system = {"Linux": "linux", "Darwin": "darwin", "Windows": "windows"}.get(platform.system())
        arch = {"x86_64": "amd64", "amd64": "amd64", "aarch64": "aarch64", "arm64": "aarch64"}.get(platform.machine().lower())
        if not system or not arch:
            raise ValueError(f"no pinned asset mapped for {platform.system()}/{platform.machine()}")
    suffix = "zip" if system == "windows" else "tar.gz"
    name = f"CLIProxyAPI_{VERSION}_{system}_{arch}.{suffix}"
    checksums = dict(line.split()[::-1] for line in (ROOT / "vendor/cliproxyapi/checksums.txt").read_text().splitlines() if line.strip())
    expected = checksums[name]
    cache = args.cache_dir
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / name
    if not archive.exists() or digest(archive) != expected:
        with tempfile.NamedTemporaryFile(dir=cache, delete=False) as temp:
            temp_path = Path(temp.name)
        try:
            url = f"https://github.com/router-for-me/CLIProxyAPI/releases/download/v{VERSION}/{name}"
            with urllib.request.urlopen(url, timeout=60) as source, temp_path.open("wb") as out:
                shutil.copyfileobj(source, out)
            if digest(temp_path) != expected:
                raise ValueError(f"checksum mismatch for {name}")
            os.replace(temp_path, archive)
        finally:
            temp_path.unlink(missing_ok=True)
    member_name = "cli-proxy-api.exe" if system == "windows" else "cli-proxy-api"
    fd, temp_name = tempfile.mkstemp(dir=cache)
    os.close(fd)
    temp_path = Path(temp_name)
    try:
        with temp_path.open("wb") as temp:
            if system == "windows":
                with zipfile.ZipFile(archive) as source:
                    matches = [m for m in source.infolist() if m.filename == member_name and not m.is_dir()]
                    if len(matches) != 1 or matches[0].file_size > 512 * 1024 * 1024:
                        raise ValueError("invalid executable member in ZIP")
                    with source.open(matches[0]) as member:
                        shutil.copyfileobj(member, temp)
            else:
                with tarfile.open(archive, "r:gz") as source:
                    matches = [m for m in source.getmembers() if m.name == member_name and m.isfile()]
                    if len(matches) != 1 or matches[0].size > 512 * 1024 * 1024:
                        raise ValueError("invalid executable member in tar")
                    with source.extractfile(matches[0]) as member:
                        shutil.copyfileobj(member, temp)
        if system != "windows":
            temp_path.chmod(0o755)
        binary = cache / member_name
        os.replace(temp_path, binary)
    finally:
        temp_path.unlink(missing_ok=True)
    print(archive)
    print(binary)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, zipfile.BadZipFile, tarfile.TarError) as error:
        sys.exit(str(error))
