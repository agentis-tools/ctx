---
id: quality-gates
title: Quality gates
sidebar_position: 4
---

# Quality gates

Wire the ctx quality suite into CI pipelines and AI coding agents.

## Exit codes are the API

Every ctx quality command uses the same three exit codes, like `grep` and most linters:

| Code | Meaning |
|------|---------|
| 0 | Success, nothing to report |
| 1 | The command ran and produced findings |
| 2 | Operational error (bad arguments, missing index, git failure) |

A shell `&&`, a CI step, or an agent hook can enforce a gate from the exit code alone. Use `--json` when a tool needs the details. An operational error exits 2, never 0 or 1, so a broken gate fails the build instead of passing it. See [Exit codes](../reference/exit-codes.md) for the full table.

## The suite

| Command | Question it answers | Gate |
|---------|--------------------|------|
| [`ctx check`](../commands/check.md) | Does this change violate our architecture rules? | exits 1 on violations |
| [`ctx hotspots`](../commands/hotspots.md) | Where does refactoring pay off most (churn × size by default, or churn × complexity)? | informational |
| [`ctx duplicates`](../commands/duplicates.md) | Which functions are structural near-copies? | `--fail-on-found` |
| [`ctx similar`](../commands/similar.md) | Does the function I'm about to write already exist? | informational |
| [`ctx score`](../commands/score.md) | Did this change make the code better or worse? | `--fail-on "metric OP value,..."` |
| [`ctx map`](../commands/map.md) | What does this codebase look like, in N tokens? | informational |

`ctx score` is the composite gate. It combines check violations, new duplication, complexity and fan-out deltas, and symbol churn into one scorecard, and `--fail-on` turns any of them into a failing condition.

## CI example

```yaml
# GitHub Actions
- name: Index codebase
  run: ctx index

- name: Architecture rules (new violations only)
  run: ctx check --against origin/main

- name: Quality score gate
  run: ctx score --against origin/main --fail-on "check_violations>0,new_duplication>0,complexity_delta>=25"
```

`--against origin/main` needs the base branch in the checkout. With `actions/checkout`, set `fetch-depth: 0`. [CI/CD integration](./ci-cd.md) shows how to install ctx in the job.

## Claude Code integration

Run `ctx harness init` in the repository. It writes three hook scripts under `.claude/hooks/ctx/`, a starter `.ctx/rules.toml`, and the hook entries in `.claude/settings.json`:

```json
{
  "permissions": {
    "allow": ["Bash(ctx *)"],
    "deny": [
      "Bash(ctx self-update*)",
      "Edit(.ctx/rules.toml)",
      "Edit(.claude/hooks/ctx/**)",
      "Edit(.claude/settings.json)"
    ]
  },
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/ctx/session-start.sh" }] }],
    "PostToolUse": [{ "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/ctx/post-tool-use.sh" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/ctx/stop.sh" }] }]
  }
}
```

What each hook does:

- `session-start.sh` runs `ctx map --budget 2000`. Claude Code adds its output to the session context, so the agent starts with a structural overview.
- `post-tool-use.sh` runs after each `Edit` or `Write`. It refreshes the index incrementally and runs `ctx check --against HEAD`. When the check finds new violations, the hook returns them as `additionalContext`, which Claude Code shows the model next to the tool result. (Claude Code passes a PostToolUse hook's plain output only to its debug log, so a bare `ctx check` command here would not reach the agent.) A clean check prints nothing.
- `stop.sh` runs `ctx score --against <default branch> --fail-on "check_violations>0"` and prints the scorecard. The `new_duplication` metric stays in the scorecard but does not fail the gate, because the duplication heuristic needs calibration for each repository. v0.4.0 hooks also gated on `new_duplication>0`; rerun `ctx harness init` after upgrading to regenerate them.

By default the Stop hook only reports. Set `CTX_GATE_BLOCKING=1` in the environment Claude Code runs in to make a failed gate exit 2, which Claude Code treats as "keep working"; the scorecard then goes to stderr, the only output Claude Code shows the model on exit 2. The hook blocks once per stop: when Claude Code reports that the agent is already continuing because of it (`stop_hook_active`), it prints the findings and lets the session end. If you add `new_duplication` to a blocking Stop hook, `ctx harness init` and `ctx harness doctor` print a warning. Every hook fails open: an operational error, or a ctx binary older than the templates, exits 0 with a note on stderr.

See [ctx harness](../commands/harness.md) for plugin mode, Codex, and upgrades.

### Recommended CLAUDE.md guidance

`ctx harness init` prints a guidance block for `CLAUDE.md`. These extra instructions help the agent avoid findings instead of hitting the gates:

```markdown
## Code quality

- Before writing a new function, run `ctx similar "<what it should do>"` -
  extend or reuse an existing implementation instead of adding a near-copy.
- Before modifying an exported symbol, run `ctx query impact <symbol>` to
  see what depends on it.
- Files flagged by `ctx hotspots` change often and are already large:
  prefer extracting a new module over growing them further.
- Run `ctx check --against HEAD` after editing; the Stop hook runs
  `ctx score --against main` and fails on architecture violations.
```

## Pre-commit hook

```bash
#!/bin/sh
# .git/hooks/pre-commit
ctx index > /dev/null && ctx score --fail-on "check_violations>0,new_duplication>0"
```

With the default `--against HEAD`, `ctx score` scores the uncommitted changes, which includes what is about to be committed.

## See also

- [ctx score](../commands/score.md), [ctx check](../commands/check.md), [ctx duplicates](../commands/duplicates.md)
- [JSON output](../json-output.md): the payload contract for `--json` consumers
- [CI/CD integration](./ci-cd.md)
