#!/usr/bin/env python3
"""Generate package-manager metadata from a release SHA256SUMS file."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = "https://github.com/agentis-tools/ctx"
TARGETS = {
    "linux-x86_64": "x86_64-unknown-linux-gnu",
    "macos-x86_64": "x86_64-apple-darwin",
    "macos-aarch64": "aarch64-apple-darwin",
    "windows-x86_64": "x86_64-pc-windows-msvc",
}


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"error: {message}")


def parse_version(value: str) -> str:
    value = value.removeprefix("v")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", value):
        fail(f"invalid release version: {value!r}")
    return value


def read_checksums(path: Path) -> dict[str, str]:
    if not path.is_file():
        fail(f"checksum file does not exist: {path}")
    result: dict[str, str] = {}
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line:
            continue
        match = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(.+)", line)
        if not match:
            fail(f"invalid SHA256SUMS line {number}: {raw!r}")
        digest, name = match.groups()
        name = Path(name).name
        if name in result:
            fail(f"duplicate checksum entry: {name}")
        result[name] = digest.lower()
    return result


def asset(version: str, target: str) -> str:
    suffix = ".zip" if "windows" in target else ".tar.gz"
    return f"ctx-v{version}-{target}{suffix}"


def digest(checksums: dict[str, str], version: str, target: str) -> str:
    name = asset(version, target)
    if name not in checksums:
        fail(f"SHA256SUMS is missing required release asset: {name}")
    return checksums[name]


def release_url(version: str, target: str) -> str:
    return f"{REPO}/releases/download/v{version}/{asset(version, target)}"


def homebrew(version: str, sums: dict[str, str]) -> str:
    mac_arm = TARGETS["macos-aarch64"]
    mac_intel = TARGETS["macos-x86_64"]
    linux = TARGETS["linux-x86_64"]
    return f'''# frozen_string_literal: true

# Homebrew formula for the prebuilt ctx CLI.
class Ctx < Formula
  desc "Fast CLI tool that generates AI-ready context from your codebase"
  homepage "https://docs.agentis.tools"
  version "{version}"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "{release_url(version, mac_arm)}"
      sha256 "{digest(sums, version, mac_arm)}"
    else
      url "{release_url(version, mac_intel)}"
      sha256 "{digest(sums, version, mac_intel)}"
    end
  end

  on_linux do
    on_intel do
      url "{release_url(version, linux)}"
      sha256 "{digest(sums, version, linux)}"
    end
  end

  def install
    bin.install "ctx-v#{{version}}-#{{target_triple}}/ctx"
  end

  def target_triple
    if OS.mac?
      Hardware::CPU.arm? ? "aarch64-apple-darwin" : "x86_64-apple-darwin"
    else
      "x86_64-unknown-linux-gnu"
    end
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/ctx --version")
  end
end
'''


def pkgbuild(version: str, sums: dict[str, str]) -> str:
    target = TARGETS["linux-x86_64"]
    return f'''# Maintainer: agentis-tools contributors
pkgname=ctx-bin
pkgver={version}
pkgrel=1
pkgdesc="Fast CLI tool that generates AI-ready context from your codebase"
arch=('x86_64')
url="{REPO}"
license=('MIT' 'Apache-2.0')
provides=('ctx')
conflicts=('ctx')
options=('!strip')
source_x86_64=("ctx-v${{pkgver}}-{target}.tar.gz::{REPO}/releases/download/v${{pkgver}}/ctx-v${{pkgver}}-{target}.tar.gz")
sha256sums_x86_64=('{digest(sums, version, target)}')

package() {{
  local srcdir="${{srcdir}}/ctx-v${{pkgver}}-{target}"
  install -Dm755 "${{srcdir}}/ctx" "${{pkgdir}}/usr/bin/ctx"
  install -Dm644 "${{srcdir}}/LICENSE-MIT" "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE-MIT"
  install -Dm644 "${{srcdir}}/LICENSE-APACHE" "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE-APACHE"
}}
'''


def srcinfo(version: str, sums: dict[str, str]) -> str:
    target = TARGETS["linux-x86_64"]
    return f'''pkgbase = ctx-bin
\tpkgdesc = Fast CLI tool that generates AI-ready context from your codebase
\tpkgver = {version}
\tpkgrel = 1
\turl = {REPO}
\tarch = x86_64
\tlicense = MIT
\tlicense = Apache-2.0
\tprovides = ctx
\tconflicts = ctx
\toptions = !strip
\tsource_x86_64 = ctx-v{version}-{target}.tar.gz::{release_url(version, target)}
\tsha256sums_x86_64 = {digest(sums, version, target)}

pkgname = ctx-bin
'''


def scoop(version: str, sums: dict[str, str]) -> str:
    target = TARGETS["windows-x86_64"]
    manifest = {
        "version": version,
        "description": "Fast CLI tool that generates AI-ready context from your codebase",
        "homepage": "https://docs.agentis.tools",
        "license": "MIT OR Apache-2.0",
        "architecture": {
            "64bit": {
                "url": release_url(version, target),
                "hash": digest(sums, version, target),
                "extract_dir": f"ctx-v{version}-{target}",
                "bin": "ctx.exe",
            }
        },
        "checkver": {"github": REPO},
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"{REPO}/releases/download/v$version/ctx-v$version-{target}.zip",
                    "hash": {"url": "$url.sha256"},
                    "extract_dir": f"ctx-v$version-{target}",
                }
            }
        },
    }
    return json.dumps(manifest, indent=4) + "\n"


def output_files(version: str, sums: dict[str, str]) -> dict[Path, str]:
    return {
        Path("packaging/homebrew/ctx.rb"): homebrew(version, sums),
        Path("packaging/aur/PKGBUILD"): pkgbuild(version, sums),
        Path("packaging/aur/.SRCINFO"): srcinfo(version, sums),
        Path("packaging/scoop/ctx.json"): scoop(version, sums),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("checksums", type=Path)
    parser.add_argument("--manager", choices=("all", "homebrew", "aur", "scoop"), default="all")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    version = parse_version(args.version)
    sums = read_checksums(args.checksums)
    selected = output_files(version, sums)
    if args.manager != "all":
        selected = {p: data for p, data in selected.items() if p.parts[1] == args.manager}
    stale: list[str] = []
    for path, content in selected.items():
        if args.check:
            if not path.is_file() or path.read_text(encoding="utf-8") != content:
                stale.append(str(path))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            print(path)
    if stale:
        fail("generated package metadata is stale: " + ", ".join(stale))


if __name__ == "__main__":
    main()
