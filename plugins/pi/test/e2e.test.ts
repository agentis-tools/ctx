/**
 * End-to-end: real pi + real ctx + a scripted OpenAI-compatible model.
 *
 * A throwaway repository has a `domain -> infrastructure` forbidden rule. The
 * scripted "model" makes one edit that violates it, then says it is done. We
 * assert on what pi actually sent to the model:
 *   1. the first request carries the ctx brief and the ctx tools,
 *   2. the edit's tool result carries the `[ctx check]` violation note,
 *   3. in block mode the end-of-run gate triggers exactly one more turn.
 *
 * Skipped when the ctx binary or the pi CLI is not available. Routing uses the
 * keyword router so the test is deterministic and needs no network.
 */
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, existsSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import assert from "node:assert/strict";

const PKG = resolve(import.meta.dirname, "..");
const PI = join(PKG, "node_modules", ".bin", "pi");
const CTX = process.env.CTX_BIN ?? "ctx";

function has(cmd: string, args: string[]): boolean {
  try {
    execFileSync(cmd, args, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

const skip = !existsSync(PI) ? "pi CLI not installed (npm install)" : !has(CTX, ["--version"]) ? "ctx binary not found (set CTX_BIN)" : false;

type Req = { messages: Array<{ role: string; content: unknown; tool_call_id?: string }>; tools?: Array<{ function: { name: string } }> };
type Step = { text?: string; tool?: { name: string; args: Record<string, unknown> } };

/** Minimal streaming chat-completions server that replays `script`. */
async function fakeModel(script: Step[]) {
  const requests: Req[] = [];
  const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    if (!req.url?.endsWith("/chat/completions")) {
      res.writeHead(404).end();
      return;
    }
    const parsed = JSON.parse(body) as Req;
    requests.push(parsed);
    const step = script[requests.length - 1] ?? { text: "Done." };
    res.writeHead(200, { "content-type": "text/event-stream" });
    const send = (o: unknown) => res.write(`data: ${JSON.stringify(o)}\n\n`);
    const base = { id: `c${requests.length}`, object: "chat.completion.chunk", created: 0, model: "fake-small" };
    if (step.tool) {
      send({ ...base, choices: [{ index: 0, delta: { role: "assistant", tool_calls: [{ index: 0, id: `call_${requests.length}`, type: "function", function: { name: step.tool.name, arguments: JSON.stringify(step.tool.args) } }] }, finish_reason: null }] });
      send({ ...base, choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] });
    } else {
      send({ ...base, choices: [{ index: 0, delta: { role: "assistant", content: step.text ?? "" }, finish_reason: null }] });
      send({ ...base, choices: [{ index: 0, delta: {}, finish_reason: "stop" }] });
    }
    send({ ...base, choices: [], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } });
    res.end("data: [DONE]\n\n");
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const port = (server.address() as { port: number }).port;
  return { requests, url: `http://127.0.0.1:${port}/v1`, close: () => server.close() };
}

function fixtureRepo(): string {
  const dir = mkdtempSync(join(tmpdir(), "pi-ctx-e2e-"));
  const w = (p: string, s: string) => {
    mkdirSync(join(dir, p, ".."), { recursive: true });
    writeFileSync(join(dir, p), s);
  };
  w("src/domain/order.ts", "export function placeOrder(total: number): number {\n  return total * 2;\n}\n");
  w("src/infra/db.ts", "export function query(sql: string): number {\n  return sql.length;\n}\n");
  w("src/app/service.ts", 'import { placeOrder } from "../domain/order";\n\nexport function checkout(): number {\n  return placeOrder(3);\n}\n');
  w(
    ".ctx/rules.toml",
    'version = 1\n\n[layers]\ndomain = ["src/domain/**"]\ninfrastructure = ["src/infra/**"]\n\n[[rules.forbidden]]\nfrom = "domain"\nto = "infrastructure"\nreason = "Domain must stay persistence-agnostic"\n',
  );
  w(".gitignore", ".ctx/*.db*\n.ctx/cache/\n");
  const git = (...a: string[]) => execFileSync("git", a, { cwd: dir, stdio: "ignore" });
  git("init", "-q", "-b", "main");
  git("-c", "user.email=t@t", "-c", "user.name=t", "add", "-A");
  git("-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init");
  return dir;
}

async function runPi(cwd: string, modelUrl: string, prompt: string, env: Record<string, string>) {
  const agentDir = mkdtempSync(join(tmpdir(), "pi-agent-"));
  writeFileSync(
    join(agentDir, "models.json"),
    JSON.stringify({
      providers: {
        fake: { baseUrl: modelUrl, api: "openai-completions", apiKey: "x", models: [{ id: "fake-small", contextWindow: 8192, maxTokens: 1024 }] },
      },
    }),
  );
  const args = ["--mode", "json", "--no-session", "--no-extensions", "-e", join(PKG, "src", "index.ts"), "--model", "fake/fake-small", prompt];
  return await new Promise<{ code: number; stdout: string; stderr: string }>((res) => {
    const p = spawn(PI, args, {
      cwd,
      // pi reads piped stdin as extra prompt input; give it none.
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, PI_CODING_AGENT_DIR: agentDir, PI_OFFLINE: "1", CTX_BIN: CTX, JEV_API_KEY: "", TYPESAFE_API_KEY: "", ...env },
    });
    let stdout = "";
    let stderr = "";
    p.stdout!.on("data", (d) => (stdout += d));
    p.stderr!.on("data", (d) => (stderr += d));
    p.on("close", (code) => res({ code: code ?? -1, stdout, stderr }));
  });
}

