import { test } from "node:test";
import assert from "node:assert/strict";
import { resolveConfig, DEFAULTS } from "../src/config.ts";
import { estimateTokens, injectionBudget, truncateToTokens, MIN_INJECT_TOKENS } from "../src/budget.ts";
import { extract, leafName } from "../src/symbols.ts";
import { keywordRoute } from "../src/router/keyword.ts";
import { Router, type SystemOneLike } from "../src/router/index.ts";
import { Ctx, versionAtLeast } from "../src/ctx-cli.ts";
import { Governance, violationKey } from "../src/governance.ts";
import { isRulesFile } from "../src/index.ts";
import { renderContext, runWorkflow, type PlanResult } from "../src/workflows.ts";
import type { Exec, ExecResult, RouteDecision } from "../src/types.ts";

// ---------------------------------------------------------------- config

test("config: defaults route with Jev, report gate, stable tools", () => {
  const c = resolveConfig(() => undefined, {});
  assert.equal(c.routing, "jev");
  assert.equal(c.gate, "report");
  assert.equal(c.tools, "stable");
  assert.equal(c.jevApiKey, undefined);
  assert.equal(c.failOn, DEFAULTS.failOn);
});

test("config: flag beats env, JEV_API_KEY beats TYPESAFE_API_KEY, CTX_GATE_BLOCKING maps to block", () => {
  const c = resolveConfig((n) => (n === "ctx-routing" ? "model" : undefined), {
    CTX_PI_ROUTING: "keyword",
    JEV_API_KEY: "jev",
    TYPESAFE_API_KEY: "ts",
    CTX_GATE_BLOCKING: "1",
  });
  assert.equal(c.routing, "model");
  assert.equal(c.jevApiKey, "jev");
  assert.equal(c.gate, "block");
  assert.equal(resolveConfig(() => undefined, { TYPESAFE_API_KEY: " ts " }).jevApiKey, "ts");
});

test("config: invalid values fall back, numbers are clamped", () => {
  const c = resolveConfig(() => "bogus", { CTX_PI_GATE_THRESHOLD: "7", CTX_PI_KEEP_CONTEXT: "x" });
  assert.equal(c.routing, "jev");
  assert.equal(c.gateThreshold, 1);
  assert.equal(c.keepContextMessages, DEFAULTS.keepContextMessages);
});

// ---------------------------------------------------------------- budget

test("budget: small windows get small briefs, never below the floor", () => {
  const small = injectionBudget(8192, 1000, 1, 6000);
  const large = injectionBudget(200_000, 1000, 1, 6000);
  assert.ok(small < 1500, `8K window budget ${small}`);
  assert.ok(small >= MIN_INJECT_TOKENS);
  assert.equal(large, 6000, "large windows hit the configured cap");
  assert.equal(injectionBudget(4096, 4000, 0, 6000), MIN_INJECT_TOKENS, "full window still gets the floor");
  assert.ok(injectionBudget(32_000, 0, 3, 50_000) > injectionBudget(32_000, 0, 0, 50_000), "scope scales budget");
});

test("budget: truncation cuts on a line and says so", () => {
  const text = Array.from({ length: 200 }, (_, i) => `line ${i} of some code`).join("\n");
  const out = truncateToTokens(text, 100);
  assert.ok(estimateTokens(out) <= 120);
  assert.match(out, /… \[truncated ~\d+ tokens\]$/);
  assert.equal(truncateToTokens("short", 100), "short");
});

// ---------------------------------------------------------------- extraction

test("extract: identifiers, qualified names, backticks and paths", () => {
  const ex = extract("Rename `Indexer::index_file` to reindexFile in src/index/mod.rs and update parse_config callers");
  assert.deepEqual(ex.paths, ["src/index/mod.rs"]);
  assert.ok(ex.identifiers.includes("Indexer::index_file"));
  assert.ok(ex.identifiers.includes("reindexFile"));
  assert.ok(ex.identifiers.includes("parse_config"));
  assert.ok(!ex.identifiers.includes("Rename"));
  assert.equal(leafName("Indexer::index_file"), "index_file");
  assert.equal(leafName("a.b.c"), "c");
});

test("extract: plain English yields terms, not identifiers", () => {
  const ex = extract("How does the scoring of duplicate pairs work?");
  assert.deepEqual(ex.identifiers, []);
  assert.ok(ex.terms.includes("scoring"));
  assert.ok(ex.terms.includes("duplicate"));
  assert.ok(!ex.terms.includes("the"));
});

// ---------------------------------------------------------------- keyword router

