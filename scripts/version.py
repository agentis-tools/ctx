#!/usr/bin/env python3
"""Authoritative ctx version, changelog, and release validation tooling."""

from __future__ import annotations

import argparse
from collections.abc import Mapping
import dataclasses
import datetime as dt
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "Cargo.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
LOCKFILES = (ROOT / "Cargo.lock", ROOT / "perf" / "Cargo.lock")
PACKAGE = "agentis-ctx"
ALLOWED_CATEGORIES = {
    "Added", "Changed", "Deprecated", "Removed", "Fixed", "Security",
    "Documentation", "Internal",
}


class PolicyError(RuntimeError):
    pass


@dataclasses.dataclass(frozen=True)
class SemVer:
    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...] = ()
    build: tuple[str, ...] = ()

    _PATTERN = re.compile(
        r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
        r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
        r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
    )

    @classmethod
    def parse(cls, value: str) -> "SemVer":
        match = cls._PATTERN.fullmatch(value)
        if not match:
            raise PolicyError(f"invalid Semantic Version: {value!r}")
        prerelease = tuple(match.group(4).split(".")) if match.group(4) else ()
        build = tuple(match.group(5).split(".")) if match.group(5) else ()
        for identifier in prerelease:
            if identifier.isdigit() and len(identifier) > 1 and identifier.startswith("0"):
                raise PolicyError(
                    f"invalid Semantic Version {value!r}: numeric prerelease identifiers "
                    "must not contain leading zeroes"
                )
        return cls(*(int(match.group(i)) for i in range(1, 4)), prerelease, build)

    def __str__(self) -> str:
        value = f"{self.major}.{self.minor}.{self.patch}"
        if self.prerelease:
            value += "-" + ".".join(self.prerelease)
        if self.build:
            value += "+" + ".".join(self.build)
        return value

    def precedence(self) -> tuple[object, ...]:
        return self.major, self.minor, self.patch

    def compare(self, other: "SemVer") -> int:
        if self.precedence() != other.precedence():
            return 1 if self.precedence() > other.precedence() else -1
        if not self.prerelease and not other.prerelease:
            return 0
        if not self.prerelease:
            return 1
        if not other.prerelease:
            return -1
        for left, right in zip(self.prerelease, other.prerelease):
            if left == right:
                continue
            if left.isdigit() and right.isdigit():
                return 1 if int(left) > int(right) else -1
            if left.isdigit() != right.isdigit():
                return -1 if left.isdigit() else 1
            return 1 if left > right else -1
        return (len(self.prerelease) > len(other.prerelease)) - (
            len(self.prerelease) < len(other.prerelease)
        )


def manifest_data() -> dict:
    with MANIFEST.open("rb") as handle:
        return tomllib.load(handle)


def current_version() -> SemVer:
    package = manifest_data().get("package", {})
    if package.get("name") != PACKAGE:
        raise PolicyError(
            f"{MANIFEST.relative_to(ROOT)} [package].name must be {PACKAGE!r}"
        )
    raw = package.get("version")
    if not isinstance(raw, str):
        raise PolicyError("Cargo.toml [package].version must be a string")
    return SemVer.parse(raw)


