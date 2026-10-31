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

`ctx check` loads a rules file, builds a file-level dependency set from the index (resolved call, implements, extends, and uses edges plus resolved imports), evaluates the rules, and reports every violation. Use it to:

- enforce layering, for example forbid the domain layer from importing infrastructure,
- freeze legacy code by forbidding new dependents of modules you plan to remove,
- fail when fan-in, fan-out, complexity, or symbols per file exceed a limit,
- gate pull requests: with `--against`, it reports only violations that touch changed files.

## Prerequisites

Index your codebase first:

```bash
ctx index
```

## Rules file

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

Layers must not overlap. A file that matches the globs of two layers is a configuration error.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--rules <PATH>` | Path to the rules file | `.ctx/rules.toml` |
| `--against <REF>` | Only report violations where at least one endpoint's file changed since REF (for `no_new_dependents`: where the new dependent changed) | none |
| `--list` | Print the parsed rules and layer membership counts, then exit 0 | false |
| `--json` | Machine-readable output (global flag) | false |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | No violations |
| 1 | At least one violation |
| 2 | Operational error (missing or invalid rules file, unknown or overlapping layers, missing index, bad git ref) |

## Examples

### Check all rules

```bash
ctx check
```

Output:
```
forbidden: domain -> infrastructure
  src/domain/order.ts:2 -> src/infra/db.ts [calls query]  (Domain layer must stay persistence-agnostic)

1 violation across 1 rule
```

### Only new violations (PR gate)

```bash
ctx check --against main
```

`ctx check` ignores existing violations in untouched files, so a legacy codebase can adopt rules one change at a time.

### Inspect parsed rules

```bash
ctx check --list
```

### JSON output

```bash
ctx check --against main --json
```

```json
{
  "ctx_version": "0.4.0",
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

See [JSON output](../json-output.md) for the full payload contract.

## Caveats

- Dependencies come from the index: resolved symbol edges plus imports resolved with per-language rules. TS/JS, Solidity, and Zig use paths relative to the importing file; C/C++ quoted includes try the importing directory, the repository root, then a unique indexed suffix, and angle includes resolve only as exact repository-relative paths; Rust uses `crate::`, `self::`, and `super::`; Python uses dotted modules; Go uses package-directory suffixes. ctx skips imports it cannot resolve and third-party imports, so external packages never trigger violations.
- ctx does not track dynamic dispatch, reflection, or macro-generated calls. Rules apply to the statically extracted graph.
- Re-run `ctx index` after changing code. Otherwise the check sees stale dependencies.

## See also

- [ctx score](./score.md): includes `check_violations` as one metric of a combined quality gate
- [Quality gates](../integrations/quality-gates.md)
- [JSON output](../json-output.md)
