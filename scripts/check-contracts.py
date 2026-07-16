#!/usr/bin/env python3
"""Capture and compare the machine-checkable ctx CLI compatibility surface."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "governance/contracts/cli.json"
SENSITIVE_PATHS = (
    "src/", "Cargo.toml", "docs/json-output.md", "docs/configuration.md",
    "scripts/release-", ".github/workflows/release.yml", "perf/schemas/",
)


class ContractError(RuntimeError):
    pass


def help_text(binary: Path, path: tuple[str, ...]) -> str:
    result = subprocess.run(
        [str(binary), *path, "--help"], cwd=ROOT, text=True, capture_output=True
    )
    if result.returncode:
        raise ContractError(
            f"{' '.join((binary.name, *path, '--help'))} failed: {result.stderr.strip()}"
        )
    return result.stdout


def section_lines(text: str, heading: str) -> list[str]:
    lines = text.splitlines()
    try:
        start = lines.index(heading) + 1
    except ValueError:
        return []
    output: list[str] = []
    for line in lines[start:]:
        if line and not line.startswith(" "):
            break
        if line.strip():
            output.append(line.strip())
    return output


def command_names(text: str) -> list[str]:
    names: list[str] = []
    for line in section_lines(text, "Commands:"):
        name = line.split()[0]
        if name != "help" and re.fullmatch(r"[a-z][a-z0-9-]*", name):
            names.append(name)
    return sorted(set(names))


def options(text: str) -> dict[str, str]:
    result: dict[str, str] = {}
    lines = text.splitlines()
    in_options = False
    current: str | None = None
    for raw in lines:
        if raw == "Options:":
            in_options = True
            continue
        if in_options and raw and not raw.startswith(" "):
            break
        if not in_options or not raw.strip():
            continue
        longs = re.findall(r"--[a-z][a-z0-9-]*", raw)
        if longs:
            current = longs[-1]
            result[current] = re.sub(r"\s+", " ", raw.strip())
        elif current and raw.startswith("          "):
            result[current] += " " + re.sub(r"\s+", " ", raw.strip())
    return dict(sorted(result.items()))


def capture(binary: Path) -> dict:
    binary = binary.resolve()
    if not binary.is_file():
        raise ContractError(f"compiled ctx binary not found: {binary}")
    pending: list[tuple[str, ...]] = [()]
    commands: dict[str, dict] = {}
    while pending:
        path = pending.pop(0)
        output = help_text(binary, path)
        children = command_names(output)
        key = "ctx" if not path else "ctx " + " ".join(path)
        commands[key] = {"options": options(output), "subcommands": children}
        for child in children:
            pending.append((*path, child))
    return {"schema": 1, "commands": dict(sorted(commands.items()))}


def write_contract(binary: Path) -> None:
    CONTRACT.parent.mkdir(parents=True, exist_ok=True)
    CONTRACT.write_text(
        json.dumps(capture(binary), indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(CONTRACT.relative_to(ROOT))


def load_contract(path: Path = CONTRACT) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot read {path}: {error}") from error
    if value.get("schema") != 1 or not isinstance(value.get("commands"), dict):
        raise ContractError(f"{path} is not a CLI contract schema 1 document")
    return value


def check_snapshot(binary: Path) -> None:
    expected = load_contract()
    actual = capture(binary)
    if actual != expected:
        raise ContractError(
            "CLI contract snapshot is stale; review the compatibility impact and run "
            f"scripts/check-contracts.py capture --binary {binary}"
        )
    print("OK: compiled CLI matches governance/contracts/cli.json")


def git_output(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True
    )
    if result.returncode:
        raise ContractError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout


def contract_from_ref(reference: str) -> dict | None:
    result = subprocess.run(
        ["git", "show", f"{reference}:governance/contracts/cli.json"],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if result.returncode:
        return None
    return json.loads(result.stdout)


def version_from_ref(reference: str) -> tuple[int, int, int]:
    raw = git_output("show", f"{reference}:Cargo.toml")
    data = tomllib.loads(raw)
    value = data["package"]["version"]
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value)
    if not match:
        raise ContractError(f"base Cargo.toml version is not SemVer: {value}")
    return tuple(int(item) for item in match.groups())


def current_version() -> tuple[int, int, int]:
    with (ROOT / "Cargo.toml").open("rb") as handle:
        value = tomllib.load(handle)["package"]["version"]
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value)
    if not match:
        raise ContractError(f"Cargo.toml version is not SemVer: {value}")
    return tuple(int(item) for item in match.groups())


def breaking_notes_section(old: tuple[int, int, int], new: tuple[int, int, int]) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    heading = "Unreleased" if new == old else ".".join(str(item) for item in new)
    marker = f"## [{heading}]"
    start = text.find(marker)
    if start < 0:
        raise ContractError(f"CHANGELOG.md is missing {marker}")
    end = text.find("\n## [", start + len(marker))
    return text[start : end if end >= 0 else len(text)]


def compare_contracts(base: dict, current: dict) -> tuple[list[str], list[str]]:
    removed: list[str] = []
    changed: list[str] = []
    base_commands = base.get("commands", {})
    current_commands = current.get("commands", {})
    for command, old in base_commands.items():
        if command not in current_commands:
            removed.append(f"command {command}")
            continue
        new = current_commands[command]
        for option in old.get("options", {}):
            if option not in new.get("options", {}):
                removed.append(f"option {command} {option}")
        for option, description in old.get("options", {}).items():
            if option in new.get("options", {}) and new["options"][option] != description:
                changed.append(f"option contract {command} {option}")
    return removed, changed


def pr_policy(base_ref: str, labels: set[str]) -> None:
    current = load_contract()
    base = contract_from_ref(base_ref)
    changed_files = set(
        git_output("diff", "--name-only", f"{base_ref}...HEAD").splitlines()
    )
    sensitive = sorted(
        path for path in changed_files if any(path == prefix or path.startswith(prefix) for prefix in SENSITIVE_PATHS)
    )
    if base is None:
        print("OK: bootstrapping compatibility contract; no prior snapshot")
        return
    removed, changed = compare_contracts(base, current)
    if (sensitive or changed or removed) and "contract-review" not in labels:
        details = ", ".join((sensitive + changed + removed)[:12])
        raise ContractError(
            "compatibility-sensitive changes require maintainer label contract-review: "
            + details
        )
    if removed and "breaking-change" not in labels:
        raise ContractError(
            "removed CLI contracts require maintainer label breaking-change: "
            + ", ".join(removed)
        )
    # An acknowledged break must read as one in the changelog. Check the lines
    # this PR adds, not the section as a whole: Unreleased accumulates entries
    # from every merged break, so "the section mentions BREAKING:" would pass on
    # somebody else's entry and wave this one through.
    #
    # The matching version increase is enforced when the release is cut
    # (version.py), not here: breaks land under Unreleased and the release PR
    # carries the bump, per governance/releasing.md.
    if "breaking-change" in labels:
        added = [
            line
            for line in git_output(
                "diff", f"{base_ref}...HEAD", "--", "CHANGELOG.md"
            ).splitlines()
            if line.startswith("+") and not line.startswith("+++")
        ]
        # Match the entry convention ("- BREAKING: ..."), not a bare mention:
        # prose that merely discusses the marker is not a declaration of a break.
        if not any(re.match(r"\+\s*-\s*BREAKING:", line) for line in added):
            raise ContractError(
                "breaking-change requires this pull request to add a prominent "
                "'- BREAKING:' changelog entry under Unreleased"
            )
    print("OK: compatibility-sensitive changes have the required review acknowledgement")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("capture", "check"):
        command = commands.add_parser(name)
        command.add_argument("--binary", required=True, type=Path)
    policy = commands.add_parser("pr-policy")
    policy.add_argument("--base", required=True)
    policy.add_argument("--labels", default="")
    args = parser.parse_args()
    try:
        if args.command == "capture":
            write_contract(args.binary)
        elif args.command == "check":
            check_snapshot(args.binary)
        else:
            pr_policy(args.base, {item.strip() for item in args.labels.split(",") if item.strip()})
        return 0
    except (ContractError, OSError, json.JSONDecodeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
