---
id: serve
title: ctx serve
sidebar_position: 5
---

# ctx serve

Run ctx as an MCP (Model Context Protocol) server for AI assistants.

## Synopsis

```bash
ctx serve --mcp
```

> `ctx serve` exists only in builds with the `mcp` feature. The release binaries and a
> default `cargo install agentis-ctx` do not include it. Build or install with
> `--features mcp` (for example `cargo install agentis-ctx --features mcp`).

## Description

`ctx serve --mcp` runs an MCP server over stdio for the project in the current directory. An assistant such as Claude can then call its tools to search symbols, read definitions and files, follow the call graph, and request a token-budgeted set of files for a task.

## Prerequisites

1. Index your codebase:
   ```bash
   ctx index
   ```

2. Generate embeddings (needed by `smart_context`):
   ```bash
   ctx embed
   ```

3. Build ctx with MCP support:
   ```bash
   cargo build --features mcp --release
   ```

## Options

| Option | Description |
|--------|-------------|
| `--mcp` | Run as MCP server over stdio |

`ctx serve` takes no path argument. It serves the directory it starts in.

## MCP tools

The server exposes eight tools.

### Search tools

| Tool | Description |
|------|-------------|
| `search_symbols` | Search for symbols by name pattern |
| `get_definition` | Get the source of a definition. Exact name matches win, and up to five same-named definitions are listed |
| `find_references` | Find references that resolve to the requested definition. Unbound same-name calls are listed separately as possible references |

### File tools

| Tool | Description |
|------|-------------|
| `get_file` | Read a file under the project root |
| `get_file_tree` | List files in the project, filtered by path and pattern |

### Analysis tools

| Tool | Description |
|------|-------------|
| `get_callers` | Find callers of the requested definition through resolved edges. `depth` defaults to 3, maximum 5 |
| `get_callees` | Find calls made by a function, with the definition each call resolved to |
| `smart_context` | Select files for a task. Takes `task`, `max_tokens` (a hard limit on the whole response, default 8000), `depth` (default 2), `top` (default 10), and `provider` (`local`, `openai`, or `ollama`) |

## Claude Desktop integration

### Configuration

Add ctx to your Claude Desktop configuration file:

- macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
- Windows: `%APPDATA%\Claude\claude_desktop_config.json`
- Linux: `~/.config/Claude/claude_desktop_config.json`

Claude Desktop starts MCP servers with `/` as the working directory and does not reliably honor a `cwd` key. Start ctx through a shell that changes to the project first:

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

On Windows:

```json
{
  "mcpServers": {
    "ctx": {
      "command": "cmd",
      "args": ["/c", "cd /d C:\\path\\to\\project && ctx serve --mcp"]
    }
  }
}
```

### Multiple projects

Give each project its own server name:

```json
{
  "mcpServers": {
    "my-app": {
      "command": "sh",
      "args": ["-c", "cd /path/to/my-app && exec ctx serve --mcp"]
    },
    "shared-lib": {
      "command": "sh",
      "args": ["-c", "cd /path/to/shared-lib && exec ctx serve --mcp"]
    }
  }
}
```

### Verification

After restarting Claude Desktop:

1. Open a new conversation.
2. Check that the tools indicator lists the ctx tools.
3. Ask Claude: "What tools do you have for exploring my codebase?"

## Example conversations

### Finding code

**You**: Where is the authentication logic in my codebase?

**Claude** (using `search_symbols`): I found several authentication-related symbols:
- `authenticate` function in `src/auth.rs:42`
- `AuthService` struct in `src/services/auth.rs:15`
- `validateToken` function in `src/middleware.rs:78`

### Understanding code

**You**: Explain how the handleRequest function works

**Claude** (using `get_definition` and `get_callers`): 
The `handleRequest` function in `src/server.rs` is the main entry point for HTTP requests. Here's what it does:

```rust
pub async fn handleRequest(req: Request) -> Response {
    // ...
}
```

It's called by:
- The main server loop in `src/main.rs`
- The test harness in `tests/integration.rs`

### Selecting files for a task

**You**: I need to add caching to the database queries

**Claude** (using `smart_context`): Based on your task, here are the relevant files:
- `src/db/queries.rs` - Main query functions
- `src/db/connection.rs` - Database connection handling
- `src/cache.rs` - Existing cache implementation
- `src/config.rs` - Configuration where cache settings would go

## Troubleshooting

### Server not starting

Check that:
1. ctx is in your PATH: `which ctx`. Claude Desktop may not see your shell's PATH, so use the full path to `ctx` in the `cd ... && exec` command if needed.
2. The codebase is indexed: `ctx query stats`
3. The build includes MCP: `ctx serve --mcp` should not error. If it does, rebuild with `--features mcp`.

### Tools not available

1. Restart Claude Desktop after config changes.
2. Check that the config file is valid JSON.
3. Check that the path in the `cd` command exists and is indexed.

### Slow responses

1. Make sure embeddings exist: `ctx embed`
2. Make sure the index is current: `ctx index`
3. Pass a smaller `depth` to `smart_context`.

## Security considerations

- The server reads the index and files under the project root. `get_file` refuses paths outside the root.
- It has no tool that runs code or modifies files.
- Consider read-only filesystem permissions in production.

## See also

- [Claude Desktop integration](../integrations/claude.md)
- [Code intelligence](../code-intelligence.md)
- [ctx smart](./smart.md)
