import type { Ctx, Caller, SearchHit } from "./ctx-cli.ts";
import type { Router } from "./router/index.ts";
import type { RepoMeta, RouteDecision, Section, SymbolRef, WorkflowId } from "./types.ts";
import { estimateTokens, truncateToTokens } from "./budget.ts";
import { extract, leafName, type Extracted } from "./symbols.ts";

/** Model-callable ctx tools registered by the extension (see tools.ts). */
export const CTX_TOOLS = ["ctx_find", "ctx_source", "ctx_impact", "ctx_similar", "ctx_check"] as const;
export type CtxToolName = (typeof CTX_TOOLS)[number];

export interface PlanContext {
  ctx: Ctx;
  router: Router;
  prompt: string;
  decision: RouteDecision;
  /** Token budget for everything this plan injects. */
  budget: number;
  meta: RepoMeta;
  /** Git ref to compare against for review (session baseline or default branch). */
  reviewBase?: string;
  signal?: AbortSignal;
}

export interface PlanResult {
  workflow: WorkflowId;
  sections: Section[];
  /** ctx tools most useful for follow-up on this request. */
  tools: CtxToolName[];
  /** One-line next steps phrased for the model. */
  hints: string[];
  target?: SymbolRef;
  /** Commands run, for the transcript details and the eval harness. */
  commands: string[];
}

/** Tools each workflow makes most relevant (used when tools=routed and in hints). */
export const WORKFLOW_TOOLS: Record<WorkflowId, CtxToolName[]> = {
  none: [],
  orient: ["ctx_find", "ctx_source"],
  locate: ["ctx_find", "ctx_source"],
  implement: ["ctx_similar", "ctx_find", "ctx_source"],
  change: ["ctx_impact", "ctx_source", "ctx_find"],
  debug: ["ctx_source", "ctx_find", "ctx_impact"],
  review: ["ctx_check", "ctx_impact"],
  health: ["ctx_find", "ctx_source"],
};

/**
 * Run the deterministic ctx plan for a routed request.
 *
 * Plans never throw for ctx failures; a failed step just contributes nothing.
 * When the router's confidence is below `minConfidence`, callers should pass
 * `light: true` to get a cheaper, more generic plan.
 */
export async function runWorkflow(pc: PlanContext, light = false): Promise<PlanResult> {
  const ex = extract(pc.prompt);
  const w = pc.decision.workflow;
  const res: PlanResult = { workflow: w, sections: [], tools: [...WORKFLOW_TOOLS[w]], hints: [], commands: [] };
  if (w === "none") return res;

  if (light) {
    await lightPlan(pc, ex, res);
    for (const alt of pc.decision.alternatives) for (const t of WORKFLOW_TOOLS[alt]) if (!res.tools.includes(t)) res.tools.push(t);
    return res;
  }

  switch (w) {
    case "orient":
      await orient(pc, ex, res, 0.9);
      break;
    case "locate":
      await locate(pc, ex, res);
      break;
    case "implement":
      await implement(pc, ex, res);
      break;
    case "change":
      await change(pc, ex, res);
      break;
    case "debug":
      await debug(pc, ex, res);
      break;
    case "review":
      await review(pc, res);
      break;
    case "health":
      await health(pc, res);
      break;
  }
  return res;
}

// ---------------------------------------------------------------------------
// Plans

async function orient(pc: PlanContext, ex: Extracted, res: PlanResult, share: number) {
  const focus = ex.paths[0];
  res.commands.push(`ctx map --budget ${Math.round(pc.budget * share)}${focus ? ` --focus ${focus}` : ""}`);
  const m = await pc.ctx.map(pc.budget * share, focus, { signal: pc.signal });
  if (m.code === 0 && m.stdout.trim()) {
    res.sections.push({ title: focus ? `Repository map (focus: ${focus})` : "Repository map", body: m.stdout.trim() });
  }
  res.hints.push("Use ctx_find to locate a symbol by name and ctx_source to read one symbol instead of whole files.");
}

async function locate(pc: PlanContext, ex: Extracted, res: PlanResult) {
  const targets = await resolveTargets(pc, ex, res);
  if (targets.length > 0) {
    const primary = targets[await pc.router.pickSymbol(pc.prompt, targets, pc.signal)];
    res.target = primary;
    await addSource(pc, primary, 0.55, res);
    await addExplain(pc, primary, 0.25, res);
    const others = targets.filter((t) => t !== primary);
    if (others.length) res.sections.push({ title: "Other matching symbols", body: others.map(fmtSym).join("\n") });
    res.hints.push(`Call ctx_source on any related symbol above to read it; ctx_impact("${symArg(primary)}") lists its callers.`);
    return;
  }
  await addSearch(pc, ex, res, 0.35);
  res.hints.push("Pick the most relevant candidate above and read it with ctx_source before answering.");
}

