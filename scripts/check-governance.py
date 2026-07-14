#!/usr/bin/env python3
"""Repository policy checks that do not belong in the product test suite."""

from __future__ import annotations

import argparse
import datetime as dt
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = (
    "governance/current-state.md",
    "governance/versioning.md",
    "governance/releasing.md",
    "governance/guardrails.md",
    "governance/agent-workflow.md",
    "governance/cookbook-authoring.md",
)
FORBIDDEN_DOCS = (
    "docs/versioning.md", "docs/releasing.md", "docs/guardrails.md",
    "docs/agent-workflow.md",
)
AGENT_MARKER = "<!-- governance-instructions:v1 -->"


class GovernanceError(RuntimeError):
    pass


def cookbook_structure_errors(root: Path) -> list[str]:
    cookbook = root / "docs/website/docs/cookbook"
    errors: list[str] = []
    exempt = {"index.md", "concepts.md"}
    required_headings = (
        "## Quickest version",
        "## What worked, and what did not",
        "## Give the workflow to an agent",
    )
    for recipe in sorted(cookbook.glob("*.md")):
        text = recipe.read_text(encoding="utf-8")
        if re.search(r"\]\((?!https?://)[^)]*\.md(?:#[^)]*)?\)", text):
            errors.append(
                f"{recipe.relative_to(root)} must use extensionless internal doc links"
            )
        if recipe.name in exempt:
            continue
        for heading in required_headings:
            if heading not in text:
                errors.append(
                    f"{recipe.relative_to(root)} must include a '{heading}' section"
                )
    return errors


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=ROOT, text=True, capture_output=True
    )
    if result.returncode:
        raise GovernanceError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout


def structural_check() -> None:
    errors: list[str] = []
    for relative in REQUIRED:
        if not (ROOT / relative).is_file():
            errors.append(f"missing internal policy file {relative}")
    for relative in FORBIDDEN_DOCS:
        if (ROOT / relative).exists():
            errors.append(f"internal policy must not be stored in product docs: {relative}")
    for relative in ("AGENTS.md", "CLAUDE.md"):
        text = (ROOT / relative).read_text(encoding="utf-8")
        if AGENT_MARKER not in text:
            errors.append(f"{relative} is missing {AGENT_MARKER}")
        for required in ("governance/agent-workflow.md", "governance/versioning.md"):
            if required not in text:
                errors.append(f"{relative} must link to {required}")
        if "docs/ is public product documentation" not in text:
            errors.append(f"{relative} must state the docs/governance boundary")
    for relative in REQUIRED[1:]:
        text = (ROOT / relative).read_text(encoding="utf-8")
        if "docs/" not in text and relative == "governance/guardrails.md":
            errors.append("guardrails.md must state the documentation boundary")
    errors.extend(cookbook_structure_errors(ROOT))
    action_files = list((ROOT / ".github/workflows").glob("*.yml"))
    action_files += list((ROOT / ".github/workflows").glob("*.yaml"))
    action_files += list((ROOT / ".github/actions").glob("**/action.yml"))
    action_files += list((ROOT / ".github/actions").glob("**/action.yaml"))
    for workflow in sorted(action_files):
        for line_number, line in enumerate(
            workflow.read_text(encoding="utf-8").splitlines(), 1
        ):
            match = re.search(r"\buses:\s*([^\s#]+)", line)
            if not match or match.group(1).startswith("./"):
                continue
            reference = match.group(1).rsplit("@", 1)[-1]
            if not re.fullmatch(r"[0-9a-f]{40}", reference):
                errors.append(
                    f"{workflow.relative_to(ROOT)}:{line_number} action must be pinned "
                    "to a full commit SHA"
                )
    deny_lines = (ROOT / "deny.toml").read_text(encoding="utf-8").splitlines()
    today = dt.datetime.now(dt.timezone.utc).date()
    for line_number, line in enumerate(deny_lines):
        if not re.search(r'"RUSTSEC-\d{4}-\d{4}"', line):
            continue
        window = "\n".join(deny_lines[max(0, line_number - 4) : line_number + 5])
        dates = re.findall(r"\b\d{4}-\d{2}-\d{2}\b", window)
        if not dates:
            errors.append(
                f"deny.toml:{line_number + 1} advisory exception needs a review deadline"
            )
            continue
        try:
            deadline = max(dt.date.fromisoformat(value) for value in dates)
        except ValueError as error:
            errors.append(f"deny.toml:{line_number + 1} has invalid deadline: {error}")
            continue
        if deadline < today:
            errors.append(
                f"deny.toml:{line_number + 1} advisory exception expired {deadline}"
            )
    if errors:
        raise GovernanceError("\n".join(errors))
    print("OK: governance files and agent instructions respect the docs boundary")


def unreleased_body(text: str) -> str:
    start = text.find("## [Unreleased]")
    if start < 0:
        return ""
    end = text.find("\n## [", start + 1)
    return text[start : end if end >= 0 else len(text)]


def unreleased_entries(text: str) -> set[str]:
    return {
        line.strip()
        for line in unreleased_body(text).splitlines()
        if line.strip().startswith("- ")
    }


def manifest_version(text: str) -> str:
    try:
        value = tomllib.loads(text)["package"]["version"]
    except (tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise GovernanceError(f"cannot read [package].version: {error}") from error
    if not isinstance(value, str):
        raise GovernanceError("[package].version must be a string")
    return value


def pr_check(base: str, labels: set[str]) -> None:
    changed = set(git("diff", "--name-only", f"{base}...HEAD").splitlines())
    version_changed = False
    if "Cargo.toml" in changed:
        current_manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        base_manifest = git("show", f"{base}:Cargo.toml")
        version_changed = manifest_version(current_manifest) != manifest_version(base_manifest)
    product_change = any(
        path == "Cargo.toml"
        or path.startswith("src/")
        or path.startswith(".ctx/")
        or path.startswith("docs/")
        or path in {"README.md", "install.sh"}
        or path.startswith("scripts/release-")
        or path == ".github/workflows/release.yml"
        for path in changed
    )
    if product_change and not version_changed and "skip-changelog" not in labels:
        if "CHANGELOG.md" not in changed:
            raise GovernanceError(
                "user-visible/product changes require an Unreleased CHANGELOG.md entry; "
                "a maintainer may apply skip-changelog only for genuinely internal changes"
            )
        current_text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
        base_text = git("show", f"{base}:CHANGELOG.md")
        added = unreleased_entries(current_text) - unreleased_entries(base_text)
        if not added:
            raise GovernanceError(
                "product changes require a newly added Unreleased changelog bullet"
            )
    if version_changed and "release-preparation" not in labels:
        raise GovernanceError(
            "authoritative version changes require maintainer label release-preparation"
        )
    print("OK: PR changelog and version-change policy satisfied")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check")
    pr = commands.add_parser("pr")
    pr.add_argument("--base", required=True)
    pr.add_argument("--labels", default="")
    args = parser.parse_args()
    try:
        structural_check()
        if args.command == "pr":
            pr_check(
                args.base,
                {item.strip() for item in args.labels.split(",") if item.strip()},
            )
        return 0
    except (GovernanceError, OSError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
