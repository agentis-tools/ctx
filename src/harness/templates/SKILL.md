---
name: ctx
description: Codebase intelligence via the indexed ctx CLI. Use when exploring an unfamiliar codebase, locating symbols or their callers, checking for existing implementations before writing new code, enforcing architecture rules, or scoring the quality impact of changes.
---

# ctx — code intelligence

ctx maintains a symbol-level index of this codebase (`.ctx/codebase.sqlite`).
Prefer its queries over grep: they resolve symbols, call graphs, and
structure instead of matching text.

All quality commands share one exit-code convention: `0` clean, `1` findings,
`2` operational error. Treat exit 1 as content, not failure.

## Keep the index fresh

```bash
ctx index            # incremental; run after pulling or large edits
ctx index --force    # full rebuild (schema changes, corruption)
```

## Orient in the codebase

```bash
ctx map --budget 2000        # token-budgeted architectural overview
ctx query find <name>        # locate symbols by name
ctx query callers <fn>       # who calls this?
ctx query deps <symbol>      # what does this depend on?
ctx query impact <symbol>    # blast radius of changing it
ctx source <symbol>          # exact source of a symbol
ctx explain <symbol>         # signature, doc, relationships
ctx search "<query>"         # hybrid name/text search
```

## Avoid writing duplicate code

```bash
ctx similar <symbol>         # near-duplicates of one function
ctx duplicates --against main  # new near-duplicate pairs in your change
```

Run `ctx similar` before implementing a helper that sounds generic — an
implementation may already exist.

## Quality gates

```bash
ctx check --against HEAD --json   # architecture rules (.ctx/rules.toml)
ctx score --against main --fail-on "check_violations>0,new_duplication>0"
ctx hotspots                      # churn x complexity refactoring targets
```

## Recommended agent workflow

1. **Start**: `ctx map --budget 2000` to orient (the SessionStart hook does
   this automatically when installed).
2. **Before writing code**: `ctx query find` / `ctx similar` to find prior
   art; `ctx query impact` before refactoring shared symbols.
3. **After edits**: `ctx index` then `ctx check --against HEAD --json`;
   fix any violations immediately.
4. **Before finishing**: `ctx score --against main` and address regressions
   (rising complexity, new duplication, rule violations).

Never edit `.ctx/rules.toml` (project policy) or the generated hook scripts;
regenerate the latter with `ctx harness init`.
