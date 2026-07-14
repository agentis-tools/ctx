---
id: chronic-hotspots
title: Investigate chronic hotspots without chasing large files
sidebar_position: 5
---

# Investigate chronic hotspots without chasing large files

A hotspot is code that is both structurally complex and frequently changed. That intersection is
more actionable than either dimension alone: a complex but stable parser may be safe, while a
moderately complex coordinator edited every week may impose a continuing cost.

This recipe distinguishes a current hotspot from a chronic one, identifies the symbols responsible,
and tests whether the pressure represents healthy feature growth or accidental responsibility.

## Understand the score before ranking work

`ctx hotspots` computes:

```text
normalized churn × normalized complexity
```

Both dimensions are min-max normalized over the analyzed candidate set. Consequently:

- the score is a ranking within one run, not an absolute health grade;
- scores from different windows, filters, or repositories are not directly comparable;
- a file with the most churn but little complexity can rank below a less frequently changed file;
- a file excluded from the index never appears, even if git reports heavy churn.

Use the raw commit and complexity values when comparing periods. Use the score to choose where to
start looking.

## 1. Choose a window that matches the question

For recent delivery pressure:

```bash
ctx index
ctx hotspots --since "90 days ago" --limit 20
```

For a longer maintenance pattern:

```bash
ctx hotspots --since "12 months ago" --min-churn 4 --limit 20
```

The default window is six months and the default minimum is two commits. A very short window
overweights an active feature branch; a very long window can hide that a former hotspot has become
stable. Run two meaningful windows and compare the raw dimensions, not just rank or score.

File renames reset churn because v1 deliberately uses git history without rename following. Note
known moves before interpreting a sudden drop.

## 2. Move from files to symbols

Start at file level, then ask which functions or methods carry the structural load:

```bash
ctx hotspots --since "90 days ago" --by file --limit 20 --json > file-hotspots.json
ctx hotspots --since "90 days ago" --by symbol --limit 30 --json > symbol-hotspots.json

jq '.data.entries[] | {
  file, commits, complexity, fan_out, score, symbols
}' file-hotspots.json
```

Symbol-level churn is approximated by the containing file's commit count. A function can therefore
rank highly even if neighboring functions—not that function—caused most edits. Confirm with git
history before assigning ownership or proposing a refactor.

## 3. Confirm that the pressure persists

Current rankings answer “where is pressure now?” Snapshot history answers “has it persisted?”:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  SELECT committed_at,
         left(commit_sha, 7) AS commit,
         symbol_count,
         total_complexity,
         max_complexity,
         churn_commits,
         round(total_complexity / nullif(symbol_count, 0), 2)
           AS complexity_per_symbol
  FROM snap.files
  WHERE path = '<hotspot-path>'
  ORDER BY committed_at;"
```

Read total complexity together with `symbol_count`, `max_complexity`, and complexity per symbol:

| Pattern | Interpretation to investigate |
|---|---|
| Total and symbol count rise; density is stable | the module grew without becoming denser |
| Maximum complexity rises repeatedly | one function may be accumulating responsibility |
| Churn stays high after feature delivery | the area may be chronically difficult to change |
| Complexity rises while churn falls | a large but stabilizing implementation |
| Churn rises while complexity stays modest | unstable requirements or ownership, not necessarily design debt |
| One short spike returns to baseline | temporary development activity rather than a chronic hotspot |

Snapshot churn uses the capture's configured churn window. Keep capture behavior and ctx versions
consistent when comparing periods.

## 4. Read the change history

Inspect why the file changes:

```bash
git log --date=short --format='%h %ad %s' -- <hotspot-path>
git log -p -- <hotspot-path>
```

Group changes by cause:

- new product capability;
- bug fixes in the same responsibility;
- repeated compatibility or migration work;
- test-only or generated changes;
- formatting and mechanical rewrites;
- changes caused by an unstable dependency or interface.

Ten coherent feature commits tell a different story from ten fixes to the same brittle branch.
Commit count alone cannot make that distinction.

## 5. Inspect ownership and blast radius

Use the symbols reported in the file-level JSON as entry points:

```bash
ctx map --focus <hotspot-path> --budget 4000
ctx source <symbol>
ctx query callers <symbol>
ctx query deps <symbol>
ctx query impact <symbol>
```

Look for responsibility boundaries rather than small-function targets:

- Does the symbol coordinate one coherent operation or several unrelated ones?
- Are most outgoing calls expected for a facade, parser, or composition root?
- Do callers depend on internal details that should be behind an interface?
- Does the file change because many concepts are colocated, or because one concept is active?
- Would extraction reduce change coupling, or merely spread navigation across more files?

Do not split a transaction, parser state machine, or orchestration flow just to reduce a score. A
refactor is useful when it creates a clearer ownership or dependency boundary.

## 6. Choose an action proportional to the evidence

Classify the hotspot:

- **Healthy active core:** coherent code is changing because the product is growing. Monitor it.
- **Intentional complex boundary:** complexity belongs at a parser, facade, adapter, or transaction
  boundary. Document the rationale and protect its contract with tests.
- **Accidental responsibility:** unrelated reasons to change have accumulated. Extract around an
  ownership boundary and re-run the measurements.
- **Unstable interface:** repeated downstream edits originate in a dependency or API shape. Fix the
  boundary rather than dividing the largest function.
- **Measurement artifact:** renames, generated code, test fixtures, or parser coverage distort the
  rank. Adjust scope or document the limitation.
- **Insufficient evidence:** keep the finding observational.

When acting, make one coherent change, then compare the branch against its base:

```bash
ctx score --against <base>
ctx hotspots --against <base> --since "90 days ago"
```

A successful refactor does not have to make every number fall. It should make the intended
responsibility, dependency direction, or future change path clearer.

## What this found in ctx itself

In the 90-day current view, `src/db/schema.rs` ranked first with 11 commits, complexity 2,084,
fan-out 900, and a relative hotspot score of 0.60. Its leading symbols included
`find_symbols_filtered`, `symbol_from_row`, and `metrics_fixture`. At symbol level, however,
`src/main.rs::run` ranked first because its containing file had 17 commits and the function's
complexity was 109.

The historical view adds necessary context. The latest snapshot recorded 109 symbols, total
complexity 2,091, maximum symbol complexity 84, 11 churn commits, and complexity per symbol 19.18
for `src/db/schema.rs`. Earlier captures reached 20.34 complexity per symbol. The file clearly grew
and remained active, but its structural density did not rise monotonically with its size.

That makes `src/db/schema.rs` a legitimate investigation target, not an automatic extraction task.
The next question is whether database schema, migrations, query mapping, vector support, and test
fixtures represent separable ownership boundaries—or whether splitting them would only fragment a
coherent persistence layer.

## Give the workflow to an agent

```text
Find chronic hotspots using both current ctx rankings and snapshot history. Compare meaningful
churn windows, use raw churn and complexity rather than comparing normalized scores across runs,
and inspect the symbols and commits responsible. Normalize file totals by symbol count, account for
renames and symbol-level churn approximation, and classify healthy active cores, intentional
boundaries, accidental responsibility, unstable interfaces, measurement artifacts, and
insufficient evidence. Recommend a change only when it creates a clearer ownership or dependency
boundary.
```

## Next steps

- Use the [architecture drift recipe](architecture-drift.md) when the hotspot crosses an intended
  layer boundary.
- Use [ctx hotspots](../commands/hotspots.md) for the complete scoring and option reference.
- Continue with the intentional-complexity recipe before deciding that a high-scoring function
  should be decomposed.