def local_lock_version(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    for block in text.split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
        if name and name.group(1) == PACKAGE:
            version = re.search(r'^version = "([^"]+)"$', block, re.MULTILINE)
            if not version:
                break
            return version.group(1)
    raise PolicyError(f"{path.relative_to(ROOT)} has no local {PACKAGE} package entry")


def changelog_section(text: str, heading: str) -> str:
    marker = f"## [{heading}]"
    start = text.find(marker)
    if start < 0:
        raise PolicyError(f"CHANGELOG.md is missing {marker}")
    body_start = text.find("\n", start)
    next_heading = text.find("\n## [", body_start + 1)
    return text[body_start + 1 : next_heading if next_heading >= 0 else len(text)].strip()


def has_release_content(section: str) -> bool:
    return any(
        line.startswith("- ")
        for line in section.splitlines()
        if not line.startswith("[Unreleased]:")
    )


def validate_changelog(version: SemVer, *, release: bool) -> list[str]:
    errors: list[str] = []
    text = CHANGELOG.read_text(encoding="utf-8")
    try:
        released = changelog_section(text, str(version))
    except PolicyError as error:
        errors.append(str(error))
        released = ""
    if released and not has_release_content(released):
        errors.append(f"CHANGELOG.md release {version} is empty")
    expected_link = (
        f"[Unreleased]: https://github.com/agentis-tools/ctx/compare/v{version}...HEAD"
    )
    if expected_link not in text:
        errors.append(
            "CHANGELOG.md Unreleased comparison must start at the authoritative "
            f"version: expected {expected_link!r}"
        )
    headings = re.findall(r"^## \[([^]]+)]", text, re.MULTILINE)
    duplicates = sorted({item for item in headings if headings.count(item) > 1})
    if duplicates:
        errors.append(f"CHANGELOG.md contains duplicate release headings: {duplicates}")
    releases: list[tuple[SemVer, dt.date]] = []
    for heading, date_text in re.findall(
        r"^## \[([^]]+)](?: - (\S+))?$", text, re.MULTILINE
    ):
        if heading == "Unreleased":
            if date_text:
                errors.append("CHANGELOG.md Unreleased heading must not have a date")
            continue
        try:
            parsed_version = SemVer.parse(heading)
        except PolicyError as error:
            errors.append(f"CHANGELOG.md heading is invalid: {error}")
            continue
        try:
            parsed_date = dt.date.fromisoformat(date_text)
        except ValueError:
            errors.append(
                f"CHANGELOG.md release {heading} needs a real ISO date, got {date_text!r}"
            )
            continue
        releases.append((parsed_version, parsed_date))
    for (newer_version, newer_date), (older_version, older_date) in zip(
        releases, releases[1:]
    ):
        if newer_version.compare(older_version) <= 0:
            errors.append(
                f"CHANGELOG.md releases are not newest-first: {newer_version}, {older_version}"
            )
        if newer_date < older_date:
            errors.append(
                f"CHANGELOG.md dates are not newest-first: {newer_date}, {older_date}"
            )
    categories = re.findall(r"^### (.+)$", text, re.MULTILINE)
    unexpected = sorted(set(categories) - ALLOWED_CATEGORIES)
    if unexpected:
        errors.append(
            "CHANGELOG.md contains unsupported categories "
            f"{unexpected}; allowed: {sorted(ALLOWED_CATEGORIES)}"
        )
    if release and not re.search(
        rf"^## \[{re.escape(str(version))}] - \d{{4}}-\d{{2}}-\d{{2}}$",
        text,
        re.MULTILINE,
    ):
        errors.append(f"release {version} needs a dated CHANGELOG.md heading")
    return errors


def run_metadata(manifest: Path) -> str | None:
    command = [
        "cargo", "metadata", "--locked", "--offline", "--no-deps",
        "--format-version", "1", "--manifest-path", str(manifest),
    ]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        return (
            f"cargo metadata failed for {manifest.relative_to(ROOT)}: "
            f"{result.stderr.strip()}"
        )
    return None


def tag_from_environment(environment: Mapping[str, str]) -> str | None:
    """Return the Actions ref name only when the workflow runs for a tag."""
    if environment.get("GITHUB_REF_TYPE") == "tag":
        return environment.get("GITHUB_REF_NAME")
    return None


def check(args: argparse.Namespace) -> int:
    errors: list[str] = []
    try:
        version = current_version()
    except PolicyError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

    package = manifest_data()["package"]
    if package.get("rust-version") != "1.91":
        errors.append("Cargo.toml rust-version must match the governed MSRV 1.91")
    bins = manifest_data().get("bin", [])
    if not any(item.get("name") == "ctx" for item in bins):
        errors.append("Cargo.toml must declare the ctx binary")

    for lockfile in LOCKFILES:
        try:
            actual = local_lock_version(lockfile)
            if actual != str(version):
                errors.append(
                    f"{lockfile.relative_to(ROOT)} resolves {PACKAGE} {actual}, "
                    f"expected {version}; run scripts/version.py set {version}"
                )
        except PolicyError as error:
            errors.append(str(error))

    for manifest in (MANIFEST, ROOT / "perf" / "Cargo.toml"):
        metadata_error = run_metadata(manifest)
        if metadata_error:
            errors.append(metadata_error)

    # GITHUB_REF_NAME is also set to values such as `main` and `123/merge`.
    # Treat it as a release tag only when GitHub identifies the ref as a tag;
    # callers outside Actions should pass --tag explicitly.
    tag = args.tag or tag_from_environment(os.environ)
    if tag and tag != f"v{version}":
        errors.append(f"release tag must be v{version}, got {tag!r}")
    release = args.release or bool(tag)
    errors.extend(validate_changelog(version, release=release))

    update_source = (ROOT / "src" / "update.rs").read_text(encoding="utf-8")
    for invariant in (
        'format!("ctx-{tag}-{target}.{ext}")',
        'format!("{}/releases/tags/{tag}", base_url())',
        'env!("CARGO_PKG_VERSION")',
    ):
        if invariant not in update_source:
            errors.append(
                "src/update.rs no longer matches the governed v<version>/artifact "
                f"contract; missing {invariant!r}"
            )

    workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
        encoding="utf-8"
    )
    if "scripts/release-check.sh" not in workflow:
        errors.append("release.yml must delegate tag/version validation to release-check.sh")
    stale = re.findall(r"ctx-v\d+\.\d+\.\d+", workflow)
    if stale:
        errors.append(f"release.yml contains hardcoded release artifacts: {stale}")

    if not args.skip_binary:
        binary = Path(args.binary or os.environ.get("CTX_BINARY", ROOT / "target/debug/ctx"))
        if not binary.is_absolute():
            binary = ROOT / binary
        if not binary.is_file():
            errors.append(
                f"compiled ctx binary not found at {binary}; build it or pass --binary"
            )
        else:
            result = subprocess.run(
                [str(binary), "--version"], cwd=ROOT, text=True, capture_output=True
            )
            expected = f"ctx {version}"
            if result.returncode or result.stdout.strip() != expected:
                errors.append(
                    f"{binary} --version returned {result.stdout.strip()!r}, "
                    f"expected {expected!r}"
                )

    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(f"OK: {PACKAGE} {version} version and release invariants are consistent")
    return 0


