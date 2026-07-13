#!/usr/bin/env python3
"""Create a deterministic ctx release archive from a built binary."""

import argparse
import gzip
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import zipfile

EPOCH = 946684800  # 2000-01-01, accepted by ZIP and stable across runners.
FILES = ("README.md", "LICENSE-MIT", "LICENSE-APACHE")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("target")
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    version = args.version.removeprefix("v")
    root = Path(__file__).resolve().parent.parent
    windows = args.target.endswith("windows-msvc")
    binary_name = "ctx.exe" if windows else "ctx"
    archive_root = f"ctx-v{version}-{args.target}"
    suffix = ".zip" if windows else ".tar.gz"
    destination = args.output / f"{archive_root}{suffix}"

    if not args.binary.is_file():
        parser.error(f"binary does not exist: {args.binary}")
    missing = [name for name in FILES if not (root / name).is_file()]
    if missing:
        parser.error(f"required release files are missing: {', '.join(missing)}")

    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        staging = Path(tmp) / archive_root
        staging.mkdir()
        shutil.copyfile(args.binary, staging / binary_name)
        os.chmod(staging / binary_name, 0o755)
        for name in FILES:
            shutil.copyfile(root / name, staging / name)

        if windows:
            write_zip(destination, staging, archive_root)
        else:
            write_tar(destination, staging, archive_root)
    print(destination)


def write_tar(destination: Path, staging: Path, archive_root: str) -> None:
    with destination.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=EPOCH) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for path in [staging, *sorted(staging.iterdir(), key=lambda item: item.name)]:
                    relative = Path(archive_root) / path.relative_to(staging)
                    info = archive.gettarinfo(str(path), str(relative))
                    info.uid = info.gid = 0
                    info.uname = info.gname = "root"
                    info.mtime = EPOCH
                    archive.addfile(info, path.open("rb") if path.is_file() else None)


def write_zip(destination: Path, staging: Path, archive_root: str) -> None:
    timestamp = (2000, 1, 1, 0, 0, 0)
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(staging.iterdir(), key=lambda item: item.name):
            info = zipfile.ZipInfo(f"{archive_root}/{path.name}", timestamp)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o755 if path.name == "ctx.exe" else 0o644) << 16
            archive.writestr(info, path.read_bytes())


if __name__ == "__main__":
    main()