async function implement(pc: PlanContext, ex: Extracted, res: PlanResult) {
  const semantic = pc.meta.embeddings !== "no";
  res.commands.push(`ctx similar <request> --limit 5${semantic ? "" : " --keyword"}`);
  const sim = await pc.ctx.similar(pc.prompt.slice(0, 400), 5, semantic, { signal: pc.signal });
  if (sim.missingEmbeddings) pc.meta.embeddings = "no";
  const wantsTests = /\btests?\b/i.test(pc.prompt);
  // Test functions are rarely reusable implementations; keep them out unless asked.
  const relevant = sim.hits.filter((h) => h.score >= 0.5 && (wantsTests || !isTestFile(h.symbol.file)));
  if (relevant.length) {
    res.sections.push({
      title: "Existing code that may already do part of this (reuse or extend before writing new code)",
      body: relevant.map(fmtHit).join("\n"),
    });
  }

  let usedSmart = false;
  if (pc.meta.embeddings !== "no") {
    res.commands.push(`ctx smart <request> --max-tokens ${Math.round(pc.budget * 0.6)}`);
    const s = await pc.ctx.smart(pc.prompt.slice(0, 400), pc.budget * 0.6, { signal: pc.signal });
    if (s.missingEmbeddings) pc.meta.embeddings = "no";
    else if (s.text?.trim()) {
      pc.meta.embeddings = "yes";
      res.sections.push({ title: "Relevant files (ctx smart)", body: s.text.trim() });
      usedSmart = true;
    }
  }
  if (!usedSmart) {
    // Without embeddings, anchor on where the feature will live: files named
    // after words in the request (e.g. "duplicates" -> src/commands/duplicates.rs),
    // else the named symbols, else the best reuse candidate.
    const files = await matchFiles(pc, ex, wantsTests, res);
    if (files.length) {
      for (const f of files.slice(0, 2)) await addOutline(pc, f, ex, res);
      res.hints.push(`Put the change where it belongs: ${files[0]} is the most likely home.`);
    } else {
      const targets = (await resolveTargets(pc, ex, res)).filter((t) => wantsTests || !isTestFile(t.file));
      const anchor = targets[0] ?? relevant.find((h) => wantsTests || !isTestFile(h.symbol.file))?.symbol;
      if (anchor) await addSource(pc, anchor, 0.35, res);
    }
  }
  res.hints.push("Before writing a new helper, call ctx_similar with a one-line description of it.");
}

async function change(pc: PlanContext, ex: Extracted, res: PlanResult) {
  const targets = await resolveTargets(pc, ex, res);
  if (!targets.length) {
    await addSearch(pc, ex, res, 0.3);
    res.hints.push("Identify the symbol to change, then call ctx_impact on it before editing.");
    return;
  }
  const primary = targets[await pc.router.pickSymbol(pc.prompt, targets, pc.signal)];
  res.target = primary;
  await addSource(pc, primary, 0.35, res);

  const name = symArg(primary);
  res.commands.push(`ctx query callers ${name} --depth 2 --file ${primary.file}`);
  const { callers } = await pc.ctx.callers(name, 2, primary.file, { signal: pc.signal });
  if (callers.length) {
    res.sections.push({
      title: `Callers of ${name} (each must keep working after the change)`,
      body: fmtCallers(callers, 30),
    });
  }
  res.commands.push(`ctx query impact ${name}`);
  const impact = await pc.ctx.impact(name, { signal: pc.signal });
  if (impact.total > 0) {
    const files = [...new Set(impact.impacted.map((c) => c.symbol.file))];
    res.sections.push({
      title: "Blast radius",
      body: `${impact.total} symbols in ${files.length} files are transitively affected:\n${files.slice(0, 25).join("\n")}${files.length > 25 ? `\n… and ${files.length - 25} more` : ""}`,
    });
  } else if (!callers.length && primary.kind !== "function" && primary.kind !== "method") {
    await addExplain(pc, primary, 0.2, res);
  }
  res.hints.push(`Update every caller listed above; after editing, the extension re-checks architecture rules automatically.`);
}