def replace_manifest_version(text: str, old: str, new: str) -> str:
    package_at = text.find("[package]")
    next_section = text.find("\n[", package_at + 1)
    prefix, section, suffix = text[:package_at], text[package_at:next_section], text[next_section:]
    replaced, count = re.subn(
        rf'(?m)^version = "{re.escape(old)}"$', f'version = "{new}"', section
    )
    if count != 1:
        raise PolicyError("could not update exactly one [package].version in Cargo.toml")
    return prefix + replaced + suffix


def replace_lock_version(text: str, old: str, new: str) -> str:
    pattern = re.compile(
        rf'(\[\[package]]\nname = "{re.escape(PACKAGE)}"\nversion = ")'
        rf'{re.escape(old)}("\n)'
    )
    updated, count = pattern.subn(rf"\g<1>{new}\2", text, count=1)
    if count != 1:
        raise PolicyError(f"could not update the {PACKAGE} lockfile entry")
    return updated


def finalize_changelog(text: str, old: str, new: str, date: str, allow_empty: bool) -> str:
    unreleased = changelog_section(text, "Unreleased")
    if not has_release_content(unreleased) and not allow_empty:
        raise PolicyError(
            "CHANGELOG.md Unreleased section is empty; add reviewed release notes "
            "or pass --allow-empty explicitly"
        )
    marker = "## [Unreleased]"
    text = text.replace(marker, f"{marker}\n\n## [{new}] - {date}", 1)
    old_unreleased = (
        f"[Unreleased]: https://github.com/agentis-tools/ctx/compare/v{old}...HEAD"
    )
    new_links = (
        f"[Unreleased]: https://github.com/agentis-tools/ctx/compare/v{new}...HEAD\n"
        f"[{new}]: https://github.com/agentis-tools/ctx/compare/v{old}...v{new}"
    )
    if old_unreleased not in text:
        raise PolicyError(f"CHANGELOG.md is missing {old_unreleased!r}")
    return text.replace(old_unreleased, new_links, 1)


def require_breaking_bump(text: str, old: SemVer, new: SemVer) -> None:
    """Refuse to release accumulated breaks under an insufficient bump.

    PR review records a break with the `breaking-change` label and a BREAKING:
    entry under Unreleased; the version increase those breaks require is owed
    at release time, which is here. Pre-1.0 the vehicle is 0.MINOR.0, so a
    minor bump suffices; from 1.0 a break needs a major.
    """
    if "BREAKING:" not in changelog_section(text, "Unreleased"):
        return
    sufficient = new.major > old.major if old.major > 0 else new.minor > old.minor
    if not sufficient:
        level = "major" if old.major > 0 else "minor"
        raise PolicyError(
            f"Unreleased contains a BREAKING: entry; {old} -> {new} is not a "
            f"{level} bump. Breaking changes require a {level} increase."
        )


