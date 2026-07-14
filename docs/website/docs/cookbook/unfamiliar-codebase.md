---
id: unfamiliar-codebase
title: Understand an unfamiliar codebase before editing it
sidebar_position: 9
---

# Understand an unfamiliar codebase before editing it

The first task in an unfamiliar repository is not to collect every file. It is to build a small,
testable model of how the software starts, where responsibilities live, and which parts deserve
closer inspection before a change.

This recipe uses ctx in layers. Each layer answers a different question, and source code remains
the final check when static relationships are ambiguous or incomplete.

## The orientation brief

Before editing, aim to write down:

- the repository's languages, build units, and executable or library entry points;
- the major subsystems and the responsibility of each;
- one verified execution path relevant to the likely task;
- tests, configuration, persistence, and public-contract locations;
- high-risk or highly connected areas worth treating carefully;
- uncertainties that static analysis could not resolve.

This is an investigation brief, not a claim that you understand every file.

## 1. Refresh the evidence

Build or incrementally update the index before trusting any symbol or graph result:

```bash
ctx index
```

The summary establishes scale and confirms what ctx indexed. In ctx itself, the current index
reported 110 files, 2,161 symbols, and 13,360 edges. Those totals describe the indexed model, not
the entire Git repository: ignored, generated, unsupported, or non-code files may be absent.

If a result contradicts source you can see, reindex before developing a theory about the code.

## 2. Survey shape and structural pressure

Start with the compact human-readable statistics and a deliberately small map:

```bash
ctx query stats
ctx map --budget 1200
```

`query stats` shows scale, symbol-heavy files, and highly connected functions. `map` adds the
repository tree and fits ranked symbols into the requested budget. Use both as a list of leads:

- a symbol-heavy file may be a schema, registry, generated structure, or test suite;
- high fan-in may identify a useful shared primitive rather than unhealthy coupling;
- high fan-out may identify a composition root, parser, or dispatcher;
- the map's first symbol is the highest-ranked indexed symbol, not necessarily the program's entry
  point.

In ctx itself, the 1,200-token map began with a test helper named `find`, followed by shared methods
such as `Metrics::get`. The statistics also highlighted `src/db/schema.rs` and `src/rules.rs`. This
was useful evidence about centrality and code concentration, but it was not an architecture
description.

Prefer the default text output for a first read. `ctx query stats --json` includes the complete
per-file table and is substantially larger; use it when a script needs the full dataset.

## 3. Establish the real entry points

Confirm build metadata and package manifests before relying on naming conventions. For this Rust
repository:

```bash
ctx Cargo.toml src/lib.rs --format markdown --no-tree
ctx query find main
ctx source main --file src/main.rs
```

The manifest proves that `src/main.rs` is the `ctx` binary and `src/lib.rs` is the library root.
`query find main` also finds the performance harness and several Python script entry points, which
is exactly why the manifest and `--file` disambiguation matter.

Apply the same principle in other ecosystems: inspect `package.json`, `pyproject.toml`, `go.mod`,
service manifests, deployment configuration, or framework registration before declaring a symbol
to be an entry point.

Read the entry-point source even when `explain` succeeds. In this repository, `ctx explain main
--file src/main.rs` listed thread-builder calls but did not connect `main` to `run_main`, because
`run_main` is passed as a function value to `spawn` rather than called with ordinary call syntax.
`ctx source` made that transition obvious.

## 4. Trace one distinctive execution path

Move one hop at a time and prefer distinctive symbol names:

```bash
ctx explain run_main --file src/main.rs
ctx query deps run_main --file src/main.rs --depth 2
ctx source run_main --file src/main.rs
ctx explain run --file src/main.rs
```

This verified the CLI path in ctx itself:

```text
main -> run_main -> run(args) -> command-specific run_* function
```

The source adds meaning that the edge list cannot: `main` creates a larger-stack worker thread,
`run_main` initializes the Rayon pool and parses arguments, and `run` dispatches subcommands while
preserving exit-code behavior.

Graph results are hypotheses to check, especially for generic names. Although `--file
src/main.rs` selected the intended Rust `run` symbol, its reported callers included unrelated
Python `subprocess.run` calls and test helpers also named `run`. Static indexing may also miss or
approximate macros, callbacks, function pointers, reflection, dynamic dispatch, generated code,
and runtime registration.

