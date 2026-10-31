---
id: codex
title: Codex integration
---

# Codex integration

Wire ctx into Codex with repository-local lifecycle hooks:

```bash
ctx harness init --target codex
```

This writes `.codex/hooks.json`, scripts under `.codex/hooks/ctx/`, and a starter
`.ctx/rules.toml`, then prints a guidance block to add to `AGENTS.md`. Codex loads
project hooks only for trusted repositories, so open `/hooks` in Codex and review and
trust the generated definitions.

The hooks do three things:

- At session start, the hook adds `ctx map --budget 2000` to the context.
- After each `Edit` or `Write`, the hook refreshes the index and runs
  `ctx check --against HEAD --json`. When it finds violations, it returns them to
  Codex as additional context with a `block` decision.
- At stop, the hook runs `ctx score --against <default branch> --fail-on "check_violations>0"`
  and passes the scorecard to Codex. Findings do not stop the turn unless you set
  `CTX_GATE_BLOCKING=1`, which makes a failed gate exit 2 so Codex keeps working.

Every hook fails open: an operational error, or a ctx binary older than the hook
templates, never blocks the session.

## Plugin

To generate a distributable plugin instead, run:

```bash
ctx harness init --target codex --mode plugin
codex plugin marketplace add ./
```

Or install the canonical v0.4.0 plugin from GitHub:

```bash
codex plugin marketplace add agentis-tools/ctx --ref main --sparse plugins/codex/ctx
codex plugin add ctx@ctx-local
```

The plugin source is [`plugins/codex/ctx`](https://github.com/agentis-tools/ctx/tree/main/plugins/codex/ctx). It contains the
hooks and the ctx skill. It does not include MCP wiring: ctx adds that only when the
generating binary was built with `--features mcp`, and release binaries are not.

See [ctx harness](../commands/harness.md) for upgrades and `ctx harness doctor`.