const PROMPT = "Change placeOrder in src/domain/order.ts so it records the order by calling query from src/infra/db.ts";
const VIOLATING_EDIT: Step = {
  tool: {
    name: "write",
    args: {
      path: "src/domain/order.ts",
      content: 'import { query } from "../infra/db";\n\nexport function placeOrder(total: number): number {\n  query("insert order");\n  return total * 2;\n}\n',
    },
  },
};

const text = (c: unknown) => (typeof c === "string" ? c : JSON.stringify(c));

test("ctx brief, tools, post-edit check and report gate reach the model", { skip, timeout: 120_000 }, async () => {
  const repo = fixtureRepo();
  const model = await fakeModel([VIOLATING_EDIT, { text: "Done." }]);
  try {
    const r = await runPi(repo, model.url, PROMPT, { CTX_PI_ROUTING: "keyword", CTX_PI_GATE: "report" });
    assert.equal(r.code, 0, r.stderr);
    assert.ok(model.requests.length >= 2, `expected >=2 model requests, got ${model.requests.length}\n${r.stderr}`);

    const first = model.requests[0];
    const all = first.messages.map((m) => text(m.content)).join("\n");
    assert.match(all, /\[ctx\] Repository context for this request \(workflow: change/);
    assert.match(all, /placeOrder/);
    assert.match(all, /checkout/, "brief lists the caller of placeOrder");
    const tools = (first.tools ?? []).map((t) => t.function.name);
    for (const t of ["ctx_find", "ctx_source", "ctx_impact", "ctx_similar", "ctx_check"]) assert.ok(tools.includes(t), `tool ${t} missing: ${tools}`);

    const second = model.requests[1];
    const toolResult = second.messages.filter((m) => m.role === "tool").map((m) => text(m.content)).join("\n");
    assert.match(toolResult, /\[ctx check\] This edit introduced 2 architecture rule violation\(s\):[\s\S]*\[calls query\][\s\S]*\[import\]/);
    assert.match(toolResult, /Domain must stay persistence-agnostic/);

    // Report mode: no extra turn, but the gate result is in the session output.
    assert.equal(model.requests.length, 2);
    assert.match(r.stdout, /\[ctx gate\]/);
  } finally {
    model.close();
  }
});

test("block mode asks the model for exactly one more turn", { skip, timeout: 120_000 }, async () => {
  const repo = fixtureRepo();
  const model = await fakeModel([VIOLATING_EDIT, { text: "Done." }, { text: "I could not fix it." }]);
  try {
    const r = await runPi(repo, model.url, PROMPT, { CTX_PI_ROUTING: "keyword", CTX_PI_GATE: "block" });
    assert.equal(r.code, 0, r.stderr);
    assert.equal(model.requests.length, 3, `expected 3 requests, got ${model.requests.length}\n${r.stderr}`);
    const third = model.requests[2].messages.map((m) => text(m.content)).join("\n");
    assert.match(third, /\[ctx gate\] The quality gate FAILED/);
    assert.match(third, /check_violations/);
  } finally {
    model.close();
  }
});

test("non-code prompt injects nothing", { skip, timeout: 120_000 }, async () => {
  const repo = fixtureRepo();
  const model = await fakeModel([{ text: "Canberra." }]);
  try {
    const r = await runPi(repo, model.url, "What is the capital of Australia?", { CTX_PI_ROUTING: "keyword" });
    assert.equal(r.code, 0, r.stderr);
    const all = model.requests[0].messages.map((m) => text(m.content)).join("\n");
    assert.doesNotMatch(all, /\[ctx\] Repository context/);
  } finally {
    model.close();
  }
});

test("the agent cannot rewrite .ctx/rules.toml to pass the gate", { skip, timeout: 120_000 }, async () => {
  const repo = fixtureRepo();
  const model = await fakeModel([{ tool: { name: "write", args: { path: "@.ctx/rules.toml", content: "version = 1\n" } } }, { text: "ok" }]);
  try {
    const r = await runPi(repo, model.url, "Relax the architecture rules", { CTX_PI_ROUTING: "keyword" });
    assert.equal(r.code, 0, r.stderr);
    const toolResult = model.requests[1].messages.filter((m) => m.role === "tool").map((m) => text(m.content)).join("\n");
    assert.match(toolResult, /fix the code, not the rules/);
    assert.match(execFileSync("cat", [join(repo, ".ctx/rules.toml")]).toString(), /rules\.forbidden/, "rules file untouched");
  } finally {
    model.close();
  }
});
