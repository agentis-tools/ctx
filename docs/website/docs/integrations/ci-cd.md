---
id: ci-cd
title: CI/CD integration
sidebar_position: 1
---

# CI/CD integration

Run ctx in continuous integration to gate merges on code quality, report on pull requests, and track quality over time.

ctx gives CI four things:

- Quality gates that fail a build below a threshold (`ctx audit --min-score`, `ctx check`, `ctx score --fail-on`).
- Context for changed files (`ctx diff`).
- Pre-commit and pre-push hooks.
- Machine-readable reports for other tools (`--output json` on `ctx audit`, `--json` on most other commands).

For the architecture-rule and scorecard gates, see [Quality gates](./quality-gates.md). This page covers `ctx audit`, hooks, and CI plumbing.

## Install ctx in CI

Release binaries are the fastest install. This step downloads the v0.4.0 Linux archive, checks it against the release's `SHA256SUMS`, and puts `ctx` on `PATH`:

```yaml
- name: Install ctx
  env:
    CTX_VERSION: 0.4.0
  run: |
    set -euo pipefail
    artifact="ctx-v${CTX_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
    release_url="https://github.com/agentis-tools/ctx/releases/download/v${CTX_VERSION}"
    cd "${RUNNER_TEMP}"
    curl --fail --location --retry 3 -sS -o "${artifact}" "${release_url}/${artifact}"
    curl --fail --location --retry 3 -sS -o SHA256SUMS "${release_url}/SHA256SUMS"
    grep " ${artifact}\$" SHA256SUMS | sha256sum --check --strict -
    mkdir -p ctx-bin
    tar -xzf "${artifact}" --strip-components=1 -C ctx-bin "${artifact%.tar.gz}/ctx"
    echo "${RUNNER_TEMP}/ctx-bin" >> "${GITHUB_PATH}"
```

To build from source instead, use `cargo install agentis-ctx --locked` after a Rust toolchain step. Neither the release binaries nor a default `cargo install` include the MCP server, which CI does not need.

## Quality gates with ctx audit

`ctx audit` prints a 0.0 to 10.0 score. With `--min-score`, it exits 1 when the score is below the threshold. `--output` selects `text` (default), `json`, or `markdown`.

### GitHub Actions

```yaml
name: Code quality

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      # Insert the "Install ctx" step from above here.

      - name: Index codebase
        run: ctx index

      - name: Run quality audit
        run: ctx audit --min-score 7.0 --output markdown >> "$GITHUB_STEP_SUMMARY"
```

The step summary gets the markdown report, and the job fails when the score is below 7.0.

### GitLab CI

```yaml
quality-audit:
  stage: test
  image: rust:latest
  script:
    - cargo install agentis-ctx --locked
    - ctx index
    - ctx audit --output json > audit.json
    - ctx audit --min-score 7.0
  artifacts:
    when: always
    paths:
      - audit.json
```

The JSON report is ctx's own format (`overall_score`, `categories`, `issues`). It is not a GitLab Code Quality report, so store it as a plain artifact.

### Jenkins

```groovy
pipeline {
    agent any

    stages {
        stage('Quality check') {
            steps {
                sh 'ctx index'
                sh 'ctx audit --min-score 7.0'
            }
        }
    }

    post {
        always {
            sh 'ctx audit --output json > audit.json'
            archiveArtifacts artifacts: 'audit.json'
        }
    }
}
```

## Pre-commit hooks

### pre-commit framework

