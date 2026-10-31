# ctx duplicates

Detect structurally similar functions with MinHash near-duplicate search.

## Synopsis

```bash
ctx duplicates [OPTIONS]
```

## Description

`ctx duplicates` compares the MinHash fingerprints that `ctx index` builds for every indexed function and method, and reports pairs whose normalized token shingles have a Jaccard similarity at or above the threshold.

ctx normalizes tokens before comparison. Identifiers become `ID`, string and number literals become `LIT`, and comments are dropped, so renamed variables and changed literals still match. LSH banding over 128-permutation MinHash signatures finds candidate pairs, and ctx then verifies each with the exact Jaccard similarity.

All fully indexed languages participate, including C, C++, Zig, and Solidity. Solidity uses the solang-parser lexer; the Tree-sitter languages use their own grammars.

> Upgrading from the old line-based detector: `--threshold` is a 0.0-1.0 Jaccard similarity over normalized 5-token shingles, not a percentage of matching lines, and the old `--similarity <PERCENT>` and `--min-lines <N>` flags are gone. Indexes built before the change lack fingerprints, so rebuild once with `ctx index --force`.

## Prerequisites

`ctx index` computes the fingerprints:

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--threshold <F>` | Jaccard similarity threshold (0.0-1.0) over normalized token shingles. Values below 0.5 are clamped to 0.5, because LSH candidate detection is unreliable below that | 0.85 |
| `--min-tokens <N>` | Ignore functions with fewer than N normalized tokens | 50 |
| `--against <REF>` | Only report pairs where at least one function is in a file changed relative to REF | none |
| `--fail-on-found` | Exit 1 when any near-duplicate pair is reported | false |
| `--json` | Machine-readable output (global flag) | false |

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success. The default mode is informational, even when it finds pairs |
| 1 | `--fail-on-found` was given and at least one pair was reported |
| 2 | Operational error (missing index, bad git ref, invalid threshold) |

## Examples

### Find near-duplicates

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

### Only pairs touching changed files

```bash
ctx duplicates --against main
```

### CI gate

```bash
ctx duplicates --against main --fail-on-found
```

### JSON output

```bash
ctx duplicates --json
```

```json
{
  "ctx_version": "0.4.0",
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
        "similarity": 0.952,
        "token_count_a": 64,
        "token_count_b": 64
      }
    ]
  }
}
```

## Caveats

- Idiomatic boilerplate (builders, trait impls, small CRUD handlers) can look structurally similar for good reasons. Raise `--min-tokens` to filter short functions.
- Nested functions share tokens with their enclosing function, so both can appear in results.
- ctx builds fingerprints at index time. Reindex after code changes before running this command.

## See also

- [ctx score](./score.md): counts newly introduced duplication as a gate metric
- [Quality gates](../integrations/quality-gates.md)
- [JSON output](../json-output.md)
