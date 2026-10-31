---
id: audit
title: ctx audit
sidebar_position: 1
---

# ctx audit

Score code quality across five categories and fail CI below a threshold.

## Synopsis

```bash
ctx audit [OPTIONS]
```

## Description

`ctx audit` analyzes the indexed codebase and prints a report with a score from 0 to 10 per category and overall. Use it to:

- block commits or deploys below a score threshold,
- run quality checks in CI pipelines,
- track the score over time,
- catch issues in a pre-commit hook.

## Prerequisites

Index your codebase first:

```bash
ctx index
```

The complexity and modularity categories use the DuckDB analytics backend, which is part of the default build. The index creates the analytics database on demand.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <FORMAT>` | Output format: text, json, markdown | text |
| `--min-score <SCORE>` | Exit 1 when the overall score is below this value (0.0-10.0) | none |
| `--categories <LIST>` | Categories to check (comma-separated) | all |
| `--incremental` | Limit some categories to changed files (partial, see below) | false |

`--incremental` is only partly implemented, although `ctx audit --help` still calls it "not yet implemented". It collects staged, unstaged, and untracked files from git. The duplication, coverage, and naming categories then look only at symbols in those files, while complexity and modularity still read the whole index. With no changed files the audit reports a score of 10.0 without analyzing anything.

## Categories

| Category | Weight | Description |
|----------|--------|-------------|
| `complexity` | 25% | Function complexity (fan-out) |
| `duplication` | 20% | Functions with similar names (a heuristic, unlike the MinHash search in [`ctx duplicates`](./duplicates.md)) |
| `coverage` | 20% | Documentation coverage |
| `modularity` | 20% | Module coupling |
| `naming` | 15% | Naming convention adherence |

## Examples

### Basic audit

```bash
ctx audit
```

Output:
```
Code Quality Audit
==================

Overall Score: 7.8/10

Categories:
  Complexity:   7.5/10  (5 issues)
  Duplication:  8.0/10  (2 issues)
  Coverage:     6.5/10  (12 issues)
  Modularity:   9.0/10  (1 issues)
  Naming:       8.5/10  (3 issues)

Critical Issues (2):
  [CRIT] src/parser.rs:142 - extract_symbols: fan-out 48 (threshold: 20)
  [CRIT] src/main.rs:809 - run_query: fan-out 45 (threshold: 20)

Warnings (8):
  [WARN] src/db.rs:234 - High coupling: 25 outgoing dependencies
  ...
```

### Quality gate

```bash
# Fail if score below 7.0
ctx audit --min-score 7.0

# In CI, this returns exit code 1 if below threshold
echo $?  # 0 = passed, 1 = failed
```

### Specific categories

```bash
# Only check complexity and naming
ctx audit --categories complexity,naming
```

### JSON output

```bash
# For programmatic processing
ctx audit --output json > audit-report.json
```

### Markdown report

```bash
# For PR comments or documentation
ctx audit --output markdown > AUDIT.md
```

## CI integration

### GitHub Actions

```yaml
name: Code Quality

on: [push, pull_request]

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install ctx
        run: cargo install agentis-ctx
      
      - name: Index codebase
        run: ctx index
      
      - name: Run quality audit
        run: ctx audit --min-score 7.0 --output markdown >> $GITHUB_STEP_SUMMARY
```

### Pre-commit hook

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/agentis-tools/ctx
    rev: v0.4.0
    hooks:
      - id: ctx-audit
        args: [--min-score, "7.0"]
```

Or create a local hook in `.git/hooks/pre-commit`:

```bash
#!/bin/bash
ctx audit --min-score 7.0
```

## Scoring

Each category gets a score from 0.0 to 10.0:

| Score | Quality level |
|-------|---------------|
| 9-10 | Excellent |
| 7-8 | Good |
| 5-6 | Acceptable |
| 3-4 | Needs improvement |
| 0-2 | Poor |

The overall score is the weighted average of the categories.

## Tips

- Start with a threshold you already meet (for example 6.0) and raise it over time.
- Use `--output json` for custom reports.
- Run the audit in CI with `--min-score` as a quality gate.
- Fix critical issues first.

## See also

- [Pre-commit integration](../integrations/ci-cd.md#pre-commit-hooks)
- [CI/CD examples](../integrations/ci-cd.md)
- [Configuration](../configuration.md)
