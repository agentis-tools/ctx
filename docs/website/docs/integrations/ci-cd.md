---
id: ci-cd
title: CI/CD Integration
sidebar_position: 1
---

# CI/CD Integration

Integrate ctx into your continuous integration and deployment pipelines.

## Overview

ctx provides several features useful in CI/CD:

- **Quality Gates** - Block merges/deploys below quality thresholds
- **Change Analysis** - Generate context for changed files
- **Pre-commit Hooks** - Catch issues before committing
- **Documentation** - Auto-generate change documentation

## Quality Gates with ctx audit

### GitHub Actions

```yaml
name: Code Quality

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      
      - name: Install ctx
        run: cargo install agentis-ctx --locked
      
      - name: Index codebase
        run: ctx index
      
      - name: Run quality audit
        run: |
          ctx audit --min-score 7.0 --output markdown >> $GITHUB_STEP_SUMMARY
```

### GitLab CI

```yaml
quality-audit:
  stage: test
  script:
    - cargo install agentis-ctx
    - ctx index
    - ctx audit --min-score 7.0 --output json > audit.json
  artifacts:
    reports:
      codequality: audit.json
```

### Jenkins

```groovy
pipeline {
    agent any
    
    stages {
        stage('Quality Check') {
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

## Pre-commit Hooks

> **Note:** `ctx audit --incremental` (auditing only changed files) is not yet
> implemented. The examples below run the full audit, which is fast for most
> projects once the index is built.

### Using pre-commit Framework

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/agentis-tools/ctx
    rev: v0.3.0
    hooks:
      - id: ctx-audit
        args: [--min-score, "7.0"]
```

Install:
```bash
pip install pre-commit
pre-commit install
```

### Manual Git Hooks

Create `.git/hooks/pre-commit`:

```bash
#!/bin/bash
set -e

# Ensure index is up to date
ctx index

# Run audit
ctx audit --min-score 7.0

echo "Quality check passed!"
```

Make executable:
```bash
chmod +x .git/hooks/pre-commit
```

### Husky (Node.js)

Add to `package.json`:

```json
{
  "husky": {
    "hooks": {
      "pre-commit": "ctx audit --min-score 7.0"
    }
  }
}
```

## PR Context Generation

### Auto-generate PR Context

```yaml
name: PR Context

on:
  pull_request:
    types: [opened, synchronize]

jobs:
  context:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      
      - name: Install ctx
        run: cargo install agentis-ctx
      
      - name: Index codebase
        run: ctx index
      
      - name: Generate change context
        run: |
          echo "## Changed Files Context" >> $GITHUB_STEP_SUMMARY
          ctx diff ${{ github.event.pull_request.base.sha }} --summary >> $GITHUB_STEP_SUMMARY
```

### Comment on PRs

```yaml
- name: Comment context
  uses: actions/github-script@v6
  with:
    script: |
      const { execSync } = require('child_process');
      const context = execSync('ctx diff origin/main --format markdown').toString();
      
      github.rest.issues.createComment({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        body: `## Code Context\n\n${context}`
      });
```

## Quality Trends

### Track Quality Over Time

```yaml
name: Quality Metrics

on:
  push:
    branches: [main]

jobs:
  metrics:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Run audit
        run: |
          ctx index
          ctx audit --output json > audit.json
      
      - name: Store metrics
        uses: benchmark-action/github-action-benchmark@v1
        with:
          tool: 'customSmallerIsBetter'
          output-file-path: audit.json
          github-token: ${{ secrets.GITHUB_TOKEN }}
```

## Category-Specific Gates

### Strict Complexity Check

```yaml
- name: Complexity check
  run: ctx audit --categories complexity --min-score 8.0
```

### Documentation Coverage

```yaml
- name: Doc coverage check
  run: ctx audit --categories coverage --min-score 7.0
```

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Audit passed (score >= threshold) |
| 1 | Audit failed (score < threshold) |

Use in scripts:

```bash
if ctx audit --min-score 7.0; then
    echo "Quality check passed"
else
    echo "Quality check failed"
    exit 1
fi
```

## Caching

### Cache the Index

```yaml
- name: Cache ctx index
  uses: actions/cache@v3
  with:
    path: .ctx
    key: ctx-index-${{ hashFiles('**/*.rs', '**/*.ts', '**/*.py', '**/*.go') }}
    restore-keys: |
      ctx-index-
```

### Incremental Indexing

```bash
# Only reindex changed files
ctx index  # Automatically incremental
```

## Tips

1. **Start lenient** - Begin with a low threshold (6.0) and increase over time
2. **Cache the index** - Speeds up CI significantly
3. **Category focus** - Use `--categories` to enforce specific standards
4. **JSON output** - Use `--output json` for integration with other tools
5. **Incremental indexing** - `ctx index` only reprocesses changed files automatically

## See Also

- [Audit Command](../commands/audit.md)
- [Diff Command](../commands/diff.md)
- [Pre-commit hooks definition](https://github.com/agentis-tools/ctx/blob/main/.pre-commit-hooks.yaml)
