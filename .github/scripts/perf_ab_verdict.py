#!/usr/bin/env python3
"""Decide the perf gate from a same-runner A/B run.

CI runs the perf harness twice on one runner: first with the base binary
(the pull request's base, the previous main commit, or the latest release),
then with the head binary, using the base results as the regression baseline.
Comparing two binaries on the same machine removes runner-hardware variance,
which a committed baseline cannot: hosted `ubuntu-latest` machines differ by
~1.4x, enough to fail a 1.2x regression gate with no code change.

Verdict per head scenario (harness statuses are taken as given):
  * a regression note (head > base * 1.2) fails;
  * a budget or RSS note fails only if the base run did not already exceed
    the same limit -- otherwise the overage predates this change and is
    reported as a warning;
  * every other status passes.

Usage: perf_ab_verdict.py BASE_RESULTS HEAD_RESULTS [--summary FILE]
Exit status: 0 pass, 1 fail, 2 bad input.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

LIMIT_KINDS = ("budget", "rss")


def _kinds(notes: list[str]) -> set[str]:
    """Failure kinds named by harness notes (`regression: ...`, `budget: ...`)."""
    return {note.split(":", 1)[0].strip() for note in notes if ":" in note}


def verdict(base: dict, head: dict) -> tuple[bool, list[tuple[str, str, str]]]:
    """Return (failed, rows) where rows are (scenario, outcome, detail)."""
    base_scenarios = base.get("scenarios", {})
    rows: list[tuple[str, str, str]] = []
    failed = False
    for name, result in sorted(head.get("scenarios", {}).items()):
        if result.get("status") != "fail":
            rows.append((name, result.get("status", "?"), ""))
            continue
        head_kinds = _kinds(result.get("notes", []))
        base_kinds = _kinds(base_scenarios.get(name, {}).get("notes", []))
        new = {k for k in head_kinds if k == "regression" or k not in base_kinds}
        detail = "; ".join(result.get("notes", []))
        if new:
            failed = True
            rows.append((name, "fail", detail))
        else:
            rows.append((name, "warn (also over limit on base)", detail))
    return failed, rows


def markdown(rows: list[tuple[str, str, str]], base_label: str) -> str:
    lines = [
        f"### Perf A/B against {base_label} (same runner)\n",
        "| scenario | outcome | detail |",
        "| --- | --- | --- |",
    ]
    lines += [f"| `{name}` | {outcome} | {detail} |" for name, outcome, detail in rows]
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("base")
    parser.add_argument("head")
    parser.add_argument("--summary")
    parser.add_argument("--base-label", default="base")
    args = parser.parse_args(argv)
    try:
        base = json.loads(Path(args.base).read_text())
        head = json.loads(Path(args.head).read_text())
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    failed, rows = verdict(base, head)
    table = markdown(rows, args.base_label)
    print(table)
    if args.summary:
        with open(args.summary, "a", encoding="utf-8") as out:
            out.write(table)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
