---
id: duplicates
title: ctx duplicates
sidebar_position: 8
---

# ctx duplicates

Detect structurally similar functions with MinHash near-duplicate search.

## Synopsis

```bash
ctx duplicates [OPTIONS]
```

## Description

The `duplicates` command compares MinHash fingerprints (built during `ctx index`) of every indexed function and method, including C/C++ and Zig functions and methods, and reports pairs whose normalized token shingles have a Jaccard similarity at or above the threshold.

Tokens are normalized before comparison - identifiers become `ID`, string and number literals become `LIT`, comments are dropped - so **renamed variables and changed literals still match**. Candidate pairs are found with LSH banding over 128-permutation MinHash signatures, then verified with the exact Jaccard similarity.

All indexed languages participate, **including C, C++, Zig, and Solidity** (Solidity is tokenized via the solang-parser lexer; the Tree-sitter languages normalize identifiers and literals and discard comments).

> **Breaking change:** this replaces the old line-based detector. `--threshold` is a 0.0-1.0 Jaccard similarity over normalized 5-token shingles, **not** a percentage of matching lines; the old `--similarity <PERCENT>` / `--min-lines <N>` flags are gone. Existing indexes lack fingerprints: rebuild once with `ctx index --force` after upgrading.

## Prerequisites

Fingerprints are computed during indexing:

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--threshold <F>` | Jaccard similarity threshold (0.0-1.0) over normalized token shingles. Values below 0.5 are clamped to 0.5 (LSH candidate detection is unreliable below that) | 0.85 |
| `--min-tokens <N>` | Ignore functions with fewer than N normalized tokens | 50 |
| `--against <REF>` | Only report pairs where at least one function is in a file changed relative to REF | none |
| `--fail-on-found` | Exit 1 when any near-duplicate pair is reported | false |
| `--json` | Machine-readable output (global flag) | false |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (default mode is informational, even with pairs found) |
| 1 | `--fail-on-found` was given and at least one pair was reported |
| 2 | Operational error (missing index, bad git ref, invalid threshold) |

## Examples

### Find Near-Duplicates

```bash
ctx duplicates
```

Output:
```
Near-duplicate functions (Jaccard similarity of 5-token shingles >= 0.85, >= 50 tokens)
====================================================================================================

1. similarity 0.952
   src/orders.rs:12 process_orders (64 tokens)
   src/invoices.rs:4 sum_invoices (64 tokens)

----------------------------------------------------------------------------------------------------
Found 1 near-duplicate pair(s).
```

### Only Pairs Touching Changed Files

```bash
ctx duplicates --against main
```

### CI Gate

```bash
ctx duplicates --against main --fail-on-found
```

### JSON Output

```bash
ctx duplicates --json
```

```json
{
  "ctx_version": "0.3.0",
  "command": "duplicates",
  "generated_at": "2026-07-09T12:00:00Z",
  "data": {
    "threshold": 0.85,
    "min_tokens": 50,
    "against": null,
    "skipped_languages": [],
    "pairs": [
      {
        "a": { "name": "process_orders", "qualified_name": null, "kind": "function", "file": "src/orders.rs", "line_start": 12, "line_end": 30 },
        "b": { "name": "sum_invoices", "qualified_name": null, "kind": "function", "file": "src/invoices.rs", "line_start": 4, "line_end": 22 },
        "similarity": 0.97,
        "token_count_a": 64,
        "token_count_b": 64
      }
    ]
  }
}
```

## Caveats

- **Idiomatic boilerplate** (builders, trait impls, small CRUD handlers) can legitimately look structurally similar; raise `--min-tokens` to filter short functions.
- Nested functions share tokens with their enclosing function, so both can appear in results.
- Fingerprints are built at index time: reindex before running this command after code changes.

## See Also

- [ctx score](./score.md) - counts *newly introduced* duplication as a gate metric
- [Quality Gates](../integrations/quality-gates.md)
- [JSON Output](../json-output.md)