test("keyword router: representative prompts", () => {
  const cases: Array<[string, string]> = [
    ["Fix the failing test test_snapshot_roundtrip", "debug"],
    ["Rename Indexer to IndexBuilder everywhere", "change"],
    ["Add a --since option to the duplicates command", "implement"],
    ["Give me an overview of the repository architecture", "orient"],
    ["Where is parse_config defined?", "locate"],
    ["Review my changes before I merge", "review"],
    ["Which files are hotspots with high complexity?", "health"],
    ["Write a haiku about autumn", "none"],
  ];
  for (const [prompt, want] of cases) assert.equal(keywordRoute(prompt).workflow, want, prompt);
  const none = keywordRoute("Write a haiku about autumn");
  assert.ok(none.needsCodebase < 0.3);
  assert.equal(none.source, "keyword");
});

// ---------------------------------------------------------------- Jev router

function fakeJev(answers: Record<string, unknown> | Error): SystemOneLike & { calls: unknown[] } {
  const calls: unknown[] = [];
  return {
    calls,
    systemOne(req) {
      calls.push(req);
      return answers instanceof Error ? Promise.reject(answers) : Promise.resolve({ answers: answers as Record<string, any> });
    },
  };
}

const JEV_OK = {
  needs_codebase: { type: "noul", noul: 0.9 },
  about_repo: { type: "noul", noul: 0.7 },
  will_modify: { type: "noul", noul: 0.95 },
  workflow: { type: "choice", choice: "change", confidence: 0.91, probabilities: { change: 0.9, locate: 0.06, none: 0.03, debug: 0.01 } },
  scope: { type: "score", score: 1.4, confidence: 0.8 },
};

test("jev router: maps typed answers into a decision; sends no source code", async () => {
  const client = fakeJev(JEV_OK);
  const r = new Router("jev", client);
  const d = await r.route({ prompt: "rename foo_bar", meta: { files: 3, symbols: 10, functions: 8, embeddings: "no", git: true } });
  assert.equal(d.source, "jev");
  assert.equal(d.workflow, "change");
  assert.equal(d.confidence, 0.91);
  assert.ok(Math.abs(d.needsCodebase - 0.8) < 1e-9, "mean of the two gate nouls");
  assert.equal(d.willModify, 0.95);
  assert.deepEqual(d.alternatives, ["locate", "debug"], "alternatives exclude the choice and none");
  const sent = JSON.stringify(client.calls[0]);
  assert.match(sent, /"request":"rename foo_bar"/);
  assert.match(sent, /"files":3/);
  assert.doesNotMatch(sent, /embeddings|git/, "only whitelisted repository counts are sent");
});

test("jev router: any failure falls back to keyword; auth failure disables Jev", async () => {
  const net = new Router("jev", fakeJev(Object.assign(new Error("socket hang up"), { name: "APIConnectionError" })));
  const d1 = await net.route({ prompt: "Fix the failing test foo_test" });
  assert.equal(d1.source, "keyword");
  assert.equal(d1.workflow, "debug");
  assert.match(d1.fallbackReason ?? "", /socket hang up/);
  assert.ok(net.usesJev, "transient errors keep Jev enabled");

  const auth = new Router("jev", fakeJev(Object.assign(new Error("bad key"), { status: 401 })));
  await auth.route({ prompt: "x y z" });
  assert.equal(auth.usesJev, false);
  assert.match(auth.status(), /rejected/);

  const noKey = new Router("jev", undefined);
  assert.equal((await noKey.route({ prompt: "where is foo_bar" })).fallbackReason, "no JEV_API_KEY");
});

test("jev router: symbol disambiguation picks by index, degrades to first", async () => {
  const syms = [
    { name: "run", kind: "function", file: "a.rs", line_start: 1, line_end: 2 },
    { name: "run", kind: "method", qualified_name: "Job::run", file: "b.rs", line_start: 1, line_end: 2 },
  ];
  const r = new Router("jev", fakeJev({ target: { type: "choice", choice: "s1", confidence: 0.8, probabilities: {} } }));
  assert.equal(await r.pickSymbol("change Job::run", syms), 1);
  const broken = new Router("jev", fakeJev(new Error("down")));
  assert.equal(await broken.pickSymbol("change Job::run", syms), 0);
});

// ---------------------------------------------------------------- ctx wrapper + workflows (fake ctx)