async function debug(pc: PlanContext, ex: Extracted, res: PlanResult) {
  const targets = await resolveTargets(pc, ex, res);
  if (!targets.length) {
    await addSearch(pc, ex, res, 0.4);
    res.hints.push("Read the failing code with ctx_source, then follow what it calls.");
    return;
  }
  const primary = targets[await pc.router.pickSymbol(pc.prompt, targets, pc.signal)];
  res.target = primary;
  await addSource(pc, primary, 0.45, res);
  res.commands.push(`ctx query deps ${symArg(primary)} --file ${primary.file}`);
  const deps = await pc.ctx.deps(symArg(primary), primary.file, { signal: pc.signal });
  const inRepo = deps.filter((d) => d.resolved && d.distance === 1);
  if (inRepo.length) {
    res.sections.push({
      title: `In-repo code called by ${symArg(primary)} (likely places for the bug)`,
      body: dedupe(inRepo.map((d) => fmtSym(d.resolved!))).slice(0, 20).join("\n"),
    });
    // The first non-test callee is the most useful second read.
    const next = inRepo.map((d) => d.resolved!).find((s) => !/test/i.test(s.file) && s.file !== primary.file) ?? inRepo[0].resolved!;
    await addSource(pc, next, 0.25, res);
  }
  res.hints.push("Reproduce the failure first (run the test), then use ctx_source on the callees above to trace it.");
}

async function review(pc: PlanContext, res: PlanResult) {
  if (!pc.meta.git || !pc.reviewBase) {
    res.hints.push("This is not a git repository with a known base; review the files directly.");
    return;
  }
  res.commands.push(`ctx score --against ${pc.reviewBase} --json`);
  const s = await pc.ctx.score(pc.reviewBase, "check_violations>0", { signal: pc.signal });
  if (s.data) res.sections.push({ title: `Change scorecard vs ${pc.reviewBase}`, body: fmtScore(s.data) });
  res.commands.push(`ctx check --against ${pc.reviewBase} --json`);
  const c = await pc.ctx.check(pc.reviewBase, { signal: pc.signal });
  if (c.data?.violations.length) {
    res.sections.push({
      title: "Architecture rule violations introduced by the change",
      body: c.data.violations.slice(0, 20).map((v) => `- ${v.message}${v.reason ? ` (${v.reason})` : ""}`).join("\n"),
    });
  }
  res.hints.push("Use ctx_impact on changed public symbols to find consumers the diff does not show.");
}

async function health(pc: PlanContext, res: PlanResult) {
  res.commands.push("ctx hotspots --limit 10 --json");
  const hs = await pc.ctx.hotspots(10, { signal: pc.signal });
  if (hs.length) {
    res.sections.push({
      title: "Hotspots (churn × size; evidence to investigate, not proof of a problem)",
      body: hs.map((h) => `${h.file}  score ${h.score.toFixed(2)}  commits ${h.commits ?? "?"}  lines ${h.lines ?? "?"}`).join("\n"),
    });
  }
  res.commands.push("ctx check --json");
  const c = await pc.ctx.check(undefined, { signal: pc.signal });
  if (c.data) {
    res.sections.push({
      title: "Architecture rules",
      body: c.data.violations.length
        ? c.data.violations.slice(0, 15).map((v) => `- ${v.message}`).join("\n")
        : "No violations of .ctx/rules.toml (only meaningful if the rules cover the intended architecture).",
    });
  }
  res.hints.push("Never refactor only to lower a metric; confirm with ctx_source that complexity is accidental.");
}

/** Low-confidence plan: named targets if any, otherwise a small map. */
async function lightPlan(pc: PlanContext, ex: Extracted, res: PlanResult) {
  const targets = await resolveTargets(pc, ex, res);
  if (targets.length) {
    res.target = targets[0];
    await addExplain(pc, targets[0], 0.5, res);
    if (targets.length > 1) res.sections.push({ title: "Other matching symbols", body: targets.slice(1).map(fmtSym).join("\n") });
  } else {
    await orient(pc, ex, res, 0.5);
  }
  res.hints.push("Context is partial because the request was ambiguous; use ctx_find/ctx_source to look further.");
}

// ---------------------------------------------------------------------------
// Steps

/**
 * Map identifiers in the request to indexed symbols (exact name or qualified
 * name matches only), falling back to a keyword search over the remaining
 * terms. At most 6 symbols, deduplicated by location.
 */
