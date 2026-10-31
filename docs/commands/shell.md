# ctx shell

Interactive shell for exploring your codebase.

## Synopsis

```bash
ctx shell [OPTIONS]
```

## Description

`ctx shell` starts an interactive REPL over the index. It offers:

- short commands for searches and call graph queries,
- tab completion of command names,
- history you can browse with the arrow keys and search with `Ctrl+R`,
- emacs (default) or vi editing.

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

## Shell commands

| Command | Description | Example |
|---------|-------------|---------|
| `find <pattern>` | Find symbols whose name contains the pattern | `find Handler` |
| `search <query>` | Hybrid search (text, plus semantic when embeddings exist) | `search "user login"` |
| `callers <symbol>` | Show functions that call a symbol | `callers handleRequest` |
| `callees <symbol>` | Show functions a symbol calls | `callees main` |
| `deps <symbol>` | Alias for `callees` | `deps UserService` |
| `impact <symbol>` | Show what a change to a symbol would affect | `impact validateToken` |
| `source <symbol>` | Show stored source code | `source authenticate` |
| `explain <symbol>` | Symbol details and relationships | `explain Config` |
| `complexity` | High-complexity functions | `complexity` |
| `audit` | Code quality audit | `audit` |
| `stats` | Show codebase statistics | `stats` |
| `help` / `?` | Show available commands | `help` |
| `exit` / `quit` / `q` | Exit the shell | `exit` |

### Built-in commands

| Command | Description |
|---------|-------------|
| `cd <path>` | Scope `find`, `source`, and `explain` to a path. `cd` with no argument clears the scope |
| `pwd` | Show the current path scope, or `(root)` |
| `clear` | Clear the screen |

`cd` and `pwd` set a path context that filters `find`, `source`, and `explain`. They do not change your
working directory. The prompt shows the active scope, for example `ctx (src/auth)>`.

## Examples

### Start the shell

```bash
ctx shell
```

Output:
```
ctx interactive shell - type 'help' for commands, 'exit' to quit

ctx> _
```

### Find symbols

```
ctx> find handle
Found 4 symbols:
  handleRequest (function) - src/server.rs:45
  handleAuth (function) - src/auth.rs:23
  handleError (function) - src/error.rs:12
  RequestHandler (struct) - src/types.rs:89
```

### View source

```
ctx> source handleRequest
// src/server.rs:45
pub async fn handleRequest(req: Request) -> Response {
    let auth = handleAuth(&req)?;
    let result = process(req, auth).await;
    handleError(result)
}
```

### Explore the call graph

```
ctx> callers handleAuth
Callers of 'handleAuth' (depth 2):
  handleRequest (function) - src/server.rs [distance: 1]
  middleware (function) - src/middleware.rs [distance: 1]
  serve (function) - src/main.rs [distance: 2]
```

### Get statistics

```
ctx> stats
Codebase Statistics:
  Files:     156
  Symbols:   2847
  Functions: 1234
  Structs:   456
  Enums:     38
  Traits:    123
  Edges:     8901
```

## Editing modes

### Emacs mode (default)

| Key | Action |
|-----|--------|
| `Ctrl+A` | Beginning of line |
| `Ctrl+E` | End of line |
| `Ctrl+K` | Delete to end |
| `Ctrl+U` | Delete to beginning |
| `Ctrl+R` | Reverse search history |
| `Tab` | Complete a command name |

### Vi mode

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

The shell saves command history to `~/.ctx_history` by default. Browse it with the arrow keys or
search it with `Ctrl+R`. (`help` lists a `history` command, but it only prints a pointer to the arrow keys.)

```bash
# Custom history location
ctx shell --history /path/to/history

# Disable history (for sensitive sessions)
ctx shell --no-history
```

## Tips

- `Tab` completes command names. Arguments are not completed.
- `find` matches substrings, so `find Handler` finds `RequestHandler` and `HandlerMap`.
- Use `--vi` if you prefer vi keybindings.

## See also

- [Code intelligence](../code-intelligence.md)
- [Configuration](../configuration.md)
