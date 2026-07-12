# Codex Integration

Wire ctx into Codex with repository-local lifecycle hooks:

```bash
ctx harness init --target codex
```

This generates `.codex/hooks.json`, scripts under `.codex/hooks/ctx/`, and a starter
`.ctx/rules.toml`. Add the printed guidance block to `AGENTS.md`, open `/hooks` in
Codex, and review and trust the generated definitions. Project hooks load only for
trusted repositories.

The session hook supplies a codebase map, the post-edit hook refreshes the index and
checks architecture rules, and the stop hook runs the quality scorecard. Set
`CTX_GATE_BLOCKING=1` to make failed stop-time gates continue the Codex turn.

For a distributable plugin instead, run:

```bash
ctx harness init --target codex --mode plugin
codex plugin marketplace add ./
```

The plugin includes hooks, the ctx skill, and MCP wiring when ctx was built with the
`mcp` feature.