async function resolveTargets(pc: PlanContext, ex: Extracted, res: PlanResult): Promise<SymbolRef[]> {
  const out: SymbolRef[] = [];
  for (const id of ex.identifiers.slice(0, 4)) {
    const leaf = leafName(id);
    res.commands.push(`ctx query find ${leaf}`);
    const found = await pc.ctx.find(leaf, { signal: pc.signal });
    const exact = found.filter(
      (s) =>
        s.name === leaf &&
        (id === leaf || (s.qualified_name ?? "").replace(/\./g, "::").endsWith(id.replace(/\.|#/g, "::"))),
    );
    out.push(...(exact.length ? exact : found.filter((s) => s.name === leaf)).slice(0, 3));
  }
  if (!out.length && ex.terms.length) {
    const q = ex.terms.slice(0, 5).join(" ");
    res.commands.push(`ctx search "${q}"`);
    const hits = await pc.ctx.search(q, 6, { signal: pc.signal });
    out.push(...hits.filter((h) => h.match_type === "exact" || h.score >= 0.9).map((h) => h.symbol).slice(0, 3));
  }
  return dedupeSyms(out).slice(0, 6);
}

/**
 * Indexed files whose basename matches a request word or path. Exact stem
 * matches rank first, then source over tests, then shallower paths.
 */
async function matchFiles(pc: PlanContext, ex: Extracted, wantsTests: boolean, res: PlanResult): Promise<string[]> {
  const words = new Set([...ex.terms, ...ex.identifiers.map((i) => leafName(i).toLowerCase())].filter((w) => w.length >= 4));
  if (!words.size && !ex.paths.length) return [];
  res.commands.push("ctx query files");
  const all = await pc.ctx.files({ signal: pc.signal });
  const scored: Array<{ f: string; score: number }> = [];
  for (const f of all) {
    if (ex.paths.some((p) => f === p || f.endsWith(`/${p}`))) {
      scored.push({ f, score: 100 });
      continue;
    }
    const stem = f.slice(f.lastIndexOf("/") + 1).replace(/\.[^.]+$/, "").toLowerCase();
    const parts = stem.split(/[_\-.]/);
    let score = 0;
    for (const w of words) {
      if (stem === w) score = Math.max(score, 10);
      else if (parts.includes(w)) score = Math.max(score, 5);
    }
    if (!score) continue;
    if (isTestFile(f)) score -= wantsTests ? 0 : 6;
    if (score <= 0) continue;
    scored.push({ f, score: score - f.split("/").length * 0.1 });
  }
  return scored.sort((a, b) => b.score - a.score).map((x) => x.f).slice(0, 3);
}

/** A file's outline plus the source of its most relevant public entry point. */
async function addOutline(pc: PlanContext, file: string, ex: Extracted, res: PlanResult) {
  res.commands.push(`ctx query find "" --file ${file}`);
  const syms = await pc.ctx.outline(file, 60, { signal: pc.signal });
  if (!syms.length) return;
  res.sections.push({
    title: `Outline of ${file}`,
    body: syms.map((s) => `- ${s.qualified_name || s.name} (${s.kind}${s.visibility === "public" ? ", pub" : ""}, line ${s.line_start})`).join("\n"),
  });
  const words = [...ex.terms, ...ex.identifiers.map((i) => leafName(i).toLowerCase())];
  const fns = syms.filter((s) => s.kind === "function" || s.kind === "method");
  const entry =
    fns.find((s) => s.visibility === "public" && words.some((w) => s.name.toLowerCase().includes(w))) ??
    fns.filter((s) => s.visibility === "public").sort((a, b) => b.line_end - b.line_start - (a.line_end - a.line_start))[0];
  if (entry) await addSource(pc, entry, 0.3, res);
}

function isTestFile(file: string): boolean {
  return /(^|\/)(tests?|__tests__|spec)\/|[._-](test|spec)\.[a-z]+$|_test\.go$|(^|\/)test_[^/]+\.py$/i.test(file);
}

async function addSource(pc: PlanContext, s: SymbolRef, share: number, res: PlanResult) {
  const name = symArg(s);
  res.commands.push(`ctx source ${name} --file ${s.file}`);
  const r = await pc.ctx.source(name, s.file, { signal: pc.signal });
  if (r.code === 0 && r.stdout.trim() && !/not found/i.test(r.stdout.slice(0, 200))) {
    res.sections.push({ title: `Source: ${name} (${s.file}:${s.line_start}-${s.line_end})`, body: fence(truncateToTokens(r.stdout.trim(), Math.round(pc.budget * share)), s.file) });
  }
}

async function addExplain(pc: PlanContext, s: SymbolRef, share: number, res: PlanResult) {
  const name = symArg(s);
  res.commands.push(`ctx explain ${name}`);
  const r = await pc.ctx.explain(name, { signal: pc.signal });
  if (r.code === 0 && r.stdout.trim()) {
    res.sections.push({ title: `Relationships: ${name}`, body: truncateToTokens(r.stdout.replace(/^Symbol:.*\n=+\n/, "").trim(), Math.round(pc.budget * share)) });
  }
}

async function addSearch(pc: PlanContext, ex: Extracted, res: PlanResult, share: number) {
  const q = [...ex.identifiers, ...ex.terms].slice(0, 6).join(" ") || pc.prompt.slice(0, 200);
  res.commands.push(`ctx search "${q}" --limit 8`);
  const hits = await pc.ctx.search(q, 8, { signal: pc.signal });
  if (!hits.length) return;
  res.sections.push({ title: `Candidate symbols for "${q}"`, body: hits.map(fmtHit).join("\n") });
  if (hits[0].score >= 0.8) await addSource(pc, hits[0].symbol, share, res);
}

// ---------------------------------------------------------------------------
// Rendering (plain text: small models follow it more reliably than JSON)

/** Assemble the model-facing message, enforcing the overall token budget. */
export function renderContext(plan: PlanResult, decision: RouteDecision, budget: number): string {
  const via =
    decision.source === "jev"
      ? `Jev, confidence ${decision.confidence.toFixed(2)}`
      : `keyword router${decision.fallbackReason ? `, Jev unavailable: ${decision.fallbackReason}` : ""}`;
  const head =
    `[ctx] Repository context for this request (workflow: ${plan.workflow}; routed by ${via}).\n` +
    "Selected from the ctx code index. Prefer it over reading whole files; verify against the files before editing.";
  const tail = plan.hints.length ? `\nNext steps:\n${plan.hints.map((h) => `- ${h}`).join("\n")}` : "";
  let remaining = budget - estimateTokens(head) - estimateTokens(tail);
  const parts: string[] = [head];
  for (const s of plan.sections) {
    if (remaining <= 60) break;
    const block = `\n## ${s.title}\n${s.body}`;
    const fitted = estimateTokens(block) <= remaining ? block : truncateToTokens(block, remaining);
    parts.push(fitted);
    remaining -= estimateTokens(fitted);
  }
  return parts.join("\n") + tail;
}

export function fmtSym(s: SymbolRef): string {
  return `${s.qualified_name || s.name} (${s.kind}, ${s.file}:${s.line_start})`;
}

function fmtHit(h: SearchHit): string {
  const brief = h.brief ? ` — ${h.brief.split("\n")[0].slice(0, 100)}` : "";
  const fanIn = h.fan_in ? `, ${h.fan_in} callers` : "";
  return `- ${fmtSym(h.symbol)}${fanIn}${brief}`;
}

function fmtCallers(callers: Caller[], max: number): string {
  const direct = callers.filter((c) => c.distance === 1);
  const indirect = callers.filter((c) => c.distance > 1);
  const lines = direct.slice(0, max).map((c) => `- ${fmtSym(c.symbol)}${c.line ? ` calls it at line ${c.line}` : ""}`);
  if (direct.length > max) lines.push(`… and ${direct.length - max} more direct callers`);
  if (indirect.length) lines.push(`(${indirect.length} indirect callers at depth 2)`);
  return lines.join("\n");
}

function fmtScore(s: { metrics: Record<string, number>; failed_conditions: string[]; files_changed: number; check_violations_note?: string | null }): string {
  const m = Object.entries(s.metrics).map(([k, v]) => `${k}=${v}`).join(", ");
  const failed = s.failed_conditions.length ? `\nFailed gate conditions: ${s.failed_conditions.join(", ")}` : "";
  const note = s.check_violations_note ? `\nNote: ${s.check_violations_note}` : "";
  return `${s.files_changed} files changed. ${m}${failed}${note}`;
}

function fence(code: string, file: string): string {
  const lang = (file.split(".").pop() ?? "").replace(/^(rs)$/, "rust").replace(/^(py)$/, "python");
  return "```" + lang + "\n" + code.replace(/^\/\/ Source: .*\n/, "") + "\n```";
}

/** Name to pass to ctx for a symbol (qualified when that disambiguates). */
export function symArg(s: SymbolRef): string {
  return s.qualified_name || s.name;
}

function dedupe(xs: string[]): string[] {
  return [...new Set(xs)];
}

function dedupeSyms(xs: SymbolRef[]): SymbolRef[] {
  const seen = new Set<string>();
  return xs.filter((s) => {
    const k = `${s.file}:${s.line_start}:${s.name}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
}
