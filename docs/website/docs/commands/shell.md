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
| `find <pattern>` | Find symbols by name (use `*` for wildcards) | `find *Handler` |
| `search <query>` | Hybrid search (text + semantic) | `search "user login"` |
| `callers <symbol>` | Show functions that call a symbol | `callers handleRequest` |
| `callees <symbol>` | Show functions a symbol calls | `callees main` |
| `deps <symbol>` | Alias for `callees` | `deps UserService` |
| `impact <symbol>` | Show what a change to a symbol would affect | `impact validateToken` |
| `source <symbol>` | Show stored source code | `source authenticate` |
| `explain <symbol>` | Symbol details and relationships | `explain Config` |
| `complexity` | Fan-out complexity report | `complexity` |
| `audit` | Code-quality audit | `audit` |
| `stats` | Show codebase statistics | `stats` |
| `help` / `?` | Show available commands | `help` |
| `exit` / `quit` / `q` | Exit the shell | `exit` |

### Built-in Commands

| Command | Description |
|---------|-------------|
| `cd <path>` | Scope subsequent queries to a path; `cd` with no argument clears the scope |
| `pwd` | Show the current path scope (or `(root)`) |
| `clear` | Clear the screen |

> `cd`/`pwd` set a **path context** that filters query results — they do not change your OS
> working directory. Use them to focus the shell on a subdirectory.

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

Command history is saved to `~/.ctx_history` by default. Navigate it with the arrow keys or
search it with `Ctrl+R`.

```bash
# Custom history location
ctx shell --history /path/to/history

# Disable history (for sensitive sessions)
ctx shell --no-history
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
