import type { Exec, ExecResult, RepoMeta, SymbolRef } from "./types.ts";

/** Minimum ctx release whose CLI/JSON contract this extension targets. */
export const MIN_CTX_VERSION = "0.4.0";

export interface CallOptions {
  signal?: AbortSignal;
  timeout?: number;
}

/** A parsed `--json` envelope; `data` is absent when the command failed. */
export interface JsonResult<T> {
  code: number;
  data?: T;
  error?: string;
}

export interface Caller {
  symbol: SymbolRef;
  distance: number;
  line?: number;
  context?: string;
}

export interface Dependency {
  distance: number;
  kind: string;
  line?: number;
  resolved?: SymbolRef | null;
  target_name: string;
}

export interface SearchHit {
  symbol: SymbolRef;
  score: number;
  brief?: string;
  signature?: string;
  fan_in?: number;
  match_type?: string;
}

type Endpoint = Partial<SymbolRef> & { file?: string };

export interface CheckViolation {
  rule: string;
  rule_id: string;
  reason?: string;
  message: string;
  file?: string;
  line?: number;
  from?: Endpoint;
  to?: Endpoint;
  subject?: Endpoint;
  metric?: string;
}

export interface ScoreData {
  against: string | null;
  baseline?: string;
  files_changed: number;
  metrics: Record<string, number>;
  failed_conditions: string[];
  check_violations_note?: string | null;
  notes?: string[];
}

export interface HotspotEntry {
  file: string;
  score: number;
  commits?: number;
  lines?: number;
  complexity?: number;
}

const NO_EMBEDDINGS = /no embeddings found|run 'ctx embed'|is `ollama serve` running|embedding error/i;

/** True when stderr says a semantic command needs `ctx embed` first. */
export function isMissingEmbeddings(stderr: string): boolean {
  return NO_EMBEDDINGS.test(stderr);
}

/**
 * Thin, typed wrapper over the ctx CLI.
 *
 * Every call goes through the injected `exec` so the extension can use
 * `pi.exec` (abortable, cwd-aware) and tests can substitute a fake. The
 * wrapper never throws for ctx's own failures: exit code 2 (operational
 * error) comes back as `{ code: 2, error }` so callers can degrade instead of
 * breaking the agent's turn.
 */
export class Ctx {
  private readonly exec: Exec;
  private readonly cwd: string;
  private readonly bin: string;
  private fileCache: string[] | undefined;

  constructor(exec: Exec, cwd: string, bin = "ctx") {
    this.exec = exec;
    this.cwd = cwd;
    this.bin = bin;
  }

  async run(args: string[], opts: CallOptions = {}): Promise<ExecResult> {
    try {
      const res = await this.exec(this.bin, args, {
        cwd: this.cwd,
        signal: opts.signal,
        timeout: opts.timeout ?? 60_000,
      });
      // pi's exec reports a killed (timed out / aborted) process as exit 0;
      // partial output must never be mistaken for a result.
      return res.killed ? { ...res, code: 124, stderr: `${res.stderr}\nctx timed out or was aborted` } : res;
    } catch (err) {
      return { stdout: "", stderr: String((err as Error)?.message ?? err), code: 127, killed: false };
    }
  }

  async json<T>(args: string[], opts: CallOptions = {}): Promise<JsonResult<T>> {
    const sep = args.indexOf("--");
    const withJson = sep === -1 ? [...args, "--json"] : [...args.slice(0, sep), "--json", ...args.slice(sep)];
    const res = await this.run(withJson, opts);
    // Exit 1 means "ran fine, findings" for gating commands; the payload is valid.
    if (res.code !== 0 && res.code !== 1) {
      return { code: res.code, error: firstLine(res.stderr) || `ctx exited with ${res.code}` };
    }
    try {
      const env = JSON.parse(res.stdout) as { data?: T };
      return { code: res.code, data: env.data };
    } catch {
      return { code: 2, error: "ctx returned malformed JSON" };
    }
  }

  /** Is ctx installed and new enough? */
  async available(opts: CallOptions = {}): Promise<{ ok: boolean; version?: string; reason?: string }> {
    const v = await this.run(["--version"], { ...opts, timeout: 10_000 });
    if (v.code !== 0) {
      return { ok: false, reason: `'${this.bin}' not found on PATH (install: cargo install agentis-ctx)` };
    }
    const version = v.stdout.trim().split(/\s+/).pop() ?? "";
    if (!versionAtLeast(version, MIN_CTX_VERSION)) {
      return { ok: false, version, reason: `ctx ${version || "(unknown version)"} is older than ${MIN_CTX_VERSION}; run 'ctx self-update'` };
    }
    return { ok: true, version };
  }

  index(opts: CallOptions = {}): Promise<ExecResult> {
    this.fileCache = undefined;
    return this.run(["index"], { timeout: 300_000, ...opts });
  }

  async repoMeta(git: boolean, opts: CallOptions = {}): Promise<RepoMeta | undefined> {
    const r = await this.json<{ files: number; symbols: number; functions: number }>(
      ["query", "stats"],
      opts,
    );
    if (!r.data) return undefined;
    return {
      files: r.data.files,
      symbols: r.data.symbols,
      functions: r.data.functions,
      embeddings: "unknown",
      git,
    };
  }

  map(budget: number, focus?: string, opts: CallOptions = {}): Promise<ExecResult> {
    const args = ["map", "--budget", String(Math.max(200, Math.round(budget)))];
    if (focus) args.push("--focus", focus);
    return this.run(args, opts);
  }

  async find(name: string, opts: CallOptions = {}): Promise<SymbolRef[]> {
    const r = await this.json<{ symbols: SymbolRef[] }>(["query", "find", "--", name], opts);
    return r.data?.symbols ?? [];
  }

