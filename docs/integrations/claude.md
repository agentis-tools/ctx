# Claude Integration

Connect ctx to Claude Code (hooks, skill, plugin) and Claude Desktop (MCP).

## Claude Code (recommended): `ctx harness init`

The fastest way to wire the ctx quality suite into [Claude Code](https://docs.anthropic.com/en/docs/claude-code) is the harness scaffolder:

```bash
cd /path/to/your/project
ctx index
ctx harness init
```

This generates hook scripts under `.claude/hooks/ctx/` (session-start codebase map, post-edit architecture check, stop-time quality scorecard), a starter `.ctx/rules.toml`, and prints a settings snippet to merge into `.claude/settings.json` plus a guidance block for your `CLAUDE.md`. ctx never edits `.claude/settings.json` itself.

The hooks are version-guarded and fail open: if the installed ctx binary is older than the templates that generated them, they warn on stderr and do nothing instead of blocking the session. Verify the integration with:

```bash
ctx harness doctor
```

To package the same integration as a distributable Claude Code plugin (hooks + skill + MCP wiring + permissions), use:

```bash
ctx harness init --mode plugin
```

To use the published v0.3.5 plugin directly, load its release ZIP for a
session:

```bash
claude --plugin-url https://github.com/agentis-tools/ctx/releases/download/v0.3.5/ctx-claude-plugin-0.3.5.zip
```

For development, clone the repository and run
`claude --plugin-dir ./plugins/claude/ctx`. The canonical plugin source is
[`plugins/claude/ctx`](../../plugins/claude/ctx). It runs locally and invokes
the `ctx` binary on `PATH`; the published default plugin does not configure
MCP. Generate an MCP-enabled variant only with a ctx binary built using
`--features mcp`.

See [`ctx harness`](../commands/harness.md) for the full reference. If you prefer to wire the hooks by hand, the manual `settings.json` reference lives in the [Quality Gates guide](./quality-gates.md).

For details about local processing and optional network-backed features, see
the [ctx Privacy Policy](../privacy.md).

## Claude Desktop (MCP)

Connect ctx to Claude Desktop for AI-powered codebase exploration.

### Overview

ctx implements the Model Context Protocol (MCP), allowing Claude to directly query your codebase through standardized tools. This enables conversations like:

> "Find all functions that call the authenticate method"
> "Show me the source code for the UserService class"
> "What files should I look at to add caching?"

## Quick Setup

### 1. Build ctx with MCP Support

```bash
cargo build --features mcp --release
cp target/release/ctx /usr/local/bin/
```

### 2. Index Your Project

```bash
cd /path/to/your/project
ctx index
ctx embed  # Optional, enables smart context
```

### 3. Configure Claude Desktop

Edit the Claude Desktop configuration file:

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

Add ctx as an MCP server:

```json
{
  "mcpServers": {
    "ctx": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/your/project"
    }
  }
}
```

### 4. Restart Claude Desktop

Close and reopen Claude Desktop to load the new configuration.

## Verification

After setup, you should see a hammer icon in Claude Desktop indicating MCP tools are available. Try asking:

- "What MCP tools do you have available?"
- "Search for functions containing 'auth' in the codebase"
- "Show me the definition of the main function"

## Available Tools

Claude can use these ctx tools:

| Tool | Description | Example Prompt |
|------|-------------|----------------|
| `search_symbols` | Find symbols by name | "Find all functions with 'handle' in the name" |
| `get_definition` | View source code | "Show me the code for authenticate" |
| `find_references` | Find usages | "Where is UserService used?" |
| `get_file` | Read file contents | "Show me the contents of config.rs" |
| `get_file_tree` | List project files | "What files are in the src directory?" |
| `get_callers` | Who calls this? | "What calls the validate function?" |
| `get_callees` | What does this call? | "What functions does main call?" |
| `smart_context` | AI file selection | "What files should I modify to add logging?" |

## Example Workflows

### Understanding Code

**You**: I'm new to this codebase. Can you give me an overview of the main components?

**Claude**: *Uses get_file_tree and search_symbols to explore, then summarizes the architecture*

### Finding Related Code

**You**: I need to modify how errors are handled. What files should I look at?

**Claude**: *Uses smart_context with "error handling" task, returns relevant files*

### Debugging

**You**: The authenticate function is failing. What code paths lead to it?

**Claude**: *Uses get_callers and get_definition to trace the call stack*

### Code Review

**You**: Can you review the handleRequest function for potential issues?

**Claude**: *Uses get_definition to fetch the code, analyzes it*

## Multiple Projects

Configure multiple codebases with unique names:

```json
{
  "mcpServers": {
    "frontend": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/frontend"
    },
    "backend": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/backend"
    },
    "shared": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/shared-lib"
    }
  }
}
```

Then specify which project in your prompts:

> "In the backend project, find all database query functions"

## Tips for Better Results

### Be Specific

Instead of: "Show me the auth code"
Try: "Show me the authenticate function in the auth module"

### Use Technical Terms

Instead of: "Where do we check if users are allowed in?"
Try: "Find the authorization middleware functions"

### Ask for Context First

Instead of: "Fix the bug in handleRequest"
Try: "First, show me the handleRequest function and its callers, then help me understand the data flow"

### Iterate

Start broad, then narrow down:
1. "Search for functions related to caching"
2. "Show me the definition of CacheService"
3. "What calls the invalidate method?"

## Troubleshooting

### "No MCP tools available"

1. Verify ctx is installed: `ctx --version`
2. Check config file syntax (must be valid JSON)
3. Restart Claude Desktop completely
4. Check logs for errors

### "Symbol not found"

1. Ensure codebase is indexed: `ctx query stats`
2. Re-index if code changed: `ctx index`
3. Try broader search terms

### "Smart context not working"

1. Generate embeddings: `ctx embed`
2. Verify embeddings exist: `ctx query stats` (check embedding count)

### Slow Responses

1. Re-index with latest changes: `ctx index`
2. Reduce smart_context depth/tokens
3. Be more specific in queries

## Security

- ctx only reads from the index database
- No code execution capabilities
- No file modification capabilities
- Access limited to indexed project directory

Consider:
- Using read-only filesystem permissions
- Not indexing sensitive files (.env, credentials)
- Reviewing what gets indexed via `.contextignore`

## See Also

- [MCP Server Command](../commands/serve.md)
- [Smart Context](../commands/smart.md)
- [Configuration](../configuration.md)
