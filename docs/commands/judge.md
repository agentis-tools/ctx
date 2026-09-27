# `ctx judge`

Model-assisted judgments over the index. Opt-in: nothing runs unless you invoke
it or enable it in `.ctx/config.toml`.

## `ctx judge edges`

Tree-sitter extraction records every call by name. ctx's resolver then binds a
name to an in-repo function when it can — which goes wrong for common names:
`map.get(k)` or `iter.find(..)` get bound to whatever function in the repository
is called `get` or `find`, and calls it cannot disambiguate stay unresolved.

`ctx judge edges` asks a decision model (TypeSafe Jev) one question per call
whose name matches at least one in-repo function or method: *which definition
does this call resolve to?* The options are the same-named candidates (up to 30,
same file and same directory first) plus `external`. The answer rewrites the
edge: bind, re-bind, or unbind as external. Answers below `--min-confidence`
(default 0.7) leave the edge as the resolver left it.

```bash
export JEV_API_KEY=...
ctx judge edges                 # ask, cache, rewrite
ctx judge edges --dry-run       # report what would change
ctx judge edges --offline       # re-apply cached answers only (no network)
ctx judge edges --limit 200     # cap new model calls
```

| Flag | Default | Meaning |
|---|---|---|
| `--offline` | off | Apply cached answers only; never call the model |
| `--dry-run` | off | Report changes without rewriting edges |
| `--limit N` | 0 (no limit) | Stop after N new model calls |
| `--min-confidence X` | `[judge] min_confidence` or 0.7 | Answers below X change nothing |
| `--concurrency N` | 8 | Parallel requests |

### Caching and re-indexing

Every answer is stored in the `judgments` table, keyed by a SHA-256 of the model
id, the question version and the exact state sent. `ctx index` rebuilds edges
for changed files; the next `ctx judge edges` re-applies cached answers for every
call whose context did not change and only asks about the rest. With
`[judge] edges = true` this happens automatically after `ctx index` (offline if
`JEV_API_KEY` is not set).

```toml
# .ctx/config.toml
[judge]
edges = true            # run after every ctx index
model = "jev-latest"
min_confidence = 0.7
```

The API key is only read from the `JEV_API_KEY` environment variable, never
from the config file.

### What is sent

Per ambiguous call: the calling function's source (up to 3 000 characters), its
file's `use`/`import` lines, the call-site line, and the candidates' names,
paths and signatures. See [privacy](../privacy.md).

### Measured effect

On ctx's own index (4 082 call edges labelled by rust-analyzer):

| | accuracy | link precision | link recall | external calls bound to repo functions |
|---|---|---|---|---|
| resolver only | 0.828 | 0.868 | 0.891 | 390 |
| + `ctx judge edges` | 0.994 | 0.998 | 0.993 | 7 |

A full run over 5 805 edges took 121 s at concurrency 10; an offline replay 4 s.

## Exit codes

`0` success (including nothing to judge); `2` operational error (no index,
missing `JEV_API_KEY`, API failure).
