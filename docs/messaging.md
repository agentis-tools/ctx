# ctx messaging & positioning (source of truth)

> Internal. This is the canonical positioning for `ctx`. Every public surface — homepage hero,
> `intro.md`, `why-ctx.md`, `comparison.md`, the agent guide, `llms.txt`, and the README opening —
> must express *this* "why" consistently. When copy and this file disagree, fix the copy.

## The thesis

**ctx is the local quality authority for AI-written code** — a queryable model of your codebase that
grounds your agent and gates its output, every turn.

The pain it names: **AI coding agents write fast, sloppy code** — they duplicate logic that already
exists, drift the architecture, and declare "done" without proof. ctx fixes that. It hands the agent
a map before it starts, shows the blast radius of every edit, and enforces your rules as
deterministic gates the agent can't ship past — locally, in the loop.

Three ideas, always unpacked in concrete terms so it never reads as jargon:

- **World model** — ctx indexes your repo into a structured, queryable model: symbols, call graphs,
  relationships, and semantics. Not a bag of files — a model an agent (or you) can ask questions of.
- **Ground** — it feeds the language model accurate, token-budgeted context, selected by meaning
  *and* call-graph relevance, so the model's changes reflect how the code actually works instead of
  what it guessed. (Retrieval / "read the right ~2,000 lines, not the wrong 200,000.")
- **Govern** — it puts guardrails on what the model changes: impact analysis for the blast radius of
  an edit, and quality/complexity/duplication audits you can gate CI on. (Oversight, not just input.)

The world model powers both: **ground/guide** is what goes *in* to the model; **govern/gate** is
control over what comes *out*. ("guide" and "gate" are the customer-facing verbs for the same two
pillars.)

### Canonical taglines (reuse verbatim)

- **Full:** ctx is the local quality authority for AI-written code — a queryable model of your
  codebase that grounds your agent and gates its output, every turn.
- **Compressed (README/social):** Unlike code-graph tools, ctx governs what agents *write* — not just
  what they read. Unlike quality platforms, it does so in milliseconds, locally, with gates you own.
- **Category line:** Code-graph tools help agents read your code. Quality platforms audit it after
  the fact, from a server. ctx is the only tool that governs what agents write, while they write it,
  locally.

## The problem (why anyone should care)

Language models now write and modify real code, but they operate blind. Given a repo they either:

1. **Read too much** — dump whole directories into the prompt. Burns tokens, blows the window, and
   buries the signal. (A grounding failure.)
2. **Read too little** — grep a few files, edit, and miss the caller three hops away that just
   broke. (Also grounding — and nothing *governs* the change before it lands.)

Grep and file-dumpers don't understand code. Raw embeddings find similar code but miss the
relationships. And almost nothing checks a model's change against the structure of the codebase
*before* it ships. There's no model of the world the agent is editing, and no guardrails on it.

## The solution

ctx builds the world model once (`ctx index`), then uses it to both ground and govern:

**Ground — the right context, in:**
- **Smart context** — `ctx smart "<task>"` ranks by meaning + call-graph relevance, then fits
  whole files to a token budget.
- **Token control** — `--count-only --encoding cl100k_base` measures file content; `--max-tokens`
  budgets selected content, while rendered wrappers and the optional tree add output tokens.

**Govern — guardrails, on what changes:**
- **Architecture rules** — `ctx check` enforces `.ctx/rules.toml` (layers, forbidden deps, limits)
  over the real edge graph, diff-scoped for PR/agent gating.
- **Composite gate** — `ctx score --fail-on` folds check violations, new duplication, and
  complexity/fan-out deltas into one pass/fail scorecard.
- **Impact, hotspots, duplicates** — `ctx query impact` (blast radius), `ctx hotspots`
  (churn × complexity), MinHash `ctx duplicates --fail-on-found`.
- **In the agent loop** — `ctx harness init --target claude` wires check/score/map into Claude Code
  hooks; exit codes are the contract (0 clean / 1 findings / 2 error).
- *Forthcoming (not yet shipped): `ctx sql` repo-committed SQL gates (`.ctx/gates/*.sql`), trend snapshots.*

**Agent-native** — `ctx serve --mcp` exposes the whole world model as MCP tools; `--output json`
everywhere. **Local by default** — Rust, one SQLite file, local embeddings; the v0.4.0 tag
indexed 2,701 symbols and 18,730 extracted edges in this repository. Configured providers may
have their own local or network dependencies.

## Audience

- **Primary:** developers using AI coding agents (Claude Code, Cursor) who want the agent grounded in
  their real codebase and guardrailed against breaking it.
- **Secondary:** tool/agent builders who need a code world model + retrieval layer via MCP/JSON
  instead of building code-RAG and static analysis themselves.

## Differentiators (name them)

| Category | Examples | What they are | What ctx adds |
|---|---|---|---|
| File packers | repomix, gitingest, files-to-prompt | Concatenate files into a blob | A *model* of the code (call graphs, impact), semantic + structural selection, governance |
| IDE indexers | ctags, LSP | A code model for human editors | A model built to **ground and govern LLMs**: LLM-ready context, semantic search, impact gates, MCP |
| DIY code-RAG | homegrown scripts | Similarity search over chunks | Purpose-built, local, one binary; structural **and** semantic; plus governance, not just retrieval |

**The line:** *a queryable world model — grounds the model's input, governs its output. Structural +
semantic, agent-first, local.*

## Proof points (ctx v0.4.0 tag on the ctx repo; see the reproducible commands below)

- **Ground:** the v0.4.0 tag at `1425cfc5a5ba96802a7adf7a9271bd1f1c3c1dda` reports **502,856
  file-content tokens** from
  `ctx --count-only --encoding cl100k_base` at the repository root; wrappers and the tree are
  excluded.
- **World model:** the same tag's default `ctx index` run reports **2,701 symbols and 18,730
  extracted edges**; timing is machine-dependent and intentionally not claimed here.
- **Govern:** `ctx query impact discover_files` shows a change ripples through `index`, `run_context`,
  `run` (the CLI entry point) and the MCP server across 5 hops.

## Primary CTA

`cargo install agentis-ctx` → point your agent at the world model over MCP, or pipe grounded context
in one command.

## Voice

Confident, concrete, developer-to-developer. Lead with the outcome, unpack the abstract terms
immediately, show real terminal sessions, let the numbers carry it. Always pair "world model / ground
/ govern" with a concrete command or result.