type Handler = (args: string[]) => Partial<ExecResult> | undefined;
function fakeExec(handler: Handler): Exec & { log: string[][] } {
  const log: string[][] = [];
  const fn = (async (_cmd: string, args: string[]) => {
    log.push(args);
    const r = handler(args) ?? { code: 2, stderr: "Error: unknown" };
    return { stdout: "", stderr: "", code: 0, killed: false, ...r };
  }) as Exec & { log: string[][] };
  fn.log = log;
  return fn;
}
const env = (data: unknown) => JSON.stringify({ command: "x", ctx_version: "0.4.0", data });
const sym = (name: string, file: string, kind = "function") => ({ name, qualified_name: null, kind, file, line_start: 10, line_end: 20 });

test("ctx wrapper: exit 1 payloads are data, exit 2 is an error, spawn failures don't throw", async () => {
  const exec = fakeExec((a) =>
    a[0] === "check" ? { code: 1, stdout: env({ violations: [{ rule: "forbidden", rule_id: "r", message: "m" }], summary: { violations: 1 } }) } : { code: 2, stderr: "Error: no index\n" },
  );
  const ctx = new Ctx(exec, "/repo");
  const c = await ctx.check("HEAD");
  assert.equal(c.data?.violations.length, 1);
  const s = await ctx.score("HEAD", "x>0");
  assert.equal(s.error, "no index");
  const broken = new Ctx(() => Promise.reject(new Error("ENOENT")), "/repo");
  assert.equal((await broken.run(["--version"])).code, 127);
  assert.equal((await broken.available()).ok, false);
});

test("ctx wrapper: semantic similar falls back to keyword when embeddings are missing", async () => {
  const exec = fakeExec((a) => {
    if (a[0] !== "similar") return undefined;
    return a.includes("--keyword") ? { stdout: env({ results: [{ symbol: sym("f", "a.rs"), score: 0.9 }] }) } : { code: 2, stderr: "Error: Embedding error: No embeddings found. Run 'ctx embed' first" };
  });
  const r = await new Ctx(exec, "/").similar("q", 5, true);
  assert.equal(r.missingEmbeddings, true);
  assert.equal(r.hits.length, 1);
});

const decision = (w: RouteDecision["workflow"], confidence = 0.95): RouteDecision => ({
  workflow: w,
  confidence,
  needsCodebase: 0.9,
  willModify: 0.9,
  scope: 1,
  alternatives: ["locate"],
  source: "jev",
  latencyMs: 1,
});

test("change workflow: resolves the named symbol, shows source, callers and blast radius", async () => {
  const exec = fakeExec((a) => {
    const [c0, c1] = a;
    if (c0 === "query" && c1 === "find") return { stdout: env({ symbols: a.at(-1) === "place_order" ? [sym("place_order", "src/order.rs"), sym("place_order_v2", "src/order.rs")] : [] }) };
    if (c0 === "source") return { stdout: "// Source: src/order.rs::place_order@10\nfn place_order() {}\n" };
    if (c0 === "query" && c1 === "callers") return { stdout: env({ callers: [{ symbol: sym("checkout", "src/app.rs"), distance: 1, line: 5 }] }) };
    if (c0 === "query" && c1 === "impact") return { stdout: env({ total: 2, impacted: [{ symbol: sym("checkout", "src/app.rs"), distance: 1 }, { symbol: sym("main", "src/main.rs"), distance: 2 }] }) };
    return undefined;
  });
  const ctx = new Ctx(exec, "/repo");
  const plan = await runWorkflow({
    ctx,
    router: new Router("keyword", undefined),
    prompt: "Change place_order to take a currency",
    decision: decision("change"),
    budget: 1000,
    meta: { files: 1, symbols: 1, functions: 1, embeddings: "unknown", git: true },
  });
  assert.equal(plan.target?.name, "place_order", "exact-name match only; place_order_v2 is not a target");
  const titles = plan.sections.map((s) => s.title);
  assert.ok(titles.some((t) => t.startsWith("Source: place_order")));
  assert.ok(titles.some((t) => t.startsWith("Callers of place_order")));
  assert.ok(titles.includes("Blast radius"));
  assert.ok(plan.tools.includes("ctx_impact"));
  const text = renderContext(plan, decision("change"), 1000);
  assert.match(text, /checkout \(function, src\/app\.rs:10\) calls it at line 5/);
  assert.match(text, /2 symbols in 2 files/);
  assert.doesNotMatch(text, /\/\/ Source:/, "ctx's source banner is stripped");
});

