---
id: vscode
title: VS Code Integration
sidebar_position: 3
---

# VS Code Integration

Use ctx with Visual Studio Code for enhanced code intelligence.

## Overview

While ctx is primarily a CLI tool, it integrates well with VS Code through:

- **Terminal integration** - Run ctx commands directly
- **Tasks** - Automated indexing and auditing
- **Extensions** - Use with AI coding assistants

## Terminal Commands

Open the integrated terminal (`Ctrl+`` `) and run ctx commands:

```bash
# Index the workspace
ctx index

# Search for symbols
ctx search "handleRequest"

# Get context for current work
ctx smart "what I'm working on"
```

## VS Code Tasks

### Configure Tasks

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

### Run Tasks

1. Press `Ctrl+Shift+P` (or `Cmd+Shift+P` on macOS)
2. Type "Tasks: Run Task"
3. Select a ctx task

### Keyboard Shortcuts

Add to `.vscode/keybindings.json`:

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

## With GitHub Copilot

Use ctx to provide context to Copilot Chat:

1. Run `ctx smart "your task"` to get relevant files
2. Open those files in VS Code
3. Ask Copilot about them with full context

### Workflow Example

```bash
# Get files for your task
ctx smart "add caching to database queries" --dry-run

# Open the suggested files
code src/db/queries.rs src/cache.rs

# Now Copilot has the right context!
```

## With Continue.dev

[Continue](https://continue.dev) is an open-source AI coding assistant that can use ctx.

### Configure Continue

Add to `.continue/config.json`:

```json
{
  "contextProviders": [
    {
      "name": "ctx",
      "params": {
        "command": "ctx smart"
      }
    }
  ]
}
```

### Usage

In Continue chat:
- `@ctx add user authentication` - Get relevant files for the task

## Code Lens Integration

While ctx doesn't provide a VS Code extension directly, you can use the output in your workflow:

### Find Callers

```bash
# Find what calls a function
ctx query callers handleRequest

# Output shows file:line references
# Ctrl+click in terminal to navigate
```

### Impact Analysis

```bash
# See what would be affected by changes
ctx query impact validateInput

# Review affected files before modifying
```

## Workspace Settings

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

## Auto-Index on Save

Create a task that runs on folder open:

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

Or use the watch mode:

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

## Tips

1. **Use watch mode** - Run `ctx index --watch` in a terminal for auto-updates
2. **Leverage terminal links** - File:line output is clickable
3. **Create snippets** - Save common ctx commands as VS Code snippets
4. **Use tasks** - Automate common workflows
5. **Combine with AI** - Use ctx output to provide context to AI assistants

## Troubleshooting

### ctx command not found

1. Ensure ctx is in your PATH
2. Restart VS Code
3. Check terminal profile settings

### Slow indexing

1. Add large directories to `.contextignore`
2. Use `.gitignore` patterns
3. Enable watch mode for incremental updates

## See Also

- [Getting Started](../getting-started.md)
- [Configuration](../configuration.md)
- [Claude Desktop Integration](./claude.md)
