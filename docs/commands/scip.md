# `ctx scip`

Precise call edges from a [SCIP](https://github.com/scip-code/scip) index. Opt-in: nothing runs unless you import an index.

ctx extracts calls with tree-sitter and binds them by name. That is fast enough to rerun after every edit, but a call like `x.get(k)` can bind to any in-repo `get`. A SCIP index comes from a compiler-backed indexer and records which definition each reference resolves to. `ctx scip import` applies those answers to ctx's call edges:

- **Defined in the repository:** the edge is bound to the ctx function or method that contains the definition.
- **No definition in the index** (standard library, dependency): the edge is unbound as external.
- **No reference at the call site, or a definition outside any ctx function:** the edge is left as it was.

```bash
scip-go && ctx scip import index.scip
ctx scip import index.scip --dry-run --json      # what would change
ctx scip status                                  # call edges by resolver: name / scip / jev
```

## Indexers

Run the indexer in the project root, on the same tree ctx indexed.

| Language | Indexer | Notes |
|---|---|---|
| Go | `scip-go` | |
| Python | `scip-python index . --project-name NAME` | Install the project first (`pip install -e .`). Otherwise `import pkg` resolves to the installed package in site-packages, and those calls are unbound as external. |
| TypeScript | `scip-typescript index` | |
| JavaScript | `scip-typescript index --infer-tsconfig` | Coverage is low for CommonJS/prototype code; `ctx judge edges` handles the rest. |
| Rust | `rust-analyzer scip .` | Runs `cargo check` for build scripts and proc macros first. A cold run takes minutes on large dependency trees. |
| C / C++ | `scip-clang --compdb-path=compile_commands.json` | Needs a compilation database. |

## How it combines with the rest of ctx

- **Provenance:** every edge SCIP answered is recorded with provenance `scip` in the `edge_provenance` table. `ctx scip status` counts call edges by provenance.
- **Jev:** `ctx judge edges` skips SCIP-answered edges, so the model is only asked about what the indexer could not settle.
- **Re-indexing:** answers are cached together with the content hashes of the calling and target files. `ctx index` re-applies them while both files are unchanged. An edited file falls back to name resolution until the next import, so the agent loop never waits for a SCIP run.
- **Schema:** both tables are created on first import. An index without them stays valid, and no rebuild is needed.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Imported (or status printed) |
| 2 | Operational error (no index, unreadable or invalid SCIP file) |