test("none workflow runs no ctx commands; low confidence runs the light plan", async () => {
  const exec = fakeExec((a) => (a[0] === "map" ? { stdout: "src/\n  lib.rs" } : { stdout: env({ symbols: [], results: [] }) }));
  const base = { ctx: new Ctx(exec, "/"), router: new Router("keyword", undefined), budget: 800, meta: { files: 1, symbols: 1, functions: 1, embeddings: "no" as const, git: false } };
  const none = await runWorkflow({ ...base, prompt: "haiku", decision: decision("none") });
  assert.equal(none.sections.length, 0);
  assert.equal(exec.log.length, 0);
  const light = await runWorkflow({ ...base, prompt: "make it better", decision: decision("implement", 0.2) }, true);
  assert.equal(light.sections[0]?.title, "Repository map");
  assert.ok(light.tools.includes("ctx_find"), "alternatives' tools are offered");
});

test("renderContext enforces the budget across sections", () => {
  const plan: PlanResult = {
    workflow: "locate",
    sections: [
      { title: "A", body: "x".repeat(4000) },
      { title: "B", body: "y".repeat(4000) },
    ],
    tools: [],
    hints: ["do the thing"],
    commands: [],
  };
  const out = renderContext(plan, decision("locate"), 600);
  assert.ok(estimateTokens(out) <= 680, `rendered ${estimateTokens(out)} tokens`);
  assert.match(out, /Next steps:\n- do the thing$/);
  assert.match(out, /routed by Jev, confidence 0\.95/);
});

// ---------------------------------------------------------------- governance

test("governance: reports only new violations; block continues once per run", async () => {
  let violations = [{ rule: "forbidden", rule_id: "r1", message: "a.ts -> db.ts", reason: "layering" }];
  let failed: string[] = ["check_violations>0"];
  const exec = fakeExec((a) => {
    if (a[0] === "index") return { code: 0 };
    if (a[0] === "check") return { code: violations.length ? 1 : 0, stdout: env({ violations, summary: { violations: violations.length } }) };
    if (a[0] === "score") return { code: failed.length ? 1 : 0, stdout: env({ against: "abc", files_changed: 1, metrics: { check_violations: violations.length, files_changed: 1 }, failed_conditions: failed }) };
    return undefined;
  });
  const g = new Governance(new Ctx(exec, "/"), "block", "check_violations>0", 1, "abc");
  g.startRun();
  const n1 = await g.afterEdit();
  assert.match(n1 ?? "", /introduced 1 architecture rule violation.*\n- a\.ts -> db\.ts \(layering\)/);
  assert.equal(await g.afterEdit(), undefined, "same violation is not repeated");
  const s1 = await g.atSettle();
  assert.equal(s1?.continue, true);
  assert.match(s1?.text ?? "", /FAILED/);
  await g.afterEdit();
  const s2 = await g.atSettle();
  assert.equal(s2?.continue, false, "continuation budget is spent");
  assert.match(s2?.text ?? "", /still FAILS/);
  assert.equal(await g.atSettle(), undefined, "clean runs without edits report nothing");

  violations = [];
  failed = [];
  g.startRun();
  await g.afterEdit();
  const s3 = await g.atSettle();
  assert.equal(s3?.failed, false);
  assert.match(s3?.text ?? "", /passed/);
});

test("governance: report mode never continues; operational errors are not a pass", async () => {
  const exec = fakeExec((a) => (a[0] === "index" ? { code: 0 } : a[0] === "check" ? { stdout: env({ violations: [], summary: { violations: 0 } }) } : { code: 2, stderr: "Error: bad ref" }));
  const g = new Governance(new Ctx(exec, "/"), "report", "check_violations>0", 3, "abc");
  await g.afterEdit();
  const s = await g.atSettle();
  assert.equal(s?.continue, false);
  assert.match(s?.text ?? "", /could not evaluate \(bad ref\); treat the gate as NOT passed/);
  const off = new Governance(new Ctx(exec, "/"), "off", "x", 1, "abc");
  assert.equal(await off.afterEdit(), undefined);
  assert.equal(await off.atSettle(), undefined);
});

// ---------------------------------------------------------------- regressions from review

test("ctx wrapper: user text is always passed after `--`, so leading dashes are not flags", async () => {
  const exec = fakeExec((a) => ({ stdout: env({ results: [], symbols: [], callers: [], dependencies: [], total: 0, impacted: [] }) }));
  const ctx = new Ctx(exec, "/");
  await ctx.search("--limit is odd", 5);
  await ctx.similar("- add a flag", 5, false);
  await ctx.smart("- add a flag", 1000);
  await ctx.find("-x");
  await ctx.source("-x", "a.rs");
  await ctx.callers("-x", 1, "a.rs");
  await ctx.deps("-x", "a.rs");
  await ctx.impact("-x");
  await ctx.outline("src/a.rs");
  for (const args of exec.log) {
    const sep = args.indexOf("--");
    assert.ok(sep > 0, `no -- in ${args.join(" ")}`);
    assert.equal(args.length - sep, 2, `exactly one positional after --: ${args.join(" ")}`);
    if (args.includes("--json")) assert.ok(args.indexOf("--json") < sep, "--json stays a flag");
  }
});

