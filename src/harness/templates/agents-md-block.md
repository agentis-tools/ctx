## Code intelligence (ctx)

Use `ctx map --budget 2000` to orient yourself, `ctx index` after code changes,
`ctx check --against HEAD --json` for architecture rules, and
`ctx score --against {{DEFAULT_BRANCH}}` before finishing. Files under
`.codex/hooks/ctx/` are generated; change `.ctx/rules.toml` to customize policy.
