# Repository guardrails

## Enforced in repository code and CI

- `scripts/version.py check` validates SemVer, both lockfiles, Cargo metadata,
  compiled `ctx --version`, tag naming, changelog state, and self-update/release
  conventions.
- `scripts/check-contracts.py` makes the committed CLI surface reproducible.
  Removed commands/flags require `breaking-change`, `contract-review`, the
  correct SemVer increase, and a `BREAKING:` entry.
- `cargo-semver-checks` compares the published Rust library surface with the PR
  base. Intentional exceptions require a reviewed baseline/version decision;
  the check is not advisory.
- Compatibility-sensitive source paths require `contract-review`, even where
  the CLI snapshot has no detectable removal.
- CI enforces rustfmt, Clippy warnings-as-errors, tests on Linux/macOS/Windows,
  all/minimal feature combinations, version/governance tests, publish dry-runs,
  changelog policy, dependency advisories/licenses/sources, and contract state.
- Release CI repeats stronger gates; validates platform archives, generated
  plugins, Debian/RPM packages, and generated package-manager definitions;
  publishes checksums; and creates GitHub artifact provenance attestations.
- Dependabot proposes weekly Cargo and GitHub Actions updates. `deny.toml`
  permits only approved licenses, denies unknown registries/git sources, and
  checks advisories and duplicate/wildcard dependency policy.
- CODEOWNERS routes workflows, governance, release scripts, Cargo metadata,
  compatibility surfaces, and security policy to maintainers.

No prose-only rule above is described as enforced unless a script/workflow
implements it.

## Human-review requirements

Automation cannot determine every semantic break. Maintainers must review
default behavior, human/JSON semantics, configuration precedence, migrations,
MCP/plugin behavior, exit codes, platform support, package installation, and
self-update behavior. Security disclosure timing and contributor attribution
also require judgment. Labels are acknowledgements, not proof of compatibility.

Tag signature verification is recommended but not enforced because no durable
maintainer keyring is committed. Performance CI remains advisory until its
baseline policy is explicitly promoted.

## GitHub repository settings

At baseline, the GitHub API reported `main` was not protected. These settings
cannot be enforced by committed files. A repository administrator must create
a branch ruleset that:

- requires pull requests and at least one approving review;
- requires CODEOWNERS review and dismissal after new commits;
- requires conversation resolution and linear history;
- blocks force pushes, deletion, and direct pushes (including administrators
  except documented emergency bypass);
- requires the CI, Policy, docs, and ctx-analysis checks selected after these
  workflows land;
- requires branches to be current before merge;
- restricts tag creation/deletion for `v*` to release maintainers.

The `release` deployment environment must require release-maintainer approval
and limit deployments to protected `v*` tags. Label permissions must also be
limited to trusted contributors: CI validates required label names, but a
label alone cannot prove who acknowledged the risk.

Administrators must audit those settings after workflow/check names change.
Until configured, repository-local CI can report failures but cannot prevent a
privileged direct push; this document deliberately does not claim otherwise.

## Documentation boundary

`docs/` is public product documentation and the user manual. Maintainer
procedure, CI policy, architecture governance, and agent operating rules live
only in `governance/`, root agent files, scripts, and workflows. The structural
governance check prevents the named policy files from being placed in `docs/`.
