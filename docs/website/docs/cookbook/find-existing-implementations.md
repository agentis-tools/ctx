---
id: find-existing-implementations
title: Find existing implementations before writing new code
sidebar_position: 11
---

# Find existing implementations before writing new code

A repository often contains the behavior you need under a different name, split across utilities,
or embedded inside a neighboring workflow. Finding it is not merely a search problem: you must
decide whether the existing semantics, ownership boundary, tests, and consumers make it safe to
reuse.

This recipe was exercised against ctx itself using the hypothetical task **“verify generated plugin
files by comparing embedded checksums.”** The investigation found an existing complete workflow,
not just a similarly named helper.

## Describe behavior, not the implementation you expect

Start with inputs, outcome, and important policy:

```text
Given a generated file path and an expected or embedded checksum,
hash the current content without counting the checksum header,
then distinguish unmodified, user-modified, foreign, and missing files.
Never overwrite user-modified or foreign files silently.
```

This description leaves room to discover a better repository-native abstraction. A query such as
“add `verify_plugin_checksum()`” prematurely assumes both a name and a new function boundary.

## 1. Search semantically for the behavior

When matching embeddings are available:

```bash
ctx similar \
  "compare a recorded checksum with the current file content to detect edits" \
  --provider ollama \
  --limit 10 \
  --json
```

The project `.ctx/config.toml` can provide the provider, so the flag is optional when the configured
model matches the index. The worked query ranked these existing functions highly:

- `recorded_checksum` — extracts an embedded `ctx:checksum` value;
- `content_checksum` — hashes content while excluding checksum-header lines;
- `Database::needs_update` — a related hash-comparison idea, but for index freshness rather than
  generated-file ownership.

The third result is conceptually similar but semantically wrong for this task. Similarity generates
candidates; it does not decide reuse.

A signature-like query can be even more effective when the intended boundary is understood:

```bash
ctx similar \
  "fn verify_generated_file(path: &Path) -> Ownership" \
  --provider ollama \
  --limit 10 \
  --json
```

In ctx itself, that query ranked the existing private `classify` function first even though its
actual name contains neither “verify” nor “generated file.”

## 2. Use keyword mode with repository vocabulary

When embeddings are unavailable, or when source terminology is already known:

```bash
ctx similar "embedded checksum" --keyword --limit 10 --json
ctx similar "content checksum" --keyword --limit 10 --json
```

These queries found `recorded_checksum`, `content_checksum`, `finalize`, and their tests without
contacting an embedding provider. The shorter terms worked better than the original sentence
“verify generated plugin files by comparing embedded checksums,” whose generic words promoted
plugin planners above the checksum implementation.

Vary the query deliberately:

- user language: what outcome is needed;
- repository vocabulary: persisted names, error text, or domain terms;
- signature-like language: likely input and result types;
- one mechanism at a time: checksum extraction, hashing, classification, or overwrite policy.

Do not treat the result score as a probability or confidence threshold. In testing, a nonsense
keyword query still returned an unrelated function with a score above 0.8. Scores rank results
within a retrieval mode; source inspection determines whether any result is actually relevant.

## 3. Inspect source before judging reuse

Read the candidates:

```bash
ctx source recorded_checksum --file src/harness/checksum.rs
ctx source content_checksum --file src/harness/checksum.rs
```

The source verified details that names alone do not reveal:

- `recorded_checksum` accepts different comment leaders and an optional `sha256:` prefix;
- `content_checksum` removes every checksum-header line before hashing UTF-8 text;
- non-UTF-8 input is hashed as raw bytes;
- both helpers return mechanisms, not the ownership decision required by the task.

Compare each candidate against a small semantic checklist:

| Question | Why it matters |
|---|---|
| Does it accept the right inputs? | Adapters may be cheaper than a duplicate implementation |
| Does it preserve the required failure behavior? | Silent fallbacks can violate policy |
| Does it have side effects? | A validator that also writes is not interchangeable |
| Is it public, private, or feature-gated? | Reuse may cross an intentional boundary |
| Do callers depend on its exact result? | Changing it may have a wider blast radius |
| Which edge cases do its tests establish? | Tests reveal the supported semantics |

## 4. Trace callers to find the composed workflow

Reusable behavior is often assembled one level above the matching primitives:

```bash
ctx query callers recorded_checksum \
  --file src/harness/checksum.rs \
  --depth 2
ctx query callers content_checksum \
  --file src/harness/checksum.rs \
  --depth 2
```

Both caller lists converged on `src/harness/mod.rs::classify`:

```bash
ctx source classify --file src/harness/mod.rs
ctx query callers classify --file src/harness/mod.rs --depth 2
ctx query find Ownership --json
```

