# AI and human contributor workflow

## Before editing

Read root `AGENTS.md`, `CLAUDE.md`, this file, `versioning.md`, and any policy
relevant to the task. Inspect the worktree and preserve unrelated changes.
Use a feature branch/worktree for release or governance changes. `docs/` is
public product documentation; internal policy belongs in `governance/`.

Use ctx for discovery when its index is available, but verify generated or
security-sensitive conclusions against source. Do not edit generated harness
hooks, Cargo lockfiles, plugin manifests, checksums, release notes, or CLI
contract snapshots by hand when their generator exists.

Cookbook additions and structural revisions must follow
[`cookbook-authoring.md`](cookbook-authoring.md). Public recipes remain in `docs/`; the contributor
contract and verification requirements remain in `governance/`.

## While changing contracts

- Add a categorized Unreleased changelog entry for product behavior. Internal
  changes may use a maintainer-approved `skip-changelog` label.
- Treat CLI, JSON, config, schemas/indexes, library API, MCP/plugins, exits,
  packaging/platforms, and self-update as compatibility surfaces.
- Run `scripts/check-contracts.py capture --binary target/release/ctx` only after
  reviewing the change. Snapshot updates never make a breaking change safe by
  themselves.
- Do not change the product version during ordinary feature/fix work. Release
  preparation uses `scripts/version.py` and the `release-preparation` label.
- Never create/push tags, publish crates, create releases, or change repository
  settings without explicit authorization.
- Never weaken/skip a gate to make CI green. Document sandbox/environment
  limitations and run the closest deterministic subset instead.

## Required validation

Run the narrowest relevant tests while iterating, then `scripts/ci.sh` before
handoff when the environment supports it. At minimum run formatting, Clippy,
relevant tests, `scripts/check-agent-docs.sh`, version checks, and contract
checks for the touched surfaces. Report any check not run or not enforceable.

Release changes additionally require `scripts/release-check.sh v<version>`.
Review generated notes and artifact names. Scripts never authorize a publish.

## Handoff

State what changed, compatibility/version impact, changelog impact, tests,
remaining human review, and external settings not enforced from the repo. Do
not claim branch protection, signatures, semantic compatibility, or provenance
unless the corresponding external check actually succeeded.
