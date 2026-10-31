---
id: claude
title: Claude integration
sidebar_position: 2
---

# Claude integration

Connect ctx to Claude Code (hooks, skill, plugin) and Claude Desktop (MCP).

## Claude Code: `ctx harness init`

The quickest way to wire the ctx quality suite into [Claude Code](https://docs.anthropic.com/en/docs/claude-code) is the harness scaffolder:

```bash
cd /path/to/your/project
ctx index
ctx harness init
```

This generates hook scripts under `.claude/hooks/ctx/` (a codebase map at session start, an architecture check after each edit, and a quality scorecard at stop) and a starter `.ctx/rules.toml`. It adds the hook entries and ctx permissions to `.claude/settings.json`, creating the file or merging into an existing one. If the existing file is not valid JSON, ctx leaves it untouched and prints the snippet to merge by hand. It also prints a guidance block for your `CLAUDE.md`.

The hooks check the ctx version and fail open: if the installed binary is older than the templates that generated them, they warn on stderr and do nothing. Verify the integration with:

```bash
ctx harness doctor
```

[Quality gates](./quality-gates.md) describes what each hook does, including the opt-in blocking Stop hook (`CTX_GATE_BLOCKING=1`).

### Plugin

To package the same integration as a distributable Claude Code plugin (hooks, skill, permissions, and MCP wiring when the binary supports it), run:

```bash
ctx harness init --mode plugin
```

To use the published v0.4.0 plugin, load its release ZIP for a session:

```bash
claude --plugin-url https://github.com/agentis-tools/ctx/releases/download/v0.4.0/ctx-claude-plugin-0.4.0.zip
```

For development, clone the repository and run `claude --plugin-dir ./plugins/claude/ctx`. The plugin source is [`plugins/claude/ctx`](https://github.com/agentis-tools/ctx/tree/main/plugins/claude/ctx). It runs locally and calls the `ctx` binary on `PATH`. The published plugin does not configure MCP, because ctx adds MCP wiring only when the generating binary was built with `--features mcp`.

See [`ctx harness`](../commands/harness.md) for the full reference. For local processing and the optional network-backed features, see the [ctx privacy policy](../privacy.md).

## Claude Desktop (MCP)

ctx implements the Model Context Protocol (MCP), so Claude Desktop can query your codebase through ctx tools. That supports requests such as:

> "Find all functions that call the authenticate method"
> "Show me the source code for the UserService class"
> "What files should I look at to add caching?"

The MCP server (`ctx serve --mcp`) exists only in builds with the `mcp` feature. Release binaries, package-manager installs, and a default `cargo install agentis-ctx` do not include it.

### 1. Install ctx with MCP support

```bash
cargo install agentis-ctx --locked --features mcp
```

### 2. Index your project

```bash
cd /path/to/your/project
ctx index
ctx embed  # optional; the smart_context tool needs embeddings
```

### 3. Configure Claude Desktop

Edit the Claude Desktop configuration file:

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

`ctx serve` serves the project in its working directory, so start it from the project root. On macOS and Linux:

```json
{
  "mcpServers": {
    "ctx": {
      "command": "sh",
      "args": ["-c", "cd /path/to/your/project && exec ctx serve --mcp"]
    }
  }
}
```

On Windows, use `"command": "cmd"` with `"args": ["/c", "cd /d C:\\path\\to\\project && ctx serve --mcp"]`.

### 4. Restart Claude Desktop

Quit and reopen Claude Desktop to load the new configuration.

## Verification

After setup, Claude Desktop lists the ctx tools in its tools menu. Try asking:

- "What MCP tools do you have available?"
- "Search for functions containing 'auth' in the codebase"
- "Show me the definition of the main function"

## Available tools

| Tool | Description | Example prompt |
|------|-------------|----------------|
| `search_symbols` | Find symbols by name | "Find all functions with 'handle' in the name" |
| `get_definition` | Show a symbol's source | "Show me the code for authenticate" |
| `find_references` | Find usages | "Where is UserService used?" |
| `get_file` | Read a file in the project | "Show me the contents of config.rs" |
| `get_file_tree` | List project files | "What files are in the src directory?" |
| `get_callers` | Find callers, up to `depth` levels (default 3, max 5) | "What calls the validate function?" |
| `get_callees` | Find calls and the definition each resolves to | "What functions does main call?" |
| `smart_context` | Select files for a task within a token budget | "What files should I modify to add logging?" |

`get_callers` and `find_references` follow resolved call edges. Calls that match by name but were never resolved to the definition are listed separately as possible callers. When several definitions share a name, the tools list up to five of them instead of picking one. See [ctx serve](../commands/serve.md) for the tool parameters.

## Example workflows

### Understanding code

**You**: I'm new to this codebase. Can you give me an overview of the main components?

**Claude**: *Uses get_file_tree and search_symbols to explore, then summarizes the architecture*

### Finding related code

**You**: I need to modify how errors are handled. What files should I look at?

**Claude**: *Uses smart_context with an "error handling" task and returns the relevant files*

### Debugging

**You**: The authenticate function is failing. What code paths lead to it?

**Claude**: *Uses get_callers and get_definition to trace the call stack*

### Code review

**You**: Can you review the handleRequest function for potential issues?

**Claude**: *Uses get_definition to fetch the code, then analyzes it*

## Multiple projects

Give each codebase its own server name:

```json
{
  "mcpServers": {
    "frontend": {
      "command": "sh",
      "args": ["-c", "cd /path/to/frontend && exec ctx serve --mcp"]
    },
    "backend": {
      "command": "sh",
      "args": ["-c", "cd /path/to/backend && exec ctx serve --mcp"]
    }
  }
}
```

Then name the project in your prompts:

> "In the backend project, find all database query functions"

## Tips for better results

### Be specific

Instead of "Show me the auth code", try "Show me the authenticate function in the auth module".

### Use technical terms

Instead of "Where do we check if users are allowed in?", try "Find the authorization middleware functions".

### Ask for context first

Instead of "Fix the bug in handleRequest", try "First show me the handleRequest function and its callers, then help me understand the data flow".

### Narrow down step by step

1. "Search for functions related to caching"
2. "Show me the definition of CacheService"
3. "What calls the invalidate method?"

## Troubleshooting

### "No MCP tools available"

1. Check that ctx is installed: `ctx --version`.
2. Check that the build includes MCP: `ctx serve --mcp` should start and wait for input instead of failing. If it fails, reinstall with `cargo install agentis-ctx --locked --features mcp`.
3. Check the config file syntax (it must be valid JSON).
4. Quit and restart Claude Desktop.
5. Check the Claude Desktop MCP logs for errors.

### "Symbol not found"

1. Check that the codebase is indexed: `ctx query stats`.
2. Re-index after code changes: `ctx index`.
3. Try broader search terms.

### "Smart context not working"

1. Generate embeddings: `ctx embed`.
2. Check the embedding count: `ctx query stats`.
3. If you changed embedding provider or model, rebuild the corpus: `ctx embed --force --provider <provider>`.

### Slow responses

1. Re-index with the latest changes: `ctx index`.
2. Lower the smart_context depth or token budget.
3. Ask more specific questions.

## Security

- The tools read the index and project files. None of them execute code or modify files.
- `get_file` only reads files inside the project root.
- `smart_context` calls an embedding provider. With the default local provider nothing leaves the machine; with OpenAI or a remote Ollama host, the task text is sent to that provider.

Also consider:

- Excluding sensitive files (`.env`, credentials) from the index with `.contextignore`.
- Running Claude Desktop with read-only filesystem permissions on the project.

## See also

- [ctx serve](../commands/serve.md)
- [ctx smart](../commands/smart.md)
- [Configuration](../configuration.md)
