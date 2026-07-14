---
id: smallest-useful-context
title: Build the smallest useful context for a task
sidebar_position: 10
---

# Build the smallest useful context for a task

Small context is valuable only when it still contains the evidence needed to make a correct
change. A short bundle that omits the writer, a contract consumer, or the relevant test is cheaper
to read and more expensive to trust.

This recipe uses `ctx smart` to discover candidates, then verifies the working set with exact
search, graph relationships, source inspection, and explicit token counting. It was exercised on
the task **“add a new field to historical snapshot metadata”** in ctx itself.

:::note Worked-example provenance
The candidate counts and token estimates were measured against ctx 0.3.5 on 2026-07-14. They are
illustrative and may change with repository content, embeddings, tokenizer, and ctx version.
:::

## Quickest version

```bash
ctx index
ctx smart "<task, behavior, and contract>" --explain --dry-run
ctx query callers <writer-or-public-symbol> --depth 1
ctx query deps <writer-or-public-symbol> --depth 1
ctx <verified-files...> --count-only
```

Use ranking to discover candidates, then explicitly add writers, readers, tests, schemas, and
contracts. Count the verified set before packaging it; a token budget is not a completeness proof.

## Define useful before defining small

Write a task-specific completeness test. For a persisted metadata field, useful context should
normally include:

- the code that writes the field;
- the code that reads or exposes it;
- the schema or compatibility version;
- a test that writes the data and a test that consumes it;
- user or machine-readable schema documentation;
- the dispatch or configuration boundary only when the change passes through it.

Other tasks have different requirements. A CLI flag needs parsing, dispatch, exits, JSON, tests,
and docs. A provider change needs the abstraction, resolver, factory, configuration, and integration
tests. Define the required evidence in engineering terms before asking a ranking algorithm to fit
files into a budget.

## 1. Generate an explained candidate set

Refresh the index, then preview `smart` selection:

```bash
ctx index
ctx smart "add a new field to historical snapshot metadata" \
  --dry-run \
  --explain \
  --provider ollama
```

Use the provider and model that match the stored embeddings. The project-level
`.ctx/config.toml` can supply the provider, so the explicit flag is optional when that configuration
is correct.

The preview answers **why a file became a candidate**: a semantic match, a caller, or a dependency.
It does not prove that every candidate belongs in the final bundle, and it does not prove that an
unselected contract surface is irrelevant.

:::caution Current dry-run behavior
In ctx 0.3.5, `smart --dry-run` reports the entire ranked candidate set rather than applying
`--max-tokens`. For the worked task, `--max-tokens 9000 --dry-run` still reported 18 files totaling
81,869 content tokens. Dry-run also emits its human preview rather than a JSON envelope. Use it to
inspect ranking and reasons, then run the non-dry command to verify the actual budgeted selection.
:::

## 2. Test the real budget boundary

Run the same task at the budget the receiving model can afford:

```bash
ctx smart "add a new field to historical snapshot metadata" \
  --max-tokens 10000 \
  --explain \
  --format plain \
  --no-tree \
  --provider ollama
```

The observed behavior changed materially with the budget:

| Content budget | Selected content | Important omission |
|---:|---|---|
| 4,000 | command wrapper, CLI test, stub test, command registry | core `src/snapshot.rs` implementation |
| 9,000 | command wrapper, core implementation, stub, registry, exits | main snapshot integration test |
| 10,000 | command wrapper, core implementation, integration test, no-DuckDB test | SQL consumer test and schema docs |

ctx packs whole files; it does not truncate a large file to make it fit. At 4,000 tokens the
6,575-token core implementation could not fit after a smaller, higher-ranked wrapper was selected.
At 10,000 tokens the four selected files contained 9,768 tokens.

The final formatted output was estimated at roughly 10,900 tokens because project framing,
headings, and explanations add overhead. Treat `--max-tokens` as a selected-content budget, not a
guarantee about the byte-for-byte size of the complete rendered response.

## 3. Audit the candidate set against the task

Search the persisted or public name directly when one is known:

```bash
ctx search "meta.parquet" --limit 20 --json
ctx query find Snapshot --json
```

The exact `meta.parquet` search found:

- `SNAPSHOT_SCHEMA_VERSION` and `write_parquet_files` in `src/snapshot.rs`;
- the Parquet creation test in `tests/snapshot_cli.rs`;
- `snapshots_meta_is_accessible` in `tests/sql.rs`.

The last result was not present in the 10,000-token smart bundle, yet it is the test that queries
the persisted `snap.meta` table. Exact terminology therefore exposed a contract consumer that the
automatic file selection omitted.

Search documentation and non-symbol contracts as text as well:

```bash
rg -n "meta\.parquet|snap\.meta|snapshot_schema_version|capture_mode" \
  docs governance scripts tests src
```

ctx's indexed search is symbol-oriented. Repository text search remains appropriate for prose,
snapshots, string contracts, generated-contract inputs, and configuration keys.

## 4. Verify relationships around the implementation

