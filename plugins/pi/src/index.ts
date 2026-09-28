/**
 * ctx for pi — grounding and governance for coding agents, tuned for small
 * local models.
 *
 * North star: make any model, and small local models in particular, code
 * better in a real repository. Small models are weakest at choosing tools,
 * following long routing instructions and budgeting their own context, so
 * by default this extension takes those decisions away from the model:
 *
 *   prompt ──▶ Jev routes it (one ~200 ms typed call) ──▶ a deterministic
 *   ctx plan runs ──▶ a token-budgeted plain-text brief is injected
 *
 * and after edits it keeps the index fresh and runs ctx's quality gates.
 * `--ctx-routing model` hands the decision back to the model (tools +
 * guidelines only), `--ctx-routing keyword` routes offline without Jev.
 */
import { resolve } from "node:path";
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { injectionBudget, MIN_INJECT_TOKENS } from "./budget.ts";
import { resolveConfig, type Config } from "./config.ts";
import { Ctx } from "./ctx-cli.ts";
import { Governance } from "./governance.ts";
import { Router, createJevClient } from "./router/index.ts";
import { registerTools } from "./tools.ts";
import type { Exec, RepoMeta, RouteDecision } from "./types.ts";
import { CTX_TOOLS, renderContext, runWorkflow, type PlanResult } from "./workflows.ts";

export const CONTEXT_MESSAGE_TYPE = "ctx-context";
export const GATE_MESSAGE_TYPE = "ctx-gate";
export const ROUTE_ENTRY_TYPE = "ctx-route";

/**
 * How long a prompt waits for session setup (the initial `ctx index`). Past
 * this, the prompt proceeds without a brief rather than stalling the user.
 */
const SETUP_WAIT_MS = 15_000;

/**
 * Old briefs are pruned only once the context is this full. Rewriting earlier
 * messages invalidates the provider's prompt cache (expensive on local
 * llama.cpp/Ollama models), so it is done only when space is actually needed.
 */
const PRUNE_AT_PERCENT = 50;

interface SessionState {
  cfg: Config;
  ctx?: Ctx;
  meta?: RepoMeta;
  router: Router;
  governance?: Governance;
  ready: Promise<void>;
  unavailableReason?: string;
  version?: string;
  previousPrompt?: string;
  /** Git ref the review workflow compares against. */
  reviewBase?: string;
  promptsSeen: number;
  last?: { decision: RouteDecision; plan?: PlanResult; injected?: string; budget?: number };
}