The ctx repository publishes hook definitions in [`.pre-commit-hooks.yaml`](https://github.com/agentis-tools/ctx/blob/main/.pre-commit-hooks.yaml). The hooks run the `ctx` on your `PATH` (`language: system`), so install ctx first and keep an index in the repository (`ctx index`).

| Hook id | Runs | Default stages |
|---|---|---|
| `ctx-audit` | `ctx audit` | pre-commit, pre-push |
| `ctx-audit-incremental` | `ctx audit --incremental` | pre-commit |
| `ctx-audit-strict` | `ctx audit --min-score 8.0` | pre-push |

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/agentis-tools/ctx
    rev: v0.4.0
    hooks:
      - id: ctx-audit
        args: [--min-score, "7.0"]
```

Install:

```bash
pip install pre-commit
pre-commit install
pre-commit install --hook-type pre-push
```

The hooks do not refresh the index. Run `ctx index` before committing, or run `ctx index -w` in the background.

`ctx audit --incremental` is only partly incremental. It limits the duplication, coverage, and naming checks to files with staged or unstaged changes, but the complexity and modularity checks still read the whole index. With no changed files it reports 10.0 without auditing anything. Its `--help` text still marks the flag as not yet implemented.

### Manual Git hook

Create `.git/hooks/pre-commit`:

```bash
#!/bin/sh
set -e

# Refresh the index (incremental; only changed files are reparsed)
ctx index > /dev/null

ctx audit --min-score 7.0
```

Make it executable:

```bash
chmod +x .git/hooks/pre-commit
```

### Husky (Node.js)

With Husky v9, add the command to `.husky/pre-commit`:

```bash
ctx index > /dev/null && ctx audit --min-score 7.0
```

## Pull request reports

The ctx repository runs ctx on its own pull requests. Two workflows split the work so that fork pull requests are analyzed without giving contributor-controlled code a write-capable token:

1. [`ctx-pr-analysis.yml`](https://github.com/agentis-tools/ctx/blob/main/.github/workflows/ctx-pr-analysis.yml) checks out the PR with read-only permissions, installs a pinned and checksum-verified ctx release, and uploads the complete JSON result bundle.
2. [`ctx-pr-comment.yml`](https://github.com/agentis-tools/ctx/blob/main/.github/workflows/ctx-pr-comment.yml) runs on `workflow_run` after the analysis, checks out its renderer from the trusted default branch, verifies that the analyzed commit is still the PR head, and updates one sticky bot comment.

The comment covers the PR quality delta, the repository-wide audit and statistics, new near-duplicates, architecture-rule findings, hotspots in changed code, and a token-budgeted architecture map. Long lists are capped to fit GitHub's comment size limit. The workflow artifact keeps the full JSON results for 14 days.

Do not replace this with a `pull_request_target` workflow that checks out the PR head. That runs contributor-controlled code with a write token. The publishing workflow downloads only the artifact from the exact run that triggered it and treats every field in it as untrusted input. The [untrusted CI recipe](../cookbook/untrusted-ci.md) explains the pattern in detail.

## Quality trends

For per-commit metrics with history, use [`ctx snapshot`](../commands/snapshot.md), which writes Parquet partitions you can query with `ctx sql --snapshots`.

To chart the audit score with [github-action-benchmark](https://github.com/benchmark-action/github-action-benchmark), convert the report to that action's custom format first:

```yaml
name: Quality metrics

on:
  push:
    branches: [main]

jobs:
  metrics:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7

      # Insert the "Install ctx" step from above here.

      - name: Run audit
        run: |
          ctx index
          ctx audit --output json \
            | jq '[{name: "audit score", unit: "score", value: .overall_score}]
                  + [.categories[] | {name: ("audit " + .name), unit: "score", value: .score}]' \
            > audit-bench.json

      - name: Store metrics
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: customBiggerIsBetter
          output-file-path: audit-bench.json
          github-token: ${{ secrets.GITHUB_TOKEN }}
          auto-push: true
```

A higher audit score is better, so the example uses `customBiggerIsBetter`.

## Category-specific gates

`--categories` restricts the audit to a comma-separated subset of `complexity`, `duplication`, `coverage`, `modularity`, and `naming`.

```yaml
- name: Complexity check
  run: ctx audit --categories complexity --min-score 8.0
```

```yaml
- name: Documentation coverage check
  run: ctx audit --categories coverage --min-score 7.0
```

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Audit passed (score at or above the threshold, or no threshold given) |
| 1 | Audit failed (score below `--min-score`) |
| 2 | Operational error, such as a missing index |

See [Exit codes](../reference/exit-codes.md) for the convention all gating commands share. In a script:

```bash
if ctx audit --min-score 7.0; then
    echo "Quality check passed"
else
    echo "Quality check failed"
    exit 1
fi
```

## Caching

### Cache the index

The index lives in `.ctx/`. Exclude the tracked configuration files so a cache restore never overwrites them:

```yaml
- name: Cache ctx index
  uses: actions/cache@v6
  with:
    path: |
      .ctx
      !.ctx/config.toml
      !.ctx/rules.toml
    key: ctx-index-${{ hashFiles('**/*.rs', '**/*.ts', '**/*.py', '**/*.go') }}
    restore-keys: |
      ctx-index-
```

### Incremental indexing

`ctx index` is incremental by default. It compares content hashes and reparses only files that changed, so a restored cache from an older commit still saves most of the work. Use `ctx index --force` to rebuild from scratch.

## Tips

1. Start with a low threshold, such as 6.0, and raise it over time.
2. Cache the index to shorten CI runs.
3. Use `--categories` to enforce one standard at a time.
4. Use `--output json` to feed the audit into other tools.

## See also

- [Quality gates](./quality-gates.md)
- [ctx audit](../commands/audit.md)
- [ctx diff](../commands/diff.md)
- [Pre-commit hook definitions](https://github.com/agentis-tools/ctx/blob/main/.pre-commit-hooks.yaml)