Use the call snippet and source location to accept or reject each important relationship. A result
that merely shares a name is not evidence of a real call path.

## 5. Focus on a subsystem without mistaking focus for isolation

Once an entry point or subsystem path is known, bias the map toward it:

```bash
ctx map --focus src/main.rs --budget 2000
```

On this repository, focus promoted `src/main.rs::run` and the command handlers that it dispatches.
It still retained globally important symbols from other areas. `--focus` is a relevance bias, not
a closed dependency slice and not a path filter.

Follow the useful leads with narrower commands:

```bash
ctx source run --file src/main.rs
ctx query deps run --file src/main.rs --depth 1
ctx query find run_index
ctx explain run_index --file src/commands/index.rs
```

Choose the branch relevant to the task instead of recursively expanding every command. For an
indexing change, continue into `run_index`, the indexer, parsers, database, and indexing tests. For
a CLI-output change, follow argument parsing, dispatch, output formatting, JSON envelopes, exits,
and command tests instead.

## 6. Read the boundaries the graph cannot infer

Complete the brief with repository evidence outside the call graph:

```bash
ctx Cargo.toml src/lib.rs src/cli.rs --count-only
ctx Cargo.toml src/lib.rs src/cli.rs --format markdown --no-tree
ctx query find config
ctx query find test
```

Use `--count-only` before packaging a larger known set. Token budgets omit whole files rather than
truncating them, so an important entry point can disappear when it does not fit. The generated
wrapper and headings also add output overhead beyond the selected files' content-token count.

Then inspect the repository's contributor instructions, compatibility policy, CI configuration,
and user documentation. ctx can locate code relationships, but it cannot infer which JSON fields,
CLI exits, persisted names, or release artifacts the project promises to keep compatible.

## What worked, and what did not

The workflow was exercised against ctx's own indexed repository before this recipe was written.

| Technique | Verified use | Limitation observed |
|---|---|---|
| `ctx index` | Refreshes the model and reports indexed scale | Indexed totals are not repository totals |
| `ctx query stats` | Finds concentration and highly connected symbols | Centrality does not explain intent |
| `ctx map --budget` | Gives a fast tree and ranked leads | Tests and generic helpers may rank first |
| `ctx query find` | Finds candidate entry points across languages | Common names require disambiguation |
| `ctx source` | Confirms the actual transition and local intent | Shows one symbol, not the whole runtime path |
| `ctx explain` and graph queries | Accelerate caller/dependency exploration | Same-name false positives and unresolved dynamic edges remain |
| `ctx map --focus` | Promotes a path or symbol and nearby handlers | Focus biases ranking; it does not isolate a subsystem |

The safe pattern is therefore **survey, identify, trace, verify, and record uncertainty**. No one
command replaces that loop.

## Write the brief before editing

Use a compact handoff format:

```text
Runtime and build units:
Entry points:
Major subsystems:
Verified task-relevant path:
Tests and fixtures:
Configuration and persistence:
Compatibility surfaces:
Structural pressure points:
Unresolved or dynamic relationships:
Files to inspect next, with reasons:
```

For ctx itself, the resulting first-pass model is: a Rust CLI and library share indexing,
database, parsing, context-selection, and governance modules; `src/main.rs` owns CLI dispatch;
`src/commands/` adapts subcommands to library behavior; `src/lib.rs` exposes the public module
surface; SQLite stores the live code index; optional DuckDB-backed analytics and Parquet snapshots
support analytical and longitudinal workflows; tests and governance scripts cover contracts that
the call graph alone cannot describe.

That model is specific enough to choose the next investigation without pretending that every
reported relationship is exact.

## Give the workflow to an agent

```text
Orient yourself in this repository before proposing or making edits. Refresh the ctx index, inspect
the compact statistics and a small map, then confirm build metadata and real entry points from
source. Trace one task-relevant execution path with disambiguated symbols, callers, dependencies,
and source. Treat centrality as a lead, map focus as a ranking bias, and graph edges as hypotheses
that require source verification. Report the architecture, vocabulary, tests, compatibility
surfaces, next files with reasons, and any unresolved dynamic relationships. Do not load or
summarize the whole repository.
```

## Next in Cookbook v2

The next recipe will turn this orientation model into the smallest useful, token-budgeted context
for a concrete task. Later recipes will use the same verified entry points for reuse discovery,
blast-radius analysis, implementation, debugging, and review.
