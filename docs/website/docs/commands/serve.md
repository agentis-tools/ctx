---
id: serve
title: ctx serve
sidebar_position: 5
---

# ctx serve

Start ctx as an MCP (Model Context Protocol) server for AI assistant integration.

## Synopsis

```bash
ctx serve --mcp
```

> **Note:** `ctx serve` is only available when ctx is built with the `mcp` feature
> (`cargo build --release --features mcp`). Default builds do not include the MCP server.

## Description

The `serve` command runs ctx as an MCP server over stdio, enabling AI assistants like Claude to query your codebase through standardized tools. This provides:

- **Real-time code intelligence** - AI can search symbols, view definitions, analyze call graphs
- **Context-aware assistance** - AI understands your codebase structure
- **Smart file selection** - AI can request relevant files for tasks

## Prerequisites

1. Index your codebase:
   ```bash
   ctx index
   ```

2. Generate embeddings (for smart context):
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

## Available MCP Tools

When running as an MCP server, ctx exposes these tools:

### Search Tools

| Tool | Description |
|------|-------------|
| `search_symbols` | Search for symbols by name pattern |
| `get_definition` | Get source code for a symbol |
| `find_references` | Find all references to a symbol |

### File Tools

| Tool | Description |
|------|-------------|
| `get_file` | Read a file's contents |
| `get_file_tree` | List files in the project |

### Analysis Tools

| Tool | Description |
|------|-------------|
| `get_callers` | Find functions that call a function |
| `get_callees` | Find functions called by a function |
| `smart_context` | Intelligently select files for a task |

## Claude Desktop Integration

### Configuration

Add ctx to your Claude Desktop configuration file:

**macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
**Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
**Linux**: `~/.config/Claude/claude_desktop_config.json`

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

### Multiple Projects

Configure multiple projects by giving each a unique name:

```json
{
  "mcpServers": {
    "my-app": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/my-app"
    },
    "shared-lib": {
      "command": "ctx",
      "args": ["serve", "--mcp"],
      "cwd": "/path/to/shared-lib"
    }
  }
}
```

### Verification

After restarting Claude Desktop:

1. Open a new conversation
2. You should see a hammer icon indicating MCP tools are available
3. Ask Claude: "What tools do you have for exploring my codebase?"

## Example Conversations

### Finding Code

**You**: Where is the authentication logic in my codebase?

**Claude** (using `search_symbols`): I found several authentication-related symbols:
- `authenticate` function in `src/auth.rs:42`
- `AuthService` struct in `src/services/auth.rs:15`
- `validateToken` function in `src/middleware.rs:78`

### Understanding Code

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

### Smart File Selection

**You**: I need to add caching to the database queries

**Claude** (using `smart_context`): Based on your task, here are the relevant files:
- `src/db/queries.rs` - Main query functions
- `src/db/connection.rs` - Database connection handling
- `src/cache.rs` - Existing cache implementation
- `src/config.rs` - Configuration where cache settings would go

## Troubleshooting

### Server Not Starting

Check that:
1. ctx is in your PATH: `which ctx`
2. The codebase is indexed: `ctx query stats`
3. MCP feature is enabled: `ctx serve --mcp` should not error (rebuild with `--features mcp` if it does)

### Tools Not Available

1. Restart Claude Desktop after config changes
2. Check the config file syntax is valid JSON
3. Verify the `cwd` path exists and is indexed

### Slow Responses

1. Ensure embeddings are generated: `ctx embed`
2. Check index is up to date: `ctx index`
3. Consider reducing `--depth` in smart_context calls

## Security Considerations

- The MCP server only reads from the indexed database
- No code execution or file modification capabilities
- Access limited to the configured project directory
- Consider using read-only filesystem permissions in production

## See Also

- [Claude Desktop Integration](../integrations/claude.md)
- [Code Intelligence](../code-intelligence.md)
- [Smart Context](./smart.md)