test("ctx wrapper: a killed (timed out) process is an error, not a success", async () => {
  const ctx = new Ctx(async () => ({ stdout: "partial", stderr: "", code: 0, killed: true }), "/");
  assert.equal((await ctx.run(["index"])).code, 124);
  assert.equal((await ctx.smart("t", 1000)).text, undefined);
});

test("ctx wrapper: callers surfaces ambiguity instead of an empty list", async () => {
  const exec = fakeExec(() => ({ stdout: env({ callers: [], ambiguous: [sym("new", "a.rs"), sym("new", "b.rs")] }) }));
  const r = await new Ctx(exec, "/").callers("new", 1);
  assert.equal(r.callers.length, 0);
  assert.equal(r.ambiguous.length, 2);
});

test("versionAtLeast compares numerically", () => {
  assert.ok(versionAtLeast("0.4.0", "0.4.0"));
  assert.ok(versionAtLeast("0.10.0", "0.4.0"));
  assert.ok(versionAtLeast("ctx 1.0.0-rc.1", "0.4.0"));
  assert.ok(!versionAtLeast("0.3.9", "0.4.0"));
  assert.ok(!versionAtLeast("unknown", "0.4.0"));
});

test("violationKey ignores line shifts but not different endpoints", () => {
  const v = (line: number, to = "src/infra/db.ts") => ({
    rule: "forbidden",
    rule_id: "forbidden: domain -> infrastructure",
    message: `src/domain/order.ts:${line} -> ${to} [calls query]`,
    from: { name: "placeOrder", file: "src/domain/order.ts", line_start: line },
    to: { name: "query", file: to },
  });
  assert.equal(violationKey(v(2)), violationKey(v(9)));
  assert.notEqual(violationKey(v(2)), violationKey(v(2, "src/infra/cache.ts")));
  const bare = (line: number) => ({ rule: "limit", rule_id: "limit", message: `src/a.ts:${line} (f): fan_in 30 exceeds max 25` });
  assert.equal(violationKey(bare(10)), violationKey(bare(12)));
});

test("rules guard resolves paths like pi's file tools", () => {
  const cwd = "/repo";
  for (const p of [".ctx/rules.toml", "@.ctx/rules.toml", ".ctx//rules.toml", "./.ctx/./rules.toml", "/repo/.ctx/rules.toml", "src/../.ctx/rules.toml"]) {
    assert.ok(isRulesFile(p, cwd), p);
  }
  for (const p of [".ctx/rules.toml.bak", "other/.ctx/rules.toml", ".ctx/config.toml"]) assert.ok(!isRulesFile(p, cwd), p);
});

test("router: circuit breaker pauses Jev after repeated network failures, then retries", async () => {
  let t = 0;
  const client = fakeJev(Object.assign(new Error("timeout"), { name: "APITimeoutError" }));
  const r = new Router("jev", client, () => t);
  await r.route({ prompt: "where is foo_bar" });
  assert.ok(r.usesJev, "one failure is tolerated");
  await r.route({ prompt: "where is foo_bar" });
  assert.equal(r.usesJev, false, "second consecutive failure opens the breaker");
  const calls = client.calls.length;
  const d = await r.route({ prompt: "where is foo_bar" });
  assert.equal(client.calls.length, calls, "no Jev call while paused");
  assert.equal(d.source, "keyword");
  t += Router.BREAKER_PAUSE_MS;
  assert.ok(r.usesJev, "Jev is retried after the pause");
});

test("governance: bash edits arm the end-of-run gate; a clean gate is not an error", async () => {
  const exec = fakeExec((a) =>
    a[0] === "score" ? { stdout: env({ against: "abc", files_changed: 1, metrics: { check_violations: 0 }, failed_conditions: [] }) } : { code: 0 },
  );
  const g = new Governance(new Ctx(exec, "/"), "report", "check_violations>0", 1, "abc");
  g.markDirty();
  const r = await g.atSettle();
  assert.equal(r?.failed, false);
  assert.equal(r?.error, false);
});
