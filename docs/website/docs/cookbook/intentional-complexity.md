---
id: intentional-complexity
title: Review and document intentional complexity
sidebar_position: 6
---

# Review and document intentional complexity

High complexity is sometimes the correct shape of the code. Parsers must cover a grammar,
dispatchers must route commands, transaction boundaries must coordinate work, and widely reused
helpers naturally acquire many incoming edges.

This recipe determines what a high ctx score actually represents, tests whether the responsibility
is coherent, and records the reasoning so the same code is not repeatedly “simplified” without
context.

## Quickest version

```bash
ctx index
ctx query stats --json
ctx source <high-scoring-symbol>
ctx query callers <high-scoring-symbol> --depth 1
ctx query deps <high-scoring-symbol> --depth 1
```

Separate fan-in from fan-out, read the responsibility in source, and check change pressure. Keep
the complexity when proposed boundaries would only move coordination or duplicate invariants.

## Begin with the metric ctx actually computes

ctx complexity is a graph heuristic:

```text
complexity = 2 × fan-out + fan-in
```

It is not cyclomatic complexity and does not count branches directly. A score can be high because:

- the symbol calls many other symbols;
- many symbols call it;
- both are true.

A tiny, popular lookup can therefore outrank a long recursive function. Always inspect `fan_in`,
`fan_out`, source length, kind, and ownership before interpreting the total.

## 1. Build a review shortlist

Query the public schema rather than choosing an arbitrary universal threshold:

```bash
ctx index
ctx sql "
  SELECT coalesce(qualified_name, name) AS symbol,
         file,
         kind,
         complexity,
         fan_in,
         fan_out,
         line_start,
         line_end
  FROM v1.symbols
  ORDER BY complexity DESC
  LIMIT 30;"
```

Route candidates by shape:

| Shape | First question |
|---|---|
| High fan-in, low fan-out | Is this a stable shared primitive or an accidental dependency magnet? |
| Low fan-in, high fan-out | Is this an intended entry point, dispatcher, or coordinator? |
| High on both dimensions | Is it a public boundary carrying too many unrelated responsibilities? |
| Long source range, moderate graph score | Does local branching or state still make it hard to reason about? |
| Test or fixture symbol | Is repetition and explicit setup improving test independence? |

The shortlist is for review, not a queue of mandatory refactors.

## 2. Inspect the symbol in context

Disambiguate common names with `--file`:

```bash
ctx explain <symbol> --file <path> --json
ctx source <symbol> --file <path>
ctx query callers <symbol>
ctx query deps <symbol>
ctx map --focus <path> --budget 4000
```

Check whether the graph matches the source. Static extraction cannot see every dynamic dispatch,
reflection path, generated call, or macro expansion, and a common symbol name can require explicit
file or kind filtering.

## 3. Apply the coherence test

Intentional complexity should have a coherent reason. Ask:

1. Can the symbol's responsibility be described in one precise sentence?
2. Do its outgoing dependencies serve that responsibility?
3. Do its callers depend on a stable contract rather than implementation details?
4. Must state, ordering, error handling, or transactionality remain visible in one place?
5. Would extraction create a meaningful ownership boundary, or only more indirection?
6. Do tests protect the behavior that makes the complexity necessary?

Strong reasons to keep complexity together include:

- exhaustive traversal of an external AST or protocol;
- a composition root that wires otherwise independent components;
- a command dispatcher whose branches delegate immediately;
- atomic transaction or cleanup sequencing;
- a compatibility boundary that must handle several versions or platforms;
- a compact shared primitive with high fan-in.

Weak reasons include “it has always lived here,” “splitting it is inconvenient,” or “the score is
only a heuristic.” A heuristic is not a verdict, but it can still reveal incoherent responsibility.

## 4. Look for change pressure

Intentional complexity can become accidental over time. Combine the source review with churn:

```bash
ctx hotspots --since "6 months ago" --by symbol --limit 30
git log --date=short --format='%h %ad %s' -- <path>
```

Use snapshots to see whether one symbol keeps accumulating graph responsibility:

