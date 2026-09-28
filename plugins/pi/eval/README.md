# pi-ctx evaluation

Does the extension help? `run.ts` answers that on your hardware and your model by running each
task through pi in four modes, each in a fresh copy of the repository:

| Mode | What runs |
|---|---|
| `baseline` | pi with no extensions (built-in read/grep/find/edit/bash) |
| `jev` | this extension with Jev routing (needs `JEV_API_KEY`) |
| `keyword` | this extension with the offline keyword router |
| `model` | this extension's tools only; the model decides when to use them |

## Run

1. Point pi at a local model in `~/.pi/agent/models.json` (Ollama example):

   ```json
   {
     "providers": {
       "ollama": {
         "baseUrl": "http://localhost:11434/v1",
         "api": "openai-completions",
         "apiKey": "ollama",
         "models": [{ "id": "qwen2.5-coder:7b", "contextWindow": 32768 }]
       }
     }
   }
   ```

   Set `contextWindow` to what the server actually serves (Ollama's `num_ctx`). The extension sizes
   its briefs from it.

2. Run:

   ```bash
   cd plugins/pi && npm install
   export CTX_BIN=$(which ctx) JEV_API_KEY=...
   node eval/run.ts --model ollama/qwen2.5-coder:7b --modes baseline,jev,keyword,model --runs 3
   ```

   Options: `--only id1,id2`, `--tasks other.json`, `--timeout 600` (seconds per run), `--full`
   (also runs each edit task's `slow_verify`, e.g. `cargo check`).

The first run clones the pinned repository into `eval/.cache/` and indexes it. Results go to
`eval/results/<timestamp>/` (`results.jsonl` per run, `summary.md` per mode and task).

## Reading the numbers

- **Pass rate** is the north-star metric. Run at least 3 runs per mode, because small models are noisy.
- **Input tokens** include the injected brief. A brief that replaces several `read` calls usually
  *lowers* total input tokens over a multi-turn run.
- **Tool calls** measure how much searching the model had to do itself.
- `control-no-code` checks that the extension stays out of the way when the codebase is irrelevant.

## Adding tasks

Each task needs a `prompt` and either `expect` (case-insensitive regexes that must all match the
final answer) or `verify` (a shell command run in the task's copy after pi exits; exit 0 = pass).
Verify every answer against the pinned commit before adding it. A wrong key makes every mode
look worse in the same way, and hides real differences.
