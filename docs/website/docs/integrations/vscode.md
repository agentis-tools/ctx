---
id: vscode
title: VS Code integration
sidebar_position: 3
---

# VS Code integration

ctx has no VS Code extension. You use it from the integrated terminal, from tasks, and alongside AI assistants.

## Terminal commands

Open the integrated terminal (``Ctrl+` ``) and run ctx commands:

```bash
# Index the workspace
ctx index

# Search for symbols
ctx search "handleRequest"

# Preview which files ctx would select for a task (needs `ctx embed` first)
ctx smart "add caching to database queries" --dry-run
```

## Tasks

### Configure tasks

Create `.vscode/tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "ctx: Index",
      "type": "shell",
      "command": "ctx index",
      "group": "build",
      "problemMatcher": []
    },
    {
      "label": "ctx: Index (Watch)",
      "type": "shell",
      "command": "ctx index --watch",
      "isBackground": true,
      "problemMatcher": []
    },
    {
      "label": "ctx: Audit",
      "type": "shell",
      "command": "ctx audit",
      "group": "test",
      "problemMatcher": []
    },
    {
      "label": "ctx: Search",
      "type": "shell",
      "command": "ctx search \"${input:searchQuery}\"",
      "problemMatcher": []
    },
    {
      "label": "ctx: Smart Context",
      "type": "shell",
      "command": "ctx smart \"${input:taskDescription}\" --dry-run",
      "problemMatcher": []
    }
  ],
  "inputs": [
    {
      "id": "searchQuery",
      "type": "promptString",
      "description": "Symbol to search for"
    },
    {
      "id": "taskDescription",
      "type": "promptString",
      "description": "Task description for smart context"
    }
  ]
}
```

### Run tasks

1. Press `Ctrl+Shift+P` (`Cmd+Shift+P` on macOS).
2. Type "Tasks: Run Task".
3. Select a ctx task.

### Keyboard shortcuts

Add to your user `keybindings.json` (Command Palette: "Preferences: Open Keyboard Shortcuts (JSON)"):

```json
[
  {
    "key": "ctrl+shift+i",
    "command": "workbench.action.tasks.runTask",
    "args": "ctx: Index"
  },
  {
    "key": "ctrl+shift+a",
    "command": "workbench.action.tasks.runTask",
    "args": "ctx: Audit"
  }
]
```

## With AI assistants

### Selecting files for an assistant

1. Run `ctx smart "your task" --dry-run` to list the relevant files.
2. Open those files in VS Code.
3. Ask your assistant (for example, Copilot Chat) about them.

```bash
# List the files for your task
ctx smart "add caching to database queries" --dry-run

# Open the suggested files
code src/db/queries.rs src/cache.rs
```

To give the assistant the files' contents directly, drop `--dry-run` and paste or attach the output. `--max-tokens` caps the size of the rendered context (default 8,000).

### Over MCP

Assistants that support the Model Context Protocol can call ctx tools directly through `ctx serve --mcp`. The MCP server needs a ctx build with the `mcp` feature (`cargo install agentis-ctx --locked --features mcp`). See [ctx serve](../commands/serve.md) for the tools and configuration.

## Navigating from terminal output

### Find callers

```bash
ctx query callers handleRequest
```

Each caller is printed as `name (path:line)`. Ctrl+click (Cmd+click on macOS) the path in the terminal to open it.

### Impact analysis

```bash
# See what a change to this symbol would affect
ctx query impact validateInput
```

Review the affected files before you change the symbol.

## Workspace settings

Add to `.vscode/settings.json`:

```json
{
  "terminal.integrated.env.osx": {
    "PATH": "${env:PATH}:/usr/local/bin"
  },
  "files.watcherExclude": {
    "**/.ctx/**": true
  }
}
```

The `PATH` entry is only needed if ctx is installed in a directory VS Code's terminal does not already search. Excluding `.ctx/` stops VS Code from watching the index files.

## Index automatically

This task refreshes the index each time you open the folder:

```json
{
  "label": "ctx: Auto Index",
  "type": "shell",
  "command": "ctx index",
  "runOptions": {
    "runOn": "folderOpen"
  },
  "presentation": {
    "reveal": "silent"
  }
}
```

To keep the index current while you edit, run watch mode on folder open instead:

```json
{
  "label": "ctx: Watch",
  "type": "shell",
  "command": "ctx index --watch",
  "isBackground": true,
  "runOptions": {
    "runOn": "folderOpen"
  }
}
```

VS Code asks once per workspace before it runs `folderOpen` tasks automatically.

## Tips

1. Run `ctx index --watch` in a terminal so the index updates as you edit.
2. `path:line` references in ctx output are clickable in the terminal.
3. Save common ctx commands as tasks.

## Troubleshooting

### ctx command not found

1. Check that the directory containing `ctx` is on your `PATH`.
2. Restart VS Code.
3. Check your terminal profile settings.

### Slow indexing

1. Add large directories to `.contextignore`.
2. Check that generated and vendored directories are in `.gitignore`.
3. Use watch mode, which reindexes only changed files.

## See also

- [Getting started](../getting-started.md)
- [Configuration](../configuration.md)
- [Claude integration](./claude.md)