`classify` already performs the complete task:

1. report a missing path as `Missing`;
2. read and hash the current file;
3. prefer the checksum from the harness lock when present;
4. fall back to the checksum embedded in UTF-8 content;
5. distinguish `OwnedUnmodified`, `OwnedModified`, and `Foreign`.

Its caller, `write_plan`, turns that classification into safe create, regenerate, skip, or forced
overwrite actions. The apparently new feature was therefore an existing ownership workflow hidden
behind a general name.

This is why reuse discovery should trace both directions: inspect what a candidate calls to
understand its mechanism, and inspect its callers to find higher-level policy already built around
it.

## 5. Verify the tests that define the behavior

Locate and read the closest behavioral test:

```bash
ctx query find Ownership --json
ctx source test_write_plan_ownership_lifecycle --file src/harness/mod.rs
```

The test proves that:

- a first run creates generated files;
- a second run regenerates unmodified owned files;
- a user-modified hook is preserved without `--force`;
- `--force` overwrites that modified generated file;
- a policy-owned `rules.toml` remains untouched even with `--force`.

These are stronger reuse constraints than the checksum algorithm itself. A new helper that merely
returns `true` or `false` would discard the repository's foreign-file and overwrite policy.

## 6. Choose the reuse boundary

Classify the decision explicitly:

- **Reuse unchanged:** call the existing function when its inputs, result, ownership, and policy
  already match.
- **Extend in place:** add a case to the existing abstraction when the new behavior has the same
  owner and all current callers should see it.
- **Extract a shared primitive:** preserve the existing policy wrapper while moving a genuinely
  common mechanism behind a narrower interface.
- **Adapt at the caller:** translate inputs or results locally when the mismatch is superficial.
- **Implement separately:** do this only when semantics or ownership are intentionally different;
  record why the nearby implementation is not reusable.

For the worked task:

- code inside the harness planning flow should reuse `classify`;
- another harness component can reuse the public `content_checksum` and `recorded_checksum`
  primitives, or justify extracting a shared classifier;
- an unrelated subsystem should not depend casually on the private `Ownership` policy;
- no caller should recreate the checksum-header exclusion logic.

Public Rust modules and functions are compatibility surfaces in this repository. Making `classify`
public merely to avoid a local adapter would require contract and SemVer review.

## Current command limitations

`ctx similar` in ctx 0.3.5 is scoped to functions and methods. Structs, enums, constants,
configuration keys, and prose must be found through `query find`, `search`, or repository text
search.

The generic CLI help also displays positional file patterns for `similar`, but the current command
does not apply them. An empirical query followed by `src/harness/` still returned functions from
`src/rules.rs`, `src/update.rs`, and scripts. Filter the JSON results when necessary:

```bash
ctx similar \
  "fn verify_generated_file(path: &Path) -> Ownership" \
  --provider ollama \
  --json |
  jq '.data.results[] | select(.symbol.file | startswith("src/harness/"))'
```

Do not assume a pattern narrowed the search unless the returned paths prove it.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| Behavior-specific semantic query | Found related mechanisms without knowing their names | The first result can be a test or conceptually adjacent function |
| Signature-like semantic query | Ranked the complete `classify` workflow first | Requires a reasonable guess about the abstraction boundary |
| Short keyword queries | Found checksum primitives without embeddings | Long queries were diluted by generic vocabulary |
| `fan_in` in similar results | Identified functions with established consumers | Popularity does not prove semantic suitability |
| Candidate source inspection | Revealed encoding, header, and fallback semantics | One helper did not show the higher-level policy |
| Caller tracing | Found the composed ownership workflow | Static caller results still require source verification |
| Behavioral test inspection | Established overwrite and foreign-file policy | Tests can be missed if search stops at the first utility |

The reliable loop is **describe behavior, vary retrieval mode, inspect candidates, trace their
callers, verify tests, then choose the ownership boundary**.

## Give the workflow to an agent

```text
Before writing a new function or adding a dependency, search for existing behavior. Describe the
desired inputs, output, side effects, failure behavior, and policy without assuming a name. Use ctx
similar with matching embeddings, then repeat with short repository vocabulary and --keyword.
Treat scores and fan-in as ranking evidence, not confidence or proof of reuse. Inspect candidate
source, dependencies, callers, and behavioral tests; callers may reveal a complete composed
workflow above the matching primitives. Decide explicitly whether to reuse unchanged, extend,
extract, adapt, or implement separately. If implementing separately, explain why the neighboring
behavior has different semantics or ownership.
```

## Next in Cookbook v2

The next recipe will trace the blast radius of a verified symbol before editing it, including
callers, dependencies, tests, configuration, persisted names, and public contracts.
