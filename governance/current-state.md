# Repository governance baseline

This inventory records the repository state observed before the governance
system was introduced. It is an internal maintainer record, not product
documentation.

## Versioning and packaging

- The repository is not a Cargo workspace. The root is one publishable package,
  `agentis-ctx`, which owns both the `ctx` binary and the `ctx` library.
- `perf/` is a separate, explicitly isolated Cargo package named `ctx-perf`.
  It is versioned `0.0.0`, is not published, and depends on the root package by
  path.
- `Cargo.toml` `[package].version` is the existing and most appropriate
  authoritative version. `Cargo.lock` and `perf/Cargo.lock` repeat the resolved
  local package version. Runtime version strings use `CARGO_PKG_VERSION`.
- `ctx --version`, JSON envelopes, MCP server metadata, snapshots, gate logs,
  generated harness files, plugin manifests, and self-update HTTP headers all
  embed the Cargo package version.
- During implementation, `main` added committed Homebrew, AUR, Scoop, Debian,
  and RPM packaging definitions. The committed Homebrew/AUR/Scoop files are a
  checked historical release snapshot; release CI deterministically generates
  and validates definitions for the tagged version from `SHA256SUMS`. Plugin
  manifests are generated during release and checked against the tag version.
- Rust 1.91 is the declared minimum supported Rust version. The crate uses Rust
  2021 edition.

## Release flow

- Releases are triggered only by pushing a `v*` Git tag.
- The existing release workflow compares the tag with Cargo metadata, tests,
  builds four target archives, packages Claude/Codex plugins and Linux native
  packages, publishes the crate, and creates a GitHub Release.
- Archives have per-file checksums and the GitHub Release contains an aggregate
  `SHA256SUMS`, which `ctx self-update` verifies.
- GitHub-generated notes, rather than the reviewed `CHANGELOG.md` section, were
  used as release notes. Crate publication and GitHub Release creation could
  run independently, allowing inconsistent partial outcomes.
- `CHANGELOG.md` follows Keep a Changelog and has an Unreleased section, but no
  deterministic tool validated headings, links, empty releases, or tag state.

## Compatibility and quality surfaces

- The published Rust library is a public API surface. No semantic-compatibility
  comparison against `main` existed.
- CLI commands/flags/defaults, exit codes, configuration, JSON envelopes,
  SQLite index schema, snapshot/SQL schemas, MCP tools, generated plugins, and
  self-update artifact naming are compatibility-sensitive.
- CI ran formatting, Clippy, a three-platform test matrix, publish dry-runs,
  and an advisory performance harness. It did not validate version/changelog
  policy, dependency advisories, licenses, dependency sources, or release
  provenance.
- Architecture checks and score gates exist in ctx itself, but this repository
  currently commits only `.ctx/config.toml`, not `.ctx/rules.toml`; therefore
  no repository architecture rules were enforced.
- Generated documentation build output and `.ctx` indexes are ignored. Cargo
  lockfiles, the public website source, benchmark schema, and performance
  baselines are committed.

## Repository settings and documentation boundary

- The GitHub API reported that `main` had no branch protection configured at
  the time of this inventory. Repository files cannot enforce review counts,
  force-push restrictions, or required-check selection; those remain explicit
  administrator actions documented in `guardrails.md`.
- `docs/` is the public ctx manual and website source. No internal policy
  directory existed. Internal policy is therefore introduced only under
  `governance/`.
- Root `AGENTS.md` was auto-generated and contained malformed command guidance.
  `CLAUDE.md` covered ctx/Linear usage but no release or compatibility workflow.

## Minimal target architecture

Keep `Cargo.toml` as the sole manually edited version source. Add one standard
library for version/changelog operations, a deterministic checker and release
note renderer, focused tests, a PR policy workflow, dependency/license/source
checks, Rust API compatibility checks, hardened tag release gates, checksummed
artifacts with GitHub provenance attestations, and concise contributor intent
in `AGENTS.md`/`CLAUDE.md`. Contracts that cannot be classified safely by a
machine are routed to explicit labelled human review rather than described as
automatically enforced.
