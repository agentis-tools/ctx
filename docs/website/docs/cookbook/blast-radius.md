---
id: blast-radius
title: Trace the blast radius before editing a symbol
sidebar_position: 12
---

# Trace the blast radius before editing a symbol

Blast radius is more than the number of functions that call a symbol. A change can affect callers,
callees, tests, persisted data, generated files, configuration, documentation, external consumers,
and compatibility promises that are not represented by a call edge.

This recipe was exercised against ctx's public `content_checksum` function using a hypothetical
change to its hashing or checksum-line filtering behavior. That symbol has a small direct call graph
but a much larger persisted and behavioral contract.

## Quickest version

```bash
ctx index
ctx query find <symbol> --json
ctx query callers <symbol> --depth 1 --json
ctx query deps <symbol> --depth 1 --json
ctx source <symbol>
```

Then search the exact persisted or public name and write down tests, formats, configuration, and
external consumers that cannot appear as call edges.

## State the proposed semantic change

Do not ask only “what calls this function?” Record what might change:

```text
Target: src/harness/checksum.rs::content_checksum
Current behavior: lowercase SHA-256 of bytes, excluding ctx:checksum lines in UTF-8 text
Possible change: alter normalization, excluded lines, algorithm, or return encoding
Expected invariants: generated files still classify correctly and user edits remain protected
```

Different edits to the same symbol have different blast radii. A documentation clarification does
not have the same impact as changing which bytes enter the digest.

## 1. Verify and disambiguate the target

```bash
ctx query find content_checksum --json
ctx source content_checksum --file src/harness/checksum.rs
ctx explain content_checksum --file src/harness/checksum.rs
```

The source establishes the real contract:

- UTF-8 text is processed line by line;
- lines recognized as `ctx:checksum` headers are excluded;
- non-UTF-8 data is hashed as raw bytes;
- the digest is returned as lowercase hexadecimal SHA-256.

Use `--file` whenever a name is not globally distinctive. Impact analysis currently accepts the
symbol name but not `--file`, so confirm that the name resolves uniquely before using it.

## 2. Start with direct relationships

Collect a bounded first layer:

```bash
ctx query callers content_checksum --file src/harness/checksum.rs
ctx query deps content_checksum --file src/harness/checksum.rs
ctx query impact content_checksum --depth 1 --json
```

The verified direct caller results identified four important roles:

| Caller | Role |
|---|---|
| `finalize` | Generates a checksum header for new harness content |
| `classify` | Distinguishes unmodified and user-modified generated files |
| `check_hooks` | Reports missing or modified installed hooks |
| `test_checksum_line_is_leader_agnostic` | Locks down header-exclusion behavior |

The dependencies show the implementation mechanism, including `is_checksum_line`, UTF-8 parsing,
and SHA-256 updates. Inspect both directions: callers reveal downstream policy, while dependencies
reveal helpers whose semantics the target inherits.

## 3. Expand one verified caller at a time

Read each direct caller before traversing farther:

```bash
ctx source finalize --file src/harness/checksum.rs
ctx source classify --file src/harness/mod.rs
ctx source check_hooks --file src/harness/doctor.rs

ctx query callers finalize --file src/harness/checksum.rs
ctx query callers classify --file src/harness/mod.rs
ctx query callers check_hooks --file src/harness/doctor.rs
```

This produces three meaningful branches:

- **generation:** render content, calculate its digest, and embed the checksum;
- **ownership:** compare current bytes with a lock or embedded checksum before overwriting;
- **diagnostics:** report installed hooks whose digest no longer matches.

Those branches are the real behavioral blast radius. Their generic function names also explain why
unbounded graph expansion becomes noisy.

:::caution Deep impact can amplify name-resolution errors
For this symbol, `query impact` returned 4 symbols at depth 1, 14 at depth 2, 32 at depth 3, and 96
at depth 5. The deeper results included index hashing, snapshots, scoring, MCP tests, and scripts
that do not depend on `content_checksum`.

The contamination began when generic names such as `finalize`, `new`, and `update` connected
unrelated functions. Use deep impact as a list of hypotheses, not a transitive proof. Stop expanding
a branch when its source does not contain the preceding call.
:::

The ctx 0.3.5 limitation where `query callers` and `query deps` ignored `--depth` has been resolved.
Both commands now traverse resolved symbol IDs breadth-first, report shortest numeric distances,
and stop safely at cycles; unresolved relationships remain non-recursive evidence leaves. Root
filters such as `--file` disambiguate the starting symbol only, so transitive results can cross file
boundaries. Continue verifying every reported step against source: resolution precision, rather
than traversal depth, remains the limiting factor. `query impact` and `query graph` remain useful
alternative projections; their JSON reports zero source-line positions for traversed nodes, so use
the returned file and name to retrieve source.

## 4. Search exact references and persisted names

Static call edges can be incomplete. Compare them with repository references:

```bash
rg -n "\bcontent_checksum\b" . \
  --glob '!target/**' \
  --glob '!.ctx/**' \
  --glob '!.git/**'

rg -n "ctx:checksum|sha256:" \
  src/harness plugins skills docs governance
```

Exact search found calls that the direct graph did not report, including:

- the checksum stored for each `LockEntry` inside `write_plan`;
- additional round-trip, tampering, JSON, YAML, and HTML tests;
- imports and documentation references.

It also exposed persisted and generated compatibility surfaces:

