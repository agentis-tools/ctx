#!/usr/bin/env python3
"""Validate release names, layouts, and SHA256SUMS."""

import argparse
import hashlib
from pathlib import Path
import posixpath
import re
import tarfile
import zipfile

TARGETS = {
    "x86_64-unknown-linux-gnu": ".tar.gz",
    "x86_64-apple-darwin": ".tar.gz",
    "aarch64-apple-darwin": ".tar.gz",
    "x86_64-pc-windows-msvc": ".zip",
}


def fail(message: str) -> None:
    raise SystemExit(f"ERROR: {message}")


def normalized_manifest_name(name: str) -> str:
    return posixpath.normpath(name.lstrip("* ").strip())


def read_checksums(path: Path) -> dict[str, str]:
    listed: dict[str, str] = {}
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        if not raw.strip():
            continue
        fields = raw.split(maxsplit=1)
        if len(fields) != 2:
            fail(f"invalid checksum line {number}: {raw!r}")
        digest, name = fields
        name = normalized_manifest_name(name)
        if name == "SHA256SUMS" or Path(name).name == "SHA256SUMS":
            fail(f"SHA256SUMS must not list itself (line {number})")
        if not re.fullmatch(r"[0-9a-fA-F]{64}", digest):
            fail(f"invalid SHA256 digest on line {number}: {digest!r}")
        if name in listed:
            fail(f"duplicate checksum entry for {name!r}")
        listed[name] = digest.lower()
    return listed


def validate_checksums(directory: Path, sums: Path) -> None:
    listed = read_checksums(sums)
    artifacts = {
        path.name: path
        for path in directory.iterdir()
        if path.is_file() and path.name != "SHA256SUMS"
    }
    missing = sorted(set(artifacts) - set(listed))
    extra = sorted(set(listed) - set(artifacts))
    if missing:
        fail(f"SHA256SUMS is missing entries: {', '.join(missing)}")
    if extra:
        fail(f"SHA256SUMS lists unknown files: {', '.join(extra)}")
    for name, path in artifacts.items():
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if listed[name] != actual:
            fail(f"missing or incorrect checksum for {name}")


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
    if args.complete and not sums.exists():
        fail(f"missing aggregate checksum manifest: {sums}")
    if sums.exists():
        validate_checksums(args.directory, sums)


def validate_archive(path: Path, target: str) -> None:
    if not path.is_file():
        fail(f"missing release archive: {path}")
    root = path.name.removesuffix(".zip").removesuffix(".tar.gz")
    binary = "ctx.exe" if target.endswith("windows-msvc") else "ctx"
    required = {f"{root}/{name}" for name in (binary, "README.md", "LICENSE-MIT", "LICENSE-APACHE")}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            names = set(archive.namelist())
    else:
        with tarfile.open(path, "r:gz") as archive:
            names = set(archive.getnames())
    if required > names:
        fail(f"{path.name} missing {sorted(required - names)}")


if __name__ == "__main__":
    main()
