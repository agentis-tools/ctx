#!/usr/bin/env python3
"""Validate release names, layouts, and SHA256SUMS."""

import argparse
import hashlib
from pathlib import Path
import tarfile
import zipfile

TARGETS = {
    "x86_64-unknown-linux-gnu": ".tar.gz",
    "x86_64-apple-darwin": ".tar.gz",
    "aarch64-apple-darwin": ".tar.gz",
    "x86_64-pc-windows-msvc": ".zip",
}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--complete", action="store_true")
    args = parser.parse_args()
    version = args.version.removeprefix("v")
    expected = []
    for target, suffix in TARGETS.items():
        path = args.directory / f"ctx-v{version}-{target}{suffix}"
        if args.complete or path.exists():
            expected.append(path)
            validate_archive(path, target)
    if not expected:
        parser.error("no release archives found")

    sums = args.directory / "SHA256SUMS"
    if sums.exists():
        listed = {}
        for line in sums.read_text().splitlines():
            digest, name = line.split(maxsplit=1)
            listed[name.lstrip("* ")] = digest
        artifacts = [p for p in args.directory.iterdir() if p.is_file() and p.name != "SHA256SUMS"]
        for path in artifacts:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
            assert listed.get(path.name) == actual, f"missing or incorrect checksum for {path.name}"


def validate_archive(path: Path, target: str) -> None:
    assert path.is_file(), f"missing release archive: {path}"
    root = path.name.removesuffix(".zip").removesuffix(".tar.gz")
    binary = "ctx.exe" if target.endswith("windows-msvc") else "ctx"
    required = {f"{root}/{name}" for name in (binary, "README.md", "LICENSE-MIT", "LICENSE-APACHE")}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            names = set(archive.namelist())
    else:
        with tarfile.open(path, "r:gz") as archive:
            names = set(archive.getnames())
    assert required <= names, f"{path.name} missing {sorted(required - names)}"


if __name__ == "__main__":
    main()