Trace the distinctive writer symbol:

```bash
ctx query callers write_parquet_files --file src/snapshot.rs --depth 2
ctx query deps write_parquet_files --file src/snapshot.rs --depth 2
ctx query impact write_parquet_files --depth 3
ctx source write_parquet_files --file src/snapshot.rs
```

The verified graph showed:

- direct caller: `capture`;
- internal helpers: timestamp conversion and `copy_to_parquet`;
- downstream paths: `capture_commit`, `backfill`, `run_snapshot`, and CLI dispatch.

The source then showed the precise contract: one SQL projection writes `meta.parquet`, including
`captured_at`, `ctx_version`, `snapshot_schema_version`, and `capture_mode`.

Use graph expansion to answer “what behavior reaches this code?” It will not discover every
documentation reference or persisted string consumer, and generic call names may produce false
relationships. Verify important edges from source.

## 5. Use symbol snippets for the smallest investigation context

When a whole consumer file is large, retrieve the relevant symbol instead of immediately adding
the entire file:

```bash
ctx source write_parquet_files --file src/snapshot.rs
ctx source snapshots_meta_is_accessible --file tests/sql.rs
ctx source capture_creates_partition_with_all_parquet_files \
  --file tests/snapshot_cli.rs
```

These snippets are enough to decide that the change crosses a Parquet writer, an integration test,
and a SQL-visible schema. They are not necessarily enough to implement safely: neighboring test
helpers, imports, and local conventions may require the complete files when editing begins.

Separate two artifacts:

- **Investigation context:** symbol source and relationship evidence used to choose the working set.
- **Implementation context:** complete files required to edit, compile, and preserve local
  conventions.

This distinction avoids loading a 4,847-token test file merely to discover one 18-line consumer,
while still allowing the complete file into the implementation phase.

## 6. Count the explicit implementation set

After the audit, name the files rather than asking ranking to choose again:

```bash
ctx \
  src/snapshot.rs \
  src/commands/snapshot.rs \
  tests/snapshot_cli.rs \
  tests/sql.rs \
  docs/website/docs/commands/snapshot.md \
  src/commands/sql_schema.md \
  --count-only
```

For the worked example, these six files totaled 19,014 content tokens. That is larger than the
automatic 10,000-token bundle because it includes the SQL contract test and both user-facing and
embedded schema documentation.

If the available window is smaller, reduce context deliberately:

1. Keep the writer and its closest integration test as complete files.
2. Replace a large secondary consumer with `ctx source <symbol>` during investigation.
3. Include only the canonical copy of duplicated documentation while reasoning, but update and
   verify every required copy during implementation.
4. Omit registries or exit helpers only after source confirms the change does not affect them.
5. Record every omitted contract surface so the implementation or review phase revisits it.

Do not lower the budget repeatedly until ctx happens to emit something short. That optimizes for
size without preserving the task's completeness test.

## 7. Package the verified set

Once the files fit the intended window:

```bash
ctx \
  src/snapshot.rs \
  src/commands/snapshot.rs \
  tests/snapshot_cli.rs \
  tests/sql.rs \
  docs/website/docs/commands/snapshot.md \
  src/commands/sql_schema.md \
  --format markdown \
  --no-tree
```

Keep `--explain` on smart-generated bundles when the recipient must understand why each file was
selected. For an explicit, audited file list, provide the reasons in the task handoff instead of
spending context on a global tree or unrelated ranked candidates.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| `smart --dry-run --explain` | Reveals the ranked candidate set and selection reasons | Current dry-run ignores the token limit and is human-formatted |
| Budgeted `smart` output | Quickly builds a useful whole-file implementation bundle | A smaller wrapper can displace a large core file; contracts may be omitted |
| Exact `ctx search` | Finds writer symbols and symbolized consumers from persisted terminology | Does not cover every prose or string-only reference |
| Callers, dependencies, and impact | Verifies the execution path around a distinctive symbol | Static edges do not replace contract or text search |
| `ctx source` | Creates very small, high-signal investigation context | A snippet omits neighboring conventions needed for editing |
| Explicit files plus `--count-only` | Measures the audited implementation set before rendering | Formatted output adds overhead beyond content tokens |

The reliable loop is **rank candidates, audit requirements, inspect symbols, name the files, count,
then package**.

## Give the workflow to an agent

```text
Build the smallest useful context for this task. First define which writers, readers, tests,
configuration, persisted schemas, public contracts, and documentation would make the context
complete. Use ctx smart --dry-run --explain only for candidate discovery, then verify the actual
budgeted output. Audit omissions with exact search, semantic search when configured, callers,
dependencies, impact, source inspection, and repository text search. Use symbol snippets for the
investigation, but include complete files needed for implementation. Count the final explicit file
set before rendering it. Report why every included file belongs and which known surfaces remain
outside the bundle.
```

## Next in Cookbook v2

The next recipe will use this evidence-backed working set to find existing implementations before
introducing new code or a new dependency.
