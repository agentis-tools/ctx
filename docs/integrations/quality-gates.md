# Quality Gates

Wire the ctx quality suite into CI pipelines and AI coding agents.

## Philosophy: Exit Codes Are the API

Every ctx quality command follows the same three-way exit-code convention (like `grep` or most linters):

| Code | Meaning |
|------|---------|
| 0 | Success, nothing to report |
| 1 | Command ran successfully but produced findings |
| 2 | Operational error (bad arguments, missing index, git failure, ...) |

That convention **is** the integration API. A shell `&&`, a CI step, or an agent hook needs no JSON parsing to enforce a gate - the process exit code carries the verdict, and `--json` is there when a tool wants the details. Crucially, code 2 never masquerades as a finding: a broken gate fails loudly instead of "passing".

## The Suite

The quality commands are designed to be composed:

| Command | Question it answers | Gate |
|---------|--------------------|------|
| [`ctx check`](../commands/check.md) | Does this change violate our architecture rules? | exits 1 on violations |
| [`ctx hotspots`](../commands/hotspots.md) | Where does refactoring pay off most (churn x complexity)? | informational |
| [`ctx duplicates`](../commands/duplicates.md) | Which functions are structural near-copies? | `--fail-on-found` |
| [`ctx similar`](../commands/similar.md) | Does the function I'm about to write already exist? | informational (pre-emptive) |
| [`ctx score`](../commands/score.md) | Did this change make the code better or worse? | `--fail-on "metric OP value,..."` |
| [`ctx map`](../commands/map.md) | What does this codebase look like, in N tokens? | informational (orientation) |

`ctx score` is the composite gate: it folds check violations, new duplication, complexity/fan-out deltas, and symbol churn into one scorecard with `--fail-on` conditions.

## CI Example

```yaml
# GitHub Actions
- name: Index codebase
  run: ctx index

- name: Architecture rules (new violations only)
  run: ctx check --against origin/main

- name: Quality score gate
  run: ctx score --against origin/main --fail-on "check_violations>0,new_duplication>0,complexity_delta>=25"
```

## Claude Code Integration (reference configuration)

This is the reference wiring for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) hooks in a project's `.claude/settings.json`. It gives the agent a codebase map at session start, checks architecture rules after every edit, and blocks it from finishing with rule violations or fresh copy-paste:

```json
{
  "permissions": { "allow": ["Bash(ctx *)"] },
  "hooks": {
    "SessionStart": [ { "hooks": [{ "type": "command", "command": "ctx map --budget 2000" }] } ],
    "PostToolUse": [ { "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": "ctx index && ctx check --against HEAD --json" }] } ],
    "Stop": [ { "hooks": [{ "type": "command", "command": "ctx score --against main --fail-on \"check_violations>0,new_duplication>0\"" }] } ]
  }
}
```

How the pieces work together:

- **SessionStart** - `ctx map --budget 2000` injects a token-budgeted structural overview, so the agent starts oriented instead of exploring blind.
- **PostToolUse** on `Edit|Write` - `ctx index && ctx check --against HEAD --json` reindexes incrementally (fast: only changed files are re-parsed) and reports any *new* architecture violation right at the edit that introduced it, as JSON the agent can act on.
- **Stop** - `ctx score --against main --fail-on "check_violations>0,new_duplication>0"` is the final gate: exit 1 tells the agent its work is not done, with the failed conditions on stderr.

### Recommended CLAUDE.md guidance

Pair the hooks with these instructions in the project's `CLAUDE.md`, so the agent avoids findings instead of just hitting the gates:

```markdown
## Code quality

- Before writing a new function, run `ctx similar "<what it should do>"` -
  extend or reuse an existing implementation instead of adding a near-copy.
- Before modifying an exported symbol, run `ctx query impact <symbol>` to
  see what depends on it.
- Files flagged by `ctx hotspots` are already complex and frequently
  changed: prefer extracting a new module over growing them further.
- The Stop hook runs `ctx score --against main`; keep `check_violations`
  and `new_duplication` at 0.
```

## Pre-commit Hook

```bash
#!/bin/sh
# .git/hooks/pre-commit
ctx index && ctx score --fail-on "check_violations>0,new_duplication>0"
```

With the default `--against HEAD`, this scores exactly what is about to be committed.

## See Also

- [ctx score](../commands/score.md), [ctx check](../commands/check.md), [ctx duplicates](../commands/duplicates.md)
- [JSON Output](../json-output.md) - payload contract for `--json` consumers
- [CI/CD Integration](./ci-cd.md)
