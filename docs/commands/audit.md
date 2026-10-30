# ctx audit

Run automated code quality analysis with scoring for CI integration.

## Synopsis

```bash
ctx audit [OPTIONS]
```

## Description

The `audit` command analyzes your codebase for quality metrics and generates a report with scores. Use it to:

- **Quality gates** - Block commits/deploys below a threshold
- **CI integration** - Automated quality checks in pipelines  
- **Progress tracking** - Monitor code quality over time
- **Pre-commit hooks** - Catch issues before committing

## Prerequisites

Index your codebase first:

```bash
ctx index
```

For full analysis (complexity, modularity), the index creates the analytics database automatically.

The report's statistics are loaded directly from the indexed symbol table and
checked against the database counts. If the index is inconsistent, `ctx audit`
fails instead of presenting empty-data perfect scores.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --output <FORMAT>` | Output format: text, json, markdown | text |
| `--min-score <SCORE>` | Minimum score threshold (0.0-10.0) | none |
| `--categories <LIST>` | Categories to check (comma-separated) | all |
| `--incremental` | Only audit changed files | false |

## Categories

| Category | Weight | Description |
|----------|--------|-------------|
| `complexity` | 25% | Function complexity (fan-out) |
| `duplication` | 20% | Code duplication patterns |
| `coverage` | 20% | Documentation coverage |
| `modularity` | 20% | Module coupling |
| `naming` | 15% | Naming convention adherence |

## Examples

### Basic Audit

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

### Quality Gate

```bash
# Fail if score below 7.0
ctx audit --min-score 7.0

# In CI, this returns exit code 1 if below threshold
echo $?  # 0 = passed, 1 = failed
```

### Specific Categories

```bash
# Only check complexity and naming
ctx audit --categories complexity,naming
```

### JSON Output

```bash
# For programmatic processing
ctx audit --output json > audit-report.json
```

### Markdown Report

```bash
# For PR comments or documentation
ctx audit --output markdown > AUDIT.md
```

### Incremental Audit

```bash
# Only audit changed files (fast for pre-commit)
ctx audit --incremental
```

## CI Integration

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
        run: cargo install --path .
      
      - name: Index codebase
        run: ctx index
      
      - name: Run quality audit
        run: ctx audit --min-score 7.0 --output markdown >> $GITHUB_STEP_SUMMARY
```

### Pre-commit Hook

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/yourusername/ctx
    rev: v0.2.0
    hooks:
      - id: ctx-audit-incremental
        args: [--min-score, "7.0"]
```

Or create a local hook in `.git/hooks/pre-commit`:

```bash
#!/bin/bash
ctx audit --incremental --min-score 7.0
```

## Scoring

Each category produces a score from 0.0 to 10.0:

| Score | Quality Level |
|-------|---------------|
| 9-10 | Excellent |
| 7-8 | Good |
| 5-6 | Acceptable |
| 3-4 | Needs Improvement |
| 0-2 | Poor |

The overall score is a weighted average of all categories.

## Tips

- Start with a realistic threshold (e.g., 6.0) and increase over time
- Use `--incremental` for pre-commit hooks (faster)
- Use `--output json` for custom reporting
- Run full audit in CI, incremental locally
- Focus on critical issues first

## See Also

- [Pre-commit Integration](../integrations/ci-cd.md#pre-commit)
- [CI/CD Examples](../integrations/ci-cd.md)
- [Configuration](../configuration.md)