export default function ctxExtension(pi: ExtensionAPI): void {
  pi.registerFlag("ctx-routing", {
    type: "string",
    description: "ctx: who picks the ctx workflow per prompt: jev (default) | keyword | model",
  });
  pi.registerFlag("ctx-gate", {
    type: "string",
    description: "ctx: quality gate after edits: report (default) | block | off",
  });
  pi.registerFlag("ctx-tools", {
    type: "string",
    description: "ctx: stable (default; all ctx tools stay active) | routed (only the tools for the routed workflow)",
  });
  pi.registerFlag("ctx-show", { type: "boolean", description: "ctx: show injected context in the transcript" });

  let s: SessionState | undefined;

  const exec: Exec = (command, args, options) => pi.exec(command, args, options);

  registerTools(pi, {
    getCtx: () => s?.ctx,
    getMeta: () => s?.meta,
    resultTokens: () => Math.max(MIN_INJECT_TOKENS, Math.round((s?.cfg.maxInjectTokens ?? 6000) * 0.5)),
    runCheck: async (signal) => (s?.governance ? s.governance.checkNow(signal) : "Quality gates are disabled (--ctx-gate off)."),
  });

  pi.on("session_start", (_event, ec) => {
    const cfg = resolveConfig((n) => pi.getFlag(n), process.env);
    const client = cfg.routing === "jev" && cfg.jevApiKey ? createJevClient(cfg.jevApiKey) : undefined;
    const state: SessionState = {
      cfg,
      router: new Router(cfg.routing, client),
      ready: Promise.resolve(),
      promptsSeen: 0,
    };
    s = state;
    // Setup runs in the background: startup must not wait for indexing.
    state.ready = setup(state, ec).catch((err) => {
      state.unavailableReason = `setup failed: ${(err as Error)?.message ?? err}`;
    });
  });

  async function setup(state: SessionState, ec: ExtensionContext): Promise<void> {
    const ctx = new Ctx((c, a, o) => exec(c, a, { ...o, cwd: ec.cwd }), ec.cwd, state.cfg.ctxBin);
    const avail = await ctx.available();
    if (!avail.ok) {
      state.unavailableReason = avail.reason;
      status(ec, `ctx off: ${avail.reason}`);
      if (ec.hasUI) ec.ui.notify(`ctx extension inactive: ${avail.reason}`, "warning");
      return;
    }
    state.version = avail.version;
    status(ec, "ctx indexing…");
    const idx = await ctx.index();
    if (idx.code !== 0) {
      state.unavailableReason = `ctx index failed: ${idx.stderr.trim().split("\n").pop()}`;
      status(ec, "ctx off: index failed");
      return;
    }
    const head = await exec("git", ["rev-parse", "HEAD"], { cwd: ec.cwd, timeout: 10_000 }).catch(() => undefined);
    const baseline = head && head.code === 0 ? head.stdout.trim() : undefined;
    state.meta = await ctx.repoMeta(baseline !== undefined);
    state.ctx = ctx;
    state.governance = new Governance(ctx, state.cfg.gate, state.cfg.failOn, state.cfg.maxGateContinuations, baseline);
    state.reviewBase = baseline ? ((await defaultBranch(ec.cwd)) ?? baseline) : undefined;

    if (state.cfg.tools === "stable" || state.cfg.routing === "model") {
      pi.setActiveTools([...new Set([...pi.getActiveTools(), ...CTX_TOOLS])]);
    }
    status(ec, `ctx ${state.version} · ${state.router.status()}`);
  }

  async function defaultBranch(cwd: string): Promise<string | undefined> {
    const r = await exec("git", ["rev-parse", "--abbrev-ref", "origin/HEAD"], { cwd, timeout: 10_000 }).catch(() => undefined);
    if (r && r.code === 0 && r.stdout.trim() && r.stdout.trim() !== "origin/HEAD") return r.stdout.trim();
    for (const b of ["main", "master"]) {
      const v = await exec("git", ["rev-parse", "--verify", "--quiet", b], { cwd, timeout: 10_000 }).catch(() => undefined);
      if (v && v.code === 0) return b;
    }
    return undefined;
  }

  pi.on("before_agent_start", async (event, ec) => {
    const state = s;
    if (!state) return;
    const ready = await Promise.race([state.ready.then(() => true), sleep(SETUP_WAIT_MS).then(() => false)]);
    if (!ready) {
      status(ec, "ctx · still indexing; this prompt runs without a brief");
      return;
    }
    if (!state.ctx || !state.meta) return;
    state.governance?.startRun();
    state.promptsSeen++;

    const prompt = event.prompt.trim();
    const previousPrompt = state.previousPrompt;
    state.previousPrompt = prompt;
    if (prompt.length < 3) return;

    const usage = ec.getContextUsage();
    const window = usage?.contextWindow ?? (ec.model as { contextWindow?: number } | undefined)?.contextWindow;

    // Model-driven mode: orient once (like the Claude SessionStart hook), then
    // leave tool choice to the model.
    if (state.cfg.routing === "model") {
      if (state.promptsSeen > 1) return;
      const budget = Math.min(1500, injectionBudget(window, usage?.tokens, 1, state.cfg.maxInjectTokens));
      const m = await state.ctx.map(budget, undefined, { signal: ec.signal });
      if (m.code !== 0 || !m.stdout.trim()) return;
      const content =
        "[ctx] Repository map from the ctx index. Use ctx_find, ctx_source, ctx_impact, ctx_similar and ctx_check " +
        "to look things up instead of reading whole files.\n\n" +
        m.stdout.trim();
      return { message: { customType: CONTEXT_MESSAGE_TYPE, content, display: state.cfg.showContext } };
    }

    const decision = await state.router.route({ prompt, previousPrompt, meta: state.meta }, ec.signal);
    state.last = { decision };
    pi.appendEntry(ROUTE_ENTRY_TYPE, { decision, prompt: prompt.slice(0, 300) });

    if (decision.workflow === "none" || decision.needsCodebase < state.cfg.gateThreshold) {
      if (state.cfg.tools === "routed") setCtxTools([]);
      status(ec, `ctx · no context needed (${decision.source})`);
      return;
    }

    const budget = injectionBudget(window, usage?.tokens, decision.scope, state.cfg.maxInjectTokens);
    const light = decision.confidence < state.cfg.minConfidence;
    const plan = await runWorkflow(
      { ctx: state.ctx, router: state.router, prompt, decision, budget, meta: state.meta, reviewBase: state.reviewBase, signal: ec.signal },
      light,
    );
    if (state.cfg.tools === "routed") setCtxTools(plan.tools);
    state.last = { decision, plan, budget };
    if (!plan.sections.length) {
      status(ec, `ctx · ${decision.workflow}: nothing found`);
      return;
    }

    const content = renderContext(plan, decision, budget);
    state.last.injected = content;
    status(ec, `ctx · ${decision.workflow}${light ? " (light)" : ""} · ${decision.source} ${decision.confidence.toFixed(2)} · ~${Math.round(content.length / 4)} tok`);
    return {
      message: {
        customType: CONTEXT_MESSAGE_TYPE,
        content,
        display: state.cfg.showContext,
        details: { decision, commands: plan.commands, budget, light },
      },
    };
  });

  function setCtxTools(wanted: readonly string[]): void {
    const base = pi.getActiveTools().filter((t) => !(CTX_TOOLS as readonly string[]).includes(t));
    pi.setActiveTools([...base, ...wanted]);
  }

  // Under context pressure keep only the newest ctx briefs and gate reports:
  // on an 8K model stale briefs crowd out the conversation.
  pi.on("context", (event, ec) => {
    const percent = ec.getContextUsage()?.percent;
    if (percent == null || percent < PRUNE_AT_PERCENT) return;
    const messages = pruneCustom(pruneCustom(event.messages, CONTEXT_MESSAGE_TYPE, s?.cfg.keepContextMessages ?? 2), GATE_MESSAGE_TYPE, 1);
    return messages.length === event.messages.length ? undefined : { messages };
  });

  // Same guardrails as the Claude/Codex plugins: the agent may not weaken the
  // policy it is being checked against, or replace the ctx binary mid-session.
  // Active from session start (not only once indexing finished).
  pi.on("tool_call", (event, ec) => {
    if (!s || s.cfg.gate === "off") return;
    const input = event.input as { path?: unknown; command?: unknown };
    if ((event.toolName === "edit" || event.toolName === "write") && typeof input.path === "string" && isRulesFile(input.path, ec.cwd)) {
      return {
        block: true,
        reason: "ctx: .ctx/rules.toml is the architecture policy the change is checked against; fix the code, not the rules (ask the user if the rule itself is wrong).",
      };
    }
    if (event.toolName === "bash" && typeof input.command === "string") {
      if (/\bctx\s+self-update\b/.test(input.command)) return { block: true, reason: "ctx: self-update is not allowed from the agent." };
      if (/\.ctx\/+(\.\/)*rules\.toml\b/.test(input.command) && /(>|\bsed\b[^|;&]*\s-i|\btee\b|\bmv\b|\bcp\b|\brm\b|\btruncate\b|\bperl\b[^|;&]*\s-i|\bpython3?\b)/.test(input.command)) {
        return { block: true, reason: "ctx: modifying .ctx/rules.toml from the shell is not allowed; fix the code, not the rules." };
      }
    }
  });

  pi.on("tool_result", async (event, ec) => {
    const gov = s?.governance;
    if (!gov?.enabled || event.isError) return;
    if (event.toolName === "bash") {
      // Shell edits (sed -i, heredocs) are not checked per call, but they still
      // arm the end-of-run gate.
      gov.markDirty();
      return;
    }
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const note = await gov.afterEdit(ec.signal);
    if (!note) return;
    return { content: [...event.content, { type: "text" as const, text: note }] };
  });

  pi.on("agent_before_settle", async (event, ec) => {
    const gov = s?.governance;
    if (!gov?.enabled || event.outcome === "aborted") return;
    const r = await gov.atSettle(ec.signal);
    if (!r) return;
    if (ec.hasUI) ec.ui.notify(r.text.split("\n")[0], r.failed || r.error ? "warning" : "info");
    if (!r.failed && !r.error) {
      // A clean gate is shown to the user but not added to the model's context.
      status(ec, "ctx · gate passed");
      return;
    }
    return {
      entries: [...event.entries, { type: "custom_message", customType: GATE_MESSAGE_TYPE, content: r.text, display: true }],
      continue: event.continue || r.continue,
    };
  });

  pi.registerCommand("ctx", {
    description: "ctx status, or '/ctx last' for the last brief, '/ctx route <text>' to preview routing",
    handler: async (args, ec) => {
      const state = s;
      const [sub, ...rest] = args.trim().split(/\s+/);
      const say = (t: string) => (ec.hasUI ? ec.ui.notify(t, "info") : console.log(t));
      if (!state) return say("ctx: no session");
      if (sub === "last") {
        if (!state.last) return say("ctx: nothing routed yet");
        const d = state.last.decision;
        return say(
          `route: ${d.workflow} via ${d.source} (conf ${d.confidence.toFixed(2)}, needs ${d.needsCodebase.toFixed(2)}, modify ${d.willModify.toFixed(2)}, scope ${d.scope.toFixed(1)}, ${d.latencyMs} ms)` +
            (state.last.plan ? `\ncommands: ${state.last.plan.commands.join(" ; ")}` : "") +
            (state.last.injected ? `\n\n${state.last.injected}` : ""),
        );
      }
      if (sub === "route") {
        const d = await state.router.route({ prompt: rest.join(" "), meta: state.meta });
        return say(JSON.stringify(d, null, 2));
      }
      await state.ready;
      say(
        [
          `ctx ${state.version ?? "?"}${state.unavailableReason ? ` — inactive: ${state.unavailableReason}` : ""}`,
          `routing: ${state.router.status()} · tools: ${state.cfg.tools} · gate: ${state.cfg.gate} (${state.cfg.failOn})`,
          state.meta ? `index: ${state.meta.files} files, ${state.meta.symbols} symbols, embeddings ${state.meta.embeddings}` : "index: not ready",
        ].join("\n"),
      );
    },
  });
}

/**
 * Does a tool path point at the repository's `.ctx/rules.toml`? Resolves the
 * path the way pi's file tools do (leading `@` stripped, relative to cwd), so
 * `@.ctx/rules.toml`, `.ctx//rules.toml` and `./.ctx/./rules.toml` all match.
 */
export function isRulesFile(path: string, cwd: string): boolean {
  const p = path.trim().replace(/^@/, "");
  return resolve(cwd, p) === resolve(cwd, ".ctx", "rules.toml");
}

/** Drop all but the newest `keep` custom messages of one type. */
function pruneCustom<T>(messages: T[], customType: string, keep: number): T[] {
  const isType = (m: T) => (m as { role?: string }).role === "custom" && (m as { customType?: string }).customType === customType;
  let drop = messages.filter(isType).length - keep;
  if (drop <= 0) return messages;
  return messages.filter((m) => !(isType(m) && drop-- > 0));
}

function status(ec: ExtensionContext, text: string): void {
  if (ec.hasUI) ec.ui.setStatus("ctx", text);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms).unref?.());
}