  /** Every indexed file path (cached per Ctx; call `invalidate()` after reindex). */
  async files(opts: CallOptions = {}): Promise<string[]> {
    if (!this.fileCache) {
      const r = await this.json<{ files: string[] }>(["query", "files"], opts);
      if (!r.data) return [];
      this.fileCache = r.data.files;
    }
    return this.fileCache;
  }

  invalidate(): void {
    this.fileCache = undefined;
  }

  /** Symbols defined in one file (an outline), in index order. */
  async outline(file: string, limit = 60, opts: CallOptions = {}): Promise<SymbolRef[]> {
    const r = await this.json<{ symbols: SymbolRef[] }>(["query", "find", "--file", file, "--limit", String(limit), "--", ""], opts);
    return (r.data?.symbols ?? []).filter((s) => s.file === file);
  }

  async search(query: string, limit: number, opts: CallOptions = {}): Promise<SearchHit[]> {
    const r = await this.json<{ results: SearchHit[] }>(
      ["search", "--limit", String(limit), "--", query],
      opts,
    );
    return r.data?.results ?? [];
  }

  /** Semantic `similar`, or keyword mode when embeddings are unavailable. */
  async similar(
    query: string,
    limit: number,
    semantic: boolean,
    opts: CallOptions = {},
  ): Promise<{ hits: SearchHit[]; missingEmbeddings: boolean }> {
    const args = ["similar", "--limit", String(limit)];
    if (!semantic) args.push("--keyword");
    args.push("--", query);
    const r = await this.json<{ results: SearchHit[] }>(args, opts);
    if (semantic && r.error && isMissingEmbeddings(r.error)) {
      const kw = await this.similar(query, limit, false, opts);
      return { hits: kw.hits, missingEmbeddings: true };
    }
    return { hits: r.data?.results ?? [], missingEmbeddings: false };
  }

  source(symbol: string, file?: string, opts: CallOptions = {}): Promise<ExecResult> {
    const args = ["source"];
    if (file) args.push("--file", file);
    return this.run([...args, "--", symbol], opts);
  }

  explain(symbol: string, opts: CallOptions = {}): Promise<ExecResult> {
    return this.run(["explain", "--", symbol], opts);
  }

  /**
   * Callers of a symbol. `ambiguous` lists the same-named candidates when ctx
   * cannot tell which one is meant; callers is then empty and must not be
   * read as "unused".
   */
  async callers(
    symbol: string,
    depth: number,
    file?: string,
    opts: CallOptions = {},
  ): Promise<{ callers: Caller[]; ambiguous: SymbolRef[] }> {
    const args = ["query", "callers", "--depth", String(depth)];
    if (file) args.push("--file", file);
    const r = await this.json<{ callers: Caller[]; ambiguous?: SymbolRef[] }>([...args, "--", symbol], opts);
    return { callers: r.data?.callers ?? [], ambiguous: r.data?.ambiguous ?? [] };
  }

  async deps(symbol: string, file?: string, opts: CallOptions = {}): Promise<Dependency[]> {
    const args = ["query", "deps"];
    if (file) args.push("--file", file);
    const r = await this.json<{ dependencies: Dependency[] }>([...args, "--", symbol], opts);
    return r.data?.dependencies ?? [];
  }

  /**
   * Transitive impact. `total: 0` is not proof of no impact: ctx builds
   * without the default `duckdb` feature always report 0, so callers should
   * lean on `callers()` and treat impact as additional evidence.
   */
  async impact(symbol: string, opts: CallOptions = {}): Promise<{ total: number; impacted: Caller[] }> {
    const r = await this.json<{ total: number; impacted: Caller[] }>(["query", "impact", "--", symbol], opts);
    return { total: r.data?.total ?? 0, impacted: r.data?.impacted ?? [] };
  }

  /** `ctx smart` bundle; `missingEmbeddings` lets callers switch strategy. */
  async smart(
    task: string,
    maxTokens: number,
    opts: CallOptions = {},
  ): Promise<{ text?: string; missingEmbeddings: boolean; error?: string }> {
    const res = await this.run(
      ["smart", "--max-tokens", String(Math.max(500, Math.round(maxTokens))), "--format", "markdown", "--no-tree", "--", task],
      opts,
    );
    if (res.code === 0) return { text: res.stdout, missingEmbeddings: false };
    return { missingEmbeddings: isMissingEmbeddings(res.stderr), error: firstLine(res.stderr) };
  }

  check(against: string | undefined, opts: CallOptions = {}): Promise<JsonResult<{ violations: CheckViolation[]; summary: { violations: number } }>> {
    const args = ["check"];
    if (against) args.push("--against", against);
    return this.json(args, opts);
  }

  score(against: string, failOn: string, opts: CallOptions = {}): Promise<JsonResult<ScoreData>> {
    return this.json(["score", "--against", against, "--fail-on", failOn], { timeout: 180_000, ...opts });
  }

  async hotspots(limit: number, opts: CallOptions = {}): Promise<HotspotEntry[]> {
    const r = await this.json<{ entries: HotspotEntry[] }>(["hotspots", "--limit", String(limit)], opts);
    return r.data?.entries ?? [];
  }
}

/** Numeric semver comparison of the leading x.y.z (pre-release tags ignored). */
export function versionAtLeast(version: string, min: string): boolean {
  const parse = (v: string) => (v.match(/(\d+)\.(\d+)\.(\d+)/)?.slice(1).map(Number) ?? null);
  const a = parse(version);
  const b = parse(min)!;
  if (!a) return false;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return true;
}

export function firstLine(s: string): string {
  return (
    s
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.length > 0 && !l.startsWith("note:")) ?? ""
  ).replace(/^Error:\s*/, "");
}
