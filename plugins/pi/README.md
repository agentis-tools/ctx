# ctx for pi

A [pi](https://pi.dev) package that plugs [ctx](https://github.com/agentis-tools/ctx), the local code
world model, into every pi session. Its north star is to **make any model, and small local
models in particular, code better in a real repository.**

Small models are weakest at exactly what agent harnesses ask of them: choosing among many tools,
following long routing instructions, and budgeting their own context. This extension takes those
decisions away from the model by default:

```
your prompt ─▶ Jev routes it ─▶ a ctx plan runs ─▶ a token-budgeted brief is added to the prompt
               (~200 ms typed call)  (deterministic,     (plain text, sized to the
                                      30–150 ms)          model's free context window)

model edits ─▶ reindex + architecture check on every edit ─▶ quality gate at the end of the run
```

[Jev](https://docs.typesafe.ai/introduction) is TypeSafe's System One model: it answers typed
questions (choice, score, yes/no) with calibrated confidence in about 200 ms, for far less than an
LLM round-trip. For every prompt it decides **whether the codebase matters at all**, **which ctx
workflow fits**, **how large the task is**, and, when several symbols match, **which one is meant**.
The rest is ordinary code.

## Requirements

- pi (`@earendil-works/pi-coding-agent`)
- `ctx` 0.4.0 or newer on `PATH` (`cargo install agentis-ctx`); the extension runs `ctx index` itself
- Optional: `JEV_API_KEY` for Jev routing. Without it the extension routes with an offline keyword
  router and says so in `/ctx`.

## Install

```bash
pi install npm:@agentis-tools/pi-ctx          # every project
pi install -l npm:@agentis-tools/pi-ctx       # this project only (.pi/settings.json)
pi -e ./plugins/pi                            # try it from a ctx checkout without installing
```

```bash
export JEV_API_KEY=...        # same variable `ctx judge` uses; TYPESAFE_API_KEY also works
pi
```

The footer shows what ctx did for each prompt, for example `ctx · change · jev 1.00 · ~490 tok`.

## What a brief looks like

For `Rename Indexer::index_file to reindex_file and update all callers` on the ctx repository
(8K-window model, about 490 tokens):

```text
[ctx] Repository context for this request (workflow: change; routed by Jev, confidence 1.00).
Selected from the ctx code index. Prefer it over reading whole files; verify against the files before editing.

## Source: Indexer::index_file (src/index/mod.rs:654-729)
    pub fn index_file(&mut self, path: &Path) -> io::Result<bool> { …

## Callers of Indexer::index_file (each must keep working after the change)
- watch_and_index (function, src/index/mod.rs:1058) calls it at line 1147
(1 indirect callers at depth 2)
Next steps:
- Update every caller listed above; after editing, the extension re-checks architecture rules automatically.
```

## Workflows

| Workflow | Chosen when the request… | ctx plan |
|---|---|---|
| `none` | doesn't need this repository | nothing is injected |
| `orient` | asks how the repo is organized | `map` |
| `locate` | asks where or how something works | `query find` → `source` + `explain`, else `search` |
| `implement` | adds new functionality | `similar` (reuse first) + `smart`; without embeddings, the files named after the feature (outline + entry point) |
| `change` | renames, refactors or changes existing code | target `source` + `query callers` + `query impact` |
| `debug` | fixes a failing test or error | target `source` + in-repo `query deps` + next suspect's source |
| `review` | reviews pending changes | `score` + `check` against the default branch |
| `health` | asks about hotspots, complexity, duplication | `hotspots` + `check` |

When Jev's confidence is below `CTX_PI_MIN_CONFIDENCE` (default 0.45), a lighter plan runs (named
symbols or a small map) and the tools for the runner-up workflows are suggested. Semantic steps
degrade to keyword search automatically when `ctx embed` has not been run.

## Routing modes

| Mode | Flag / env | Behaviour |
|---|---|---|
| `jev` (default) | `--ctx-routing jev` | Jev decides. Any Jev failure falls back to `keyword` for that prompt; two consecutive network failures pause Jev for 5 minutes so an offline machine isn't slowed down |
| `keyword` | `--ctx-routing keyword` | Offline deterministic router, no network |
| `model` | `--ctx-routing model` | The model decides: all ctx tools plus a map on the first prompt, like the Claude Code plugin |

Jev routing is the default for strong models too, because it is far cheaper than spending LLM
tokens on tool selection.

## Tools

Five tools, each with one or two plain string parameters (small models call simple schemas
reliably). Output is plain text, truncated to fit the context window.

| Tool | Use |
|---|---|
| `ctx_find` | find symbols by name or description |
| `ctx_source` | read one symbol's source instead of the whole file |
| `ctx_impact` | callers and blast radius before changing code |
| `ctx_similar` | check for existing code before writing new code |
| `ctx_check` | reindex and run architecture rules + quality gate now |

`--ctx-tools stable` (default) keeps all five active so the system prompt, and the local model's
prompt cache, stays stable across prompts. `--ctx-tools routed` activates only the tools for the
routed workflow, which gives the model fewer choices but invalidates the prompt cache when the set changes.

## Quality gates

- After each successful `edit`/`write`: `ctx index` then `ctx check --against <session start commit>`.
  New violations are appended to that tool result, so the model sees them while it can still fix them.
- At the end of a run that edited files (including shell edits): `ctx score --against <session start commit> --fail-on <expr>`.
  - `report` (default): a failed gate's scorecard is added to the session. A passing gate only shows in the footer and does not use the model's context.
  - `block`: a failed gate asks pi for another model turn, at most `CTX_PI_MAX_GATE_CONTINUATIONS` (default 1) per run.
  - `off`: no reindexing or gates.
- The agent cannot write `.ctx/rules.toml` (any spelling of the path, and common shell rewrites) or run
  `ctx self-update`, matching the Claude and Codex plugins.
- A gate that cannot run (exit code 2) is reported as **not passed**, never as clean.

## Configuration

Flags win over environment variables.

| Setting | Flag | Environment | Default |
|---|---|---|---|
| Routing | `--ctx-routing` | `CTX_PI_ROUTING` | `jev` |
| Jev key | — | `JEV_API_KEY` or `TYPESAFE_API_KEY` | unset (keyword routing) |
| Gate | `--ctx-gate` | `CTX_PI_GATE`, or `CTX_GATE_BLOCKING=1` | `report` |
| Gate expression | — | `CTX_PI_FAIL_ON` | `check_violations>0` |
| Tool activation | `--ctx-tools` | `CTX_PI_TOOLS` | `stable` |
| Show briefs in the transcript | `--ctx-show` | `CTX_PI_SHOW_CONTEXT` | off (always sent to the model) |
| Skip threshold (Jev "needs codebase") | — | `CTX_PI_GATE_THRESHOLD` | 0.3 |
| Light-plan threshold | — | `CTX_PI_MIN_CONFIDENCE` | 0.45 |
| Max brief size (tokens) | — | `CTX_PI_MAX_INJECT_TOKENS` | 6000 |
| Briefs kept in context | — | `CTX_PI_KEEP_CONTEXT` | 2 |
| ctx binary | — | `CTX_BIN` | `ctx` |

A brief uses at most 15% of the model's free context window, scaled by task size, with a floor of
400 tokens and the cap above. Once the context is half full, older briefs (beyond
`CTX_PI_KEEP_CONTEXT`) and older gate reports are dropped from what the model sees. This happens
only under pressure, because rewriting earlier messages invalidates a local model's prompt cache.
If the first prompt arrives while the initial `ctx index` is still running (more than 15 s), that
prompt runs without a brief instead of waiting.

## Privacy

ctx runs locally. With Jev routing, each prompt sends TypeSafe:

- your prompt (first 2,000 characters) and the previous prompt (first 500);
- repository counts (files, symbols, functions) and whether semantic search is available;
- when disambiguating a symbol: candidate symbol names, kinds and file paths.

**No source code is sent.** Use `--ctx-routing keyword` to keep everything on the machine.

## Commands

- `/ctx`: status (ctx version, routing, gate, index size)
- `/ctx last`: the last routing decision, the commands run, and the brief
- `/ctx route <text>`: preview how a prompt would be routed

## Measure it

`eval/` runs the same tasks with and without the extension on any pi model and reports pass
rate, tokens, tool calls and time:

```bash
cd plugins/pi && npm install
node eval/run.ts --model ollama/qwen2.5-coder:7b --modes baseline,jev,keyword,model --runs 3
```

Tasks are pinned to a ctx commit with verified answers (`eval/tasks.json`); add your own
repository and tasks the same way. See `eval/README.md`.

## Development

```bash
npm install
npm run check            # typecheck + unit tests + e2e (e2e needs ctx; set CTX_BIN)
pi -e ./src/index.ts     # run pi with the working copy
```

The tests run TypeScript directly with `node --test`, which needs Node 22.6 or newer. The extension
itself runs on whatever Node pi runs on.

The end-to-end tests drive real pi against a scripted OpenAI-compatible model and a throwaway
repository with an architecture rule. They assert on what pi actually sends to the model.

## License

MIT OR Apache-2.0
