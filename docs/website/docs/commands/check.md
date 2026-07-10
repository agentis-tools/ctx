---
id: check
title: ctx check
sidebar_position: 6
---

# ctx check

Enforce architecture rules from `.ctx/rules.toml` against the code intelligence index.

## Synopsis

```bash
ctx check [OPTIONS]
```

## Description

The `check` command loads a declarative rules file, builds a file-level dependency set from the index (resolved call/implements/extends/uses edges plus resolved imports), evaluates the rules, and reports every violation. Use it to:

- **Enforce layering** - e.g. the domain layer must never import infrastructure
- **Freeze legacy code** - forbid new dependents of modules slated for removal
- **Cap complexity** - fail when fan-in, fan-out, complexity, or file symbol counts exceed limits
- **Gate PRs** - with `--against`, only violations touching changed files are reported

## Prerequisites

Index your codebase first:

```bash
ctx index
```

## Rules File

Rules live in `.ctx/rules.toml` (override with `--rules`):

```toml
version = 1

[layers]                                   # layer name -> globs over indexed files
domain         = ["src/domain/**"]
application    = ["src/app/**"]
infrastructure = ["src/infra/**", "src/db/**"]

[[rules.forbidden]]                        # `from` must not depend on `to`
from   = "domain"
to     = "infrastructure"
reason = "Domain layer must stay persistence-agnostic"

[[rules.allowed_dependents]]               # only `only` may depend on `layer`
layer = "infrastructure"                   # (files in no layer are exempt)
only  = ["application"]

[[rules.limit]]                            # metric thresholds
metric  = "fan_in"                         # fan_in | fan_out | complexity | file_symbols
scope   = "symbol"                         # symbol | file
max     = 25
exclude = ["src/core/**"]

[[rules.no_new_dependents]]                # frozen paths
paths  = ["src/legacy/**"]
reason = "Legacy module is frozen; do not add new callers"
```

Layers must not overlap: a file matching two layers' globs is a configuration error.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--rules <PATH>` | Path to the rules file | `.ctx/rules.toml` |
| `--against <REF>` | Only report violations where at least one endpoint's file changed since REF (for `no_new_dependents`: where the new dependent changed) | none |
| `--list` | Print the parsed rules and layer membership counts, then exit 0 | false |
| `--json` | Machine-readable output (global flag) | false |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | No violations |
| 1 | At least one violation |
| 2 | Operational error (missing/invalid rules file, unknown or overlapping layers, missing index, bad git ref) |

## Examples

### Check All Rules

```bash
ctx check
```

Output:
```
forbidden: domain -> infrastructure
  src/domain/order.ts:2 -> src/infra/db.ts [calls query]  (Domain layer must stay persistence-agnostic)

1 violation across 1 rule
```

### Only New Violations (PR gate)

```bash
ctx check --against main
```

Pre-existing violations in untouched files are ignored, so a legacy codebase can adopt rules incrementally.

### Inspect Parsed Rules

```bash
ctx check --list
```

### JSON Output

```bash
ctx check --against main --json
```

```json
{
  "ctx_version": "0.3.0",
  "command": "check",
  "generated_at": "2026-07-09T12:00:00Z",
  "data": {
    "rules_path": ".ctx/rules.toml",
    "against": "main",
    "summary": { "violations": 1, "rules_violated": 1 },
    "violations": [
      {
        "rule": "forbidden",
        "rule_id": "forbidden: domain -> infrastructure",
        "reason": "Domain layer must stay persistence-agnostic",
        "message": "src/domain/order.ts:2 -> src/infra/db.ts [calls query]",
        "file": "src/domain/order.ts",
        "line": 2,
        "from": { "file": "src/domain/order.ts" },
        "to": { "file": "src/infra/db.ts" }
      }
    ]
  }
}
```

See [JSON Output](../json-output.md) for the full payload contract.

## Caveats

- Dependencies come from the index: resolved symbol edges plus imports resolved with per-language heuristics (relative paths for TS/JS/Solidity, `crate::`/`self::`/`super::` for Rust, dotted modules for Python, package-directory suffixes for Go). Unresolvable or third-party imports are silently skipped, so external packages never trigger violations.
- Dynamic dispatch, reflection, and macro-generated calls are not tracked; rules are enforced on the statically extracted graph.
- Re-run `ctx index` after changing code, or the check sees stale dependencies.

## See Also

- [ctx score](./score.md) - includes `check_violations` as one metric of a combined quality gate
- [Quality Gates](../integrations/quality-gates.md)
- [JSON Output](../json-output.md)
