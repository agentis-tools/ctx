/**
 * With/without evaluation of the ctx pi extension on a local (or any) model.
 *
 *   node eval/run.ts --model ollama/qwen2.5-coder:7b [--modes baseline,jev,keyword,model]
 *                    [--runs 3] [--tasks eval/tasks.json] [--only id1,id2] [--timeout 600] [--full]
 *
 * For every (mode, task, run) it copies a prepared checkout of the pinned
 * repository, runs `pi --mode json` non-interactively, and grades the result.
 *
 * Modes:
 *   baseline  pi with no extensions (built-in read/grep/edit/bash only)
 *   jev       this extension, Jev routing (needs JEV_API_KEY)
 *   keyword   this extension, offline keyword routing
 *   model     this extension, model-driven tool use (no pre-injected brief)
 *
 * Output: eval/results/<timestamp>/results.jsonl and summary.md.
 */
import { spawn, execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

interface Task {
  id: string;
  kind: string;
  prompt: string;
  expect?: string[];
  verify?: string;
  slow_verify?: string;
}
interface TaskFile {
  repo: string;
  commit: string;
  tasks: Task[];
}
type Mode = "baseline" | "jev" | "keyword" | "model";

const PKG = resolve(import.meta.dirname, "..");
const args = parseArgs(process.argv.slice(2));
const model = args.model;
if (!model) {
  console.error("usage: node eval/run.ts --model <provider/model> [--modes baseline,jev,keyword,model] [--runs N]");
  process.exit(2);
}
const modes = (args.modes ?? "baseline,jev,keyword").split(",") as Mode[];
const runs = Number(args.runs ?? 1);
const timeoutS = Number(args.timeout ?? 600);
const full = "full" in args;
const taskFile = JSON.parse(readFileSync(resolve(args.tasks ?? join(PKG, "eval", "tasks.json")), "utf8")) as TaskFile;
const only = args.only?.split(",");
const tasks = taskFile.tasks.filter((t) => !only || only.includes(t.id));
const pi = args.pi ?? (existsSync(join(PKG, "node_modules", ".bin", "pi")) ? join(PKG, "node_modules", ".bin", "pi") : "pi");
const ctxBin = process.env.CTX_BIN ?? "ctx";

if (modes.includes("jev") && !process.env.JEV_API_KEY && !process.env.TYPESAFE_API_KEY) {
  console.error("mode 'jev' needs JEV_API_KEY (or TYPESAFE_API_KEY); drop it from --modes or set the key.");
  process.exit(2);
}

// ---------------------------------------------------------------- prepare the golden checkout once
const cache = resolve(args.cache ?? join(PKG, "eval", ".cache"));
const golden = join(cache, `repo-${taskFile.commit.slice(0, 12)}`);
if (!existsSync(join(golden, ".git"))) {
  mkdirSync(cache, { recursive: true });
  console.error(`cloning ${taskFile.repo} @ ${taskFile.commit.slice(0, 12)} …`);
  execFileSync("git", ["clone", "--quiet", taskFile.repo, golden], { stdio: "inherit" });
  execFileSync("git", ["-C", golden, "checkout", "--quiet", taskFile.commit], { stdio: "inherit" });
}
console.error("indexing golden checkout …");
execFileSync(ctxBin, ["index"], { cwd: golden, stdio: "ignore" });

const outDir = join(PKG, "eval", "results", new Date().toISOString().replace(/[:.]/g, "-"));
mkdirSync(outDir, { recursive: true });
const resultsPath = join(outDir, "results.jsonl");

interface Result {
  mode: Mode;
  task: string;
  kind: string;
  run: number;
  pass: boolean;
  seconds: number;
  inputTokens: number;
  outputTokens: number;
  turns: number;
  toolCalls: Record<string, number>;
  routed?: string;
  answer: string;
  error?: string;
}

const results: Result[] = [];
for (const task of tasks) {
  for (const mode of modes) {
    for (let run = 1; run <= runs; run++) {
      const r = await runOne(task, mode, run);
      results.push(r);
      writeFileSync(resultsPath, JSON.stringify(r) + "\n", { flag: "a" });
      console.error(`${r.pass ? "PASS" : "FAIL"} ${mode.padEnd(8)} ${task.id} #${run}  ${r.seconds.toFixed(0)}s  in=${r.inputTokens} out=${r.outputTokens} tools=${sum(r.toolCalls)}${r.routed ? ` route=${r.routed}` : ""}${r.error ? `  (${r.error})` : ""}`);
    }
  }
}
writeFileSync(join(outDir, "summary.md"), summarize(results));
console.log(readFileSync(join(outDir, "summary.md"), "utf8"));
console.error(`\nwritten: ${outDir}`);

// ---------------------------------------------------------------- one run

async function runOne(task: Task, mode: Mode, run: number): Promise<Result> {
  const work = mkdtempSync(join(tmpdir(), `pi-ctx-eval-${task.id}-`));
  cpSync(golden, work, { recursive: true });
  const piArgs = ["--mode", "json", "--no-session", "--model", model!];
  if (mode === "baseline") piArgs.push("--no-extensions");
  else piArgs.push("--no-extensions", "-e", join(PKG, "src", "index.ts"));
  piArgs.push(task.prompt);

  const env: Record<string, string | undefined> = { ...process.env, CTX_BIN: ctxBin, CTX_PI_GATE: "report" };
  if (mode !== "baseline") env.CTX_PI_ROUTING = mode;

  const started = Date.now();
  const { stdout, code, killed } = await runProcess(pi, piArgs, work, env, timeoutS * 1000);
  const seconds = (Date.now() - started) / 1000;
  const parsed = parseEvents(stdout);

  let pass = false;
  let error: string | undefined = killed ? "timeout" : code !== 0 ? `pi exited ${code}` : undefined;
  if (task.expect) pass = task.expect.every((re) => new RegExp(re, "i").test(parsed.answer));
  if (task.verify) {
    pass = shell(task.verify, work);
    if (pass && full && task.slow_verify) pass = shell(task.slow_verify, work);
  }
  if (!pass && !error && !parsed.answer) error = "no answer";
  rmSync(work, { recursive: true, force: true });
  return { mode, task: task.id, kind: task.kind, run, pass, seconds, ...parsed, error };
}

function parseEvents(jsonl: string): Pick<Result, "inputTokens" | "outputTokens" | "turns" | "toolCalls" | "routed" | "answer"> {
  let inputTokens = 0;
  let outputTokens = 0;
  let turns = 0;
  let answer = "";
  let routed: string | undefined;
  const toolCalls: Record<string, number> = {};
  for (const line of jsonl.split("\n")) {
    if (!line.startsWith("{")) continue;
    let e: any;
    try {
      e = JSON.parse(line);
    } catch {
      continue;
    }
    if (e.type === "message_end" && e.message?.role === "assistant") {
      turns++;
      inputTokens += e.message.usage?.input ?? 0;
      outputTokens += e.message.usage?.output ?? 0;
      const text = (e.message.content ?? []).filter((c: any) => c.type === "text").map((c: any) => c.text).join("");
      if (text.trim()) answer = text;
    }
    if (e.type === "tool_execution_start") toolCalls[e.toolName] = (toolCalls[e.toolName] ?? 0) + 1;
    if (e.type === "message_end" && e.message?.role === "custom" && e.message.customType === "ctx-context") {
      routed = /workflow: (\w+)/.exec(typeof e.message.content === "string" ? e.message.content : "")?.[1];
    }
  }
  return { inputTokens, outputTokens, turns, toolCalls, routed, answer };
}

// ---------------------------------------------------------------- report

function summarize(rs: Result[]): string {
  const byMode = group(rs, (r) => r.mode);
  const lines = [
    `# pi-ctx eval — ${model}`,
    "",
    `Repository ${taskFile.repo} @ ${taskFile.commit.slice(0, 12)}; ${tasks.length} tasks × ${runs} run(s); ${new Date().toISOString()}`,
    "",
    "| mode | pass rate | mean input tok | mean output tok | mean tool calls | mean seconds |",
    "|---|---|---|---|---|---|",
  ];
  for (const [mode, xs] of byMode) {
    lines.push(
      `| ${mode} | ${pct(xs.filter((x) => x.pass).length, xs.length)} | ${mean(xs.map((x) => x.inputTokens))} | ${mean(xs.map((x) => x.outputTokens))} | ${mean(xs.map((x) => sum(x.toolCalls)), 1)} | ${mean(xs.map((x) => x.seconds), 1)} |`,
    );
  }
  lines.push("", "## Per task (passes / runs)", "", `| task | ${[...byMode.keys()].join(" | ")} |`, `|---|${[...byMode.keys()].map(() => "---").join("|")}|`);
  for (const [task, xs] of group(rs, (r) => r.task)) {
    const cells = [...byMode.keys()].map((m) => {
      const ys = xs.filter((x) => x.mode === m);
      return `${ys.filter((y) => y.pass).length}/${ys.length}`;
    });
    lines.push(`| ${task} | ${cells.join(" | ")} |`);
  }
  lines.push("", "Input tokens include the injected brief. Pass rate is the north-star metric; tokens and tool calls show cost.");
  return lines.join("\n") + "\n";
}

// ---------------------------------------------------------------- helpers

function runProcess(cmd: string, argv: string[], cwd: string, env: Record<string, string | undefined>, timeoutMs: number) {
  return new Promise<{ stdout: string; code: number; killed: boolean }>((res) => {
    const p = spawn(cmd, argv, { cwd, env: env as NodeJS.ProcessEnv, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let killed = false;
    p.stdout!.on("data", (d) => (stdout += d));
    p.stderr!.on("data", () => undefined);
    const t = setTimeout(() => {
      killed = true;
      p.kill("SIGTERM");
    }, timeoutMs);
    p.on("close", (code) => {
      clearTimeout(t);
      res({ stdout, code: code ?? -1, killed });
    });
  });
}

function shell(script: string, cwd: string): boolean {
  try {
    execFileSync("bash", ["-c", script], { cwd, stdio: "ignore", timeout: 900_000 });
    return true;
  } catch {
    return false;
  }
}

function parseArgs(argv: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) out[key] = "true";
    else out[key] = argv[++i];
  }
  return out;
}

function group<T>(xs: T[], key: (x: T) => string): Map<string, T[]> {
  const m = new Map<string, T[]>();
  for (const x of xs) m.set(key(x), [...(m.get(key(x)) ?? []), x]);
  return m;
}
function sum(o: Record<string, number>): number {
  return Object.values(o).reduce((a, b) => a + b, 0);
}
function mean(xs: number[], digits = 0): string {
  return xs.length ? (xs.reduce((a, b) => a + b, 0) / xs.length).toFixed(digits) : "-";
}
function pct(n: number, d: number): string {
  return d ? `${Math.round((100 * n) / d)}% (${n}/${d})` : "-";
}