- `.ctx/harness.lock` stores `sha256:<hex>` values;
- generated hooks, skills, and plugin documentation embed checksum headers;
- `classify` uses the lock first and the embedded checksum as a fallback;
- harness documentation describes the generated checksum marker.

A change to the algorithm or excluded bytes can therefore make previously generated files appear
user-modified. That could prevent regeneration or create pressure to use `--force`, defeating the
ownership protection the checksum is meant to provide.

## 5. Compare neighboring implementations

Find similar mechanisms before assuming all hashing behavior should change together:

```bash
ctx similar "SHA-256 file content hash" --keyword --limit 10 --json
ctx source compute_hash --file src/index/mod.rs
ctx query callers compute_hash --file src/index/mod.rs
```

The query found both `content_checksum` and `compute_hash`. Source inspection showed that
`compute_hash` hashes all UTF-8 content for incremental indexing. It deliberately does not exclude
generated checksum lines or implement ownership policy.

This comparison prevents two mistakes:

- changing the general index hash when only generated-file ownership should change;
- replacing the specialized harness checksum with a superficially similar generic helper.

Parallel implementations are part of the design evidence. Decide whether their difference is
intentional before consolidating or updating both.

## 6. Add non-graph compatibility surfaces

Classify the blast radius explicitly:

| Surface | Evidence for this symbol |
|---|---|
| Internal callers | generation, ownership classification, diagnostics |
| Dependencies | SHA-256, UTF-8 handling, checksum-line recognition |
| Tests | round trip, tampering, comment styles, JSON, ownership lifecycle |
| Persisted data | checksums in `.ctx/harness.lock` |
| Generated artifacts | checksum headers in hooks, skills, and plugin files |
| Documentation | harness checksum format and regeneration behavior |
| Public API | `pub fn` inside the public `ctx::harness::checksum` module |
| External consumers | not visible in the repository index |

The last two rows matter even with few internal callers. A public Rust function can have downstream
users that ctx cannot index from this repository. Changing its signature or documented digest
semantics requires compatibility and SemVer review.

Apply the same audit to other symbol types:

- CLI handlers: flags, defaults, exits, help, JSON, shell scripts, docs;
- configuration readers: defaults, validation, examples, environment overrides;
- persistence writers: readers, migrations, historical data, schema versions;
- serialization types: field names, optionality, snapshots, external clients;
- feature-gated code: default, no-default-feature, and platform builds.

## 7. Turn the blast radius into a validation plan

Run the narrowest tests that establish current behavior before editing:

```bash
cargo test --locked --all-features harness::checksum::tests
cargo test --locked --all-features test_write_plan_ownership_lifecycle
```

Both focused suites passed during this investigation: seven checksum tests and the ownership
lifecycle test.

For a real checksum-behavior change, the plan should also include:

- a compatibility fixture containing an artifact and lock written by the previous algorithm;
- harness regeneration and doctor checks for Claude and Codex layouts;
- verification that modified user files are still skipped without force;
- regenerated canonical plugins and checksum lockstep checks;
- documentation and changelog review;
- public API and SemVer assessment;
- default-feature and relevant platform validation.

Do not implement until each material impact has either a test, an inspection step, or an explicit
reason it is unaffected.

## Write the impact brief

```text
Target and proposed semantic change:
Verified direct callers:
Verified dependencies:
Confirmed higher-level branches:
Behavioral tests:
Exact string and configuration references:
Persisted and generated formats:
Public or external compatibility surfaces:
Rejected graph relationships, with reason:
Files likely to change:
Files to validate but not necessarily change:
Required migration or compatibility plan:
Remaining uncertainty:
```

Separating likely edits from validation-only files keeps the working set focused without hiding the
real blast radius.

## What worked, and what did not

| Technique | Verified use | Limitation observed |
|---|---|---|
| Disambiguated source and explain | Established the target's actual semantics | Does not reveal persisted or external contracts |
| Direct callers and dependencies | Identified the main behavioral branches and helpers | Missed closure and several test references |
| `impact --depth 1` | Produced a useful first-layer checklist | JSON line positions were zero |
| Deep impact | Surfaced possible transitive paths quickly | Generic names caused rapid false-positive expansion |
| Exact repository search | Found lock writes, generated markers, tests, and docs | Text matches still need classification by role |
| Similar implementation search | Distinguished specialized ownership hashing from index hashing | Similar mechanics were not interchangeable |
| Focused tests | Confirmed the behavior currently enforced | Passing tests do not cover external consumers or migration |

The reliable loop is **define the semantic change, verify the target, inspect direct edges, expand
branches manually, search contracts, then build the validation plan**.

## Give the workflow to an agent

```text
Trace the blast radius of this symbol before editing it. State the proposed semantic change, then
disambiguate and inspect the target. Start with direct callers, dependencies, and impact depth 1.
Read each important caller before expanding it; treat deeper graph results as hypotheses and reject
branches whose source does not contain the preceding relationship. Search exact symbol names,
persisted strings, configuration, generated artifacts, tests, docs, and contract files. Compare
neighboring implementations to distinguish intentional semantic differences. Include public and
external consumers that the local graph cannot observe. Produce separate lists of likely edit files,
validation-only files, compatibility or migration work, rejected graph results, and uncertainty.
```

## Next in Cookbook v2

The next recipe will use the verified orientation, reuse decision, working set, and impact brief to
implement a feature with an evidence-backed edit-and-reindex loop.