```bash
ctx sql --snapshots=../ctx-snapshots/snapshots "
  SELECT committed_at,
         left(commit_sha, 7) AS commit,
         complexity,
         fan_in,
         fan_out,
         line_end - line_start + 1 AS lines
  FROM snap.symbols
  WHERE file = '<path>'
    AND name = '<symbol>'
  ORDER BY committed_at;"
```

Symbol identities can move when functions are renamed or reorganized, so confirm discontinuities
with git history. A stable score and stable responsibility support intentionality; repeated growth
across unrelated changes deserves renewed review.

## 5. Decide whether decomposition improves the design

Before editing, state the proposed boundary and expected benefit:

| Proposed change | Useful when | Harmful when |
|---|---|---|
| Extract pure transformation | it has an independent contract and tests | it hides a small step needed to understand sequencing |
| Split dispatcher branches | branches contain domain behavior | branches already delegate and the split adds navigation |
| Introduce interface | callers need a stable boundary or alternate implementation | it exists only to reduce direct edges |
| Separate state machine phases | phases have explicit invariants | state must be passed through a fragmented call chain |
| Move test fixtures | setup is broadly reusable | tests become coupled through an over-general helper |

After a justified change, compare against the base:

```bash
ctx score --against <base>
ctx query impact <moved-or-extracted-symbol>
```

Do not declare success solely because the complexity number fell. Verify readability, invariants,
dependency direction, tests, and the likely path of the next change.

## 6. Record the decision

When complexity is intentional, leave durable context close to the engineering decision:

- a focused doc comment explaining the responsibility or invariant;
- tests named after the behavior that must remain atomic or exhaustive;
- an architecture decision for a cross-cutting boundary;
- a reviewed `.ctx/rules.toml` exclusion only when a generic limit rule would otherwise misclassify
  a known case;
- a follow-up trigger such as “revisit if another command family/state/adapter is added.”

Avoid comments that merely repeat “this function is intentionally complex.” Explain why its parts
belong together and what event would invalidate that reasoning.

## What worked, and what did not in ctx itself

The current highest ctx complexity score belongs to `Metrics::get` in `src/score.rs`: complexity
226, fan-in 212, and fan-out 7. `ctx explain` shows that it is a public metric-name lookup spanning
only lines 80–91. Its score reflects reuse and importance, not a large implementation. Splitting it
would not address the reason it ranks highly.

`extract_calls_from_expr` in the Solidity parser has a different shape: complexity 131, fan-in 25,
fan-out 53, and roughly 199 source lines. Its large recursive match mirrors the external expression
grammar and keeps traversal behavior exhaustive. Extraction may still help if independent
expression families have stable invariants, but line count or score alone is not enough.

`run` in `src/main.rs` scores 109 with fan-out 54 and acts as the CLI composition and dispatch
boundary. Most branches delegate to command modules, which is evidence of a coherent dispatcher.
Its 90-day symbol hotspot rank is high because `src/main.rs` changed frequently. The right review is
whether command-specific behavior is leaking into the dispatcher—not whether every match arm must
move to reduce the score.

These three symbols occupy the same leaderboard for three different reasons: popularity, exhaustive
grammar traversal, and central dispatch. Treating them with one threshold would erase the most
useful information.

## Give the workflow to an agent

```text
Review the highest-complexity ctx symbols without assuming they should be decomposed. Separate
fan-in from fan-out, inspect source and call relationships, correlate with churn and historical
growth, and apply a coherence test. Classify each symbol as a shared primitive, intentional
boundary, coherent exhaustive implementation, accidental responsibility, measurement limitation,
or insufficient evidence. For intentional cases, explain why the parts belong together and define
a concrete trigger for future re-evaluation. Judge proposed refactors by ownership, invariants,
dependency direction, and future change cost—not by score reduction alone.
```

## Next steps

- Use the [chronic-hotspots recipe](chronic-hotspots) when high complexity also changes
  frequently.
- Use [ctx explain and source](../code-intelligence#symbol-information) to verify the symbol's
  actual role.
- Continue with the duplication-trajectory recipe to apply the same evidence-first reasoning to
  similar implementations.
