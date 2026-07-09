---
id: shell
title: ctx shell
sidebar_position: 4
---

# ctx shell

Interactive shell for exploring your codebase.

## Synopsis

```bash
ctx shell [OPTIONS]
```

## Description

The `shell` command starts an interactive REPL for exploring your indexed codebase. It provides:

- **Quick searches** - Find symbols without typing full commands
- **Tab completion** - Auto-complete commands and arguments
- **History** - Navigate previous commands with arrow keys
- **Vi/Emacs modes** - Choose your preferred editing style

## Prerequisites

Index your codebase first:

```bash
ctx index
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--history <PATH>` | History file location | `~/.ctx_history` |
| `--no-history` | Disable command history | false |
| `--vi` | Use vi editing mode | false (emacs) |

## Shell Commands

| Command | Description | Example |
|---------|-------------|---------|
| `find <pattern>` | Search symbols by name | `find auth` |
| `search <query>` | Semantic search | `search "user login"` |
| `callers <symbol>` | Show callers | `callers handleRequest` |
| `callees <symbol>` | Show callees | `callees main` |
| `deps <symbol>` | Show dependencies | `deps UserService` |
| `source <symbol>` | Show source code | `source authenticate` |
| `explain <symbol>` | Detailed symbol info | `explain Config` |
| `files` | List indexed files | `files` |
| `stats` | Show codebase stats | `stats` |
| `help` | Show available commands | `help` |
| `exit` / `quit` | Exit the shell | `exit` |

### Built-in Commands

| Command | Description |
|---------|-------------|
| `cd <path>` | Change working directory |
| `pwd` | Show current directory |
| `history` | Show command history |
| `clear` | Clear screen |

## Examples

### Start Shell

```bash
ctx shell
```

Output:
```
ctx shell - Interactive codebase explorer
Type 'help' for available commands, 'exit' to quit.

ctx> _
```

### Find Symbols

```
ctx> find handle
Found 12 symbols:
  handleRequest     function  src/server.rs:45
  handleAuth        function  src/auth.rs:23
  handleError       function  src/error.rs:12
  RequestHandler    struct    src/types.rs:89
  ...
```

### View Source

```
ctx> source handleRequest
// src/server.rs:45-67
pub async fn handleRequest(req: Request) -> Response {
    let auth = handleAuth(&req)?;
    let result = process(req, auth).await;
    handleError(result)
}
```

### Explore Call Graph

```
ctx> callers handleAuth
handleAuth is called by:
  handleRequest  src/server.rs:46
  middleware     src/middleware.rs:23
  testAuth       tests/auth_test.rs:15
```

### Get Statistics

```
ctx> stats
Codebase Statistics:
  Files indexed:    156
  Total symbols:    2,847
  Functions:        1,234
  Structs/Classes:  456
  Interfaces:       123
  Edges:            8,901
```

## Editing Modes

### Emacs Mode (Default)

| Key | Action |
|-----|--------|
| `Ctrl+A` | Beginning of line |
| `Ctrl+E` | End of line |
| `Ctrl+K` | Delete to end |
| `Ctrl+U` | Delete to beginning |
| `Ctrl+R` | Reverse search history |
| `Tab` | Auto-complete |

### Vi Mode

```bash
ctx shell --vi
```

| Mode | Key | Action |
|------|-----|--------|
| Normal | `i` | Insert mode |
| Normal | `a` | Append |
| Normal | `0` | Beginning of line |
| Normal | `$` | End of line |
| Normal | `/` | Search history |
| Insert | `Esc` | Normal mode |

## History

Command history is saved to `~/.ctx_history` by default.

```bash
# Custom history location
ctx shell --history /path/to/history

# Disable history (for sensitive sessions)
ctx shell --no-history
```

In the shell:
```
ctx> history
1: find auth
2: callers handleAuth
3: source handleAuth
4: stats
```

## Tips

- Use `Tab` for command and symbol completion
- Use `Ctrl+R` to search command history
- Prefix patterns with `*` for wildcard: `find *Handler`
- Use `--vi` if you prefer vi keybindings
- Run `help <command>` for detailed help on any command

## See Also

- [Code Intelligence](../code-intelligence.md)
- [Configuration](../configuration.md)