def ensure_clean(allow_dirty: bool) -> None:
    result = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=normal"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=True,
    )
    if result.stdout.strip() and not allow_dirty:
        raise PolicyError(
            "working tree is dirty; commit/stash changes or pass --allow-dirty explicitly"
        )
    if result.stdout.strip():
        print("WARNING: changing version in a dirty working tree", file=sys.stderr)


def set_version(args: argparse.Namespace) -> int:
    ensure_clean(args.allow_dirty)
    old = current_version()
    new = SemVer.parse(args.version)
    comparison = new.compare(old)
    if comparison < 0 and not args.allow_regression:
        raise PolicyError(
            f"version regression {old} -> {new} refused; pass --allow-regression explicitly"
        )
    if comparison == 0 and str(new) == str(old):
        print(f"{new}: already current; no files changed")
        return 0
    date = args.date or dt.datetime.now(dt.timezone.utc).date().isoformat()
    try:
        dt.date.fromisoformat(date)
    except ValueError as error:
        raise PolicyError(f"invalid release date {date!r}: {error}") from error

    changed: list[Path] = []
    manifest_text = MANIFEST.read_text(encoding="utf-8")
    changelog_text = CHANGELOG.read_text(encoding="utf-8")
    require_breaking_bump(changelog_text, old, new)
    updates: dict[Path, str] = {
        MANIFEST: replace_manifest_version(manifest_text, str(old), str(new)),
        CHANGELOG: finalize_changelog(
            changelog_text, str(old), str(new), date, args.allow_empty
        ),
    }
    for lockfile in LOCKFILES:
        updates[lockfile] = replace_lock_version(
            lockfile.read_text(encoding="utf-8"), str(old), str(new)
        )
    for path, content in updates.items():
        if path.read_text(encoding="utf-8") != content:
            path.write_text(content, encoding="utf-8")
            changed.append(path)
    print(f"Updated {old} -> {new}")
    for path in changed:
        print(path.relative_to(ROOT))
    return 0


def bump(args: argparse.Namespace) -> int:
    current = current_version()
    if args.part == "patch":
        target = SemVer(current.major, current.minor, current.patch + 1)
    elif args.part == "minor":
        target = SemVer(current.major, current.minor + 1, 0)
    else:
        target = SemVer(current.major + 1, 0, 0)
    args.version = str(target)
    return set_version(args)


def release_notes(args: argparse.Namespace) -> int:
    version = SemVer.parse(args.version)
    current = current_version()
    if version.compare(current) != 0 or str(version) != str(current):
        raise PolicyError(
            f"release notes requested for {version}, authoritative version is {current}"
        )
    section = changelog_section(CHANGELOG.read_text(encoding="utf-8"), str(version))
    if not has_release_content(section):
        raise PolicyError(f"CHANGELOG.md release {version} is empty")
    output = f"# ctx {version}\n\n{section.strip()}\n"
    if args.output:
        Path(args.output).write_text(output, encoding="utf-8")
    else:
        sys.stdout.write(output)
    return 0


def parser() -> argparse.ArgumentParser:
    cli = argparse.ArgumentParser(description=__doc__)
    commands = cli.add_subparsers(dest="command", required=True)
    commands.add_parser("show", aliases=["current"], help="print the authoritative version")

    check_parser = commands.add_parser("check", help="validate version/release invariants")
    check_parser.add_argument("--tag")
    check_parser.add_argument("--release", action="store_true")
    check_parser.add_argument("--binary")
    check_parser.add_argument("--skip-binary", action="store_true")

    def mutation_options(command: argparse.ArgumentParser) -> None:
        command.add_argument("--allow-dirty", action="store_true")
        command.add_argument("--allow-regression", action="store_true")
        command.add_argument("--allow-empty", action="store_true")
        command.add_argument("--date")

    set_parser = commands.add_parser("set", help="set an explicit release version")
    set_parser.add_argument("version")
    mutation_options(set_parser)
    bump_parser = commands.add_parser("bump", help="bump a SemVer component")
    bump_parser.add_argument("part", choices=("patch", "minor", "major"))
    mutation_options(bump_parser)

    notes = commands.add_parser("notes", help="render reviewed release notes")
    notes.add_argument("version")
    notes.add_argument("--output")
    return cli


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command in ("show", "current"):
            print(current_version())
            return 0
        if args.command == "check":
            return check(args)
        if args.command == "set":
            return set_version(args)
        if args.command == "bump":
            return bump(args)
        return release_notes(args)
    except (PolicyError, OSError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
