import type { CheckViolation, Ctx } from "./ctx-cli.ts";
import type { GateMode } from "./types.ts";

/**
 * Keeps the index fresh and governs what the agent changes.
 *
 * - After each successful edit/write: incremental `ctx index`, then
 *   `ctx check --against <baseline>`; only violations not reported before are
 *   appended to that tool result, so the model sees them at the moment it can
 *   still fix them (same contract as the Claude/Codex post-tool-use hook).
 * - At the end of a run that edited files: `ctx score --against <baseline>
 *   --fail-on <expr>`. `report` shows the scorecard; `block` additionally asks
 *   pi for one more model turn to address failed conditions (bounded).
 *
 * Operational errors (exit 2) fail open: they are reported, never treated as
 * a pass or a block.
 */
export class Governance {
  private dirty = false;
  private continuations = 0;
  private reported = new Set<string>();
  private queue: Promise<unknown> = Promise.resolve();

  private readonly ctx: Ctx;
  private readonly mode: GateMode;
  private readonly failOn: string;
  private readonly maxContinuations: number;
  /** Session-start commit; undefined outside git. */
  private readonly baseline: string | undefined;

  constructor(ctx: Ctx, mode: GateMode, failOn: string, maxContinuations: number, baseline: string | undefined) {
    this.ctx = ctx;
    this.mode = mode;
    this.failOn = failOn;
    this.maxContinuations = maxContinuations;
    this.baseline = baseline;
  }

  get enabled(): boolean {
    return this.mode !== "off";
  }

  /** New agent run: reset the per-run continuation budget. */
  startRun(): void {
    this.continuations = 0;
  }

  /** Serialize index/check runs; parallel edit results must not race the index. */
  private serial<T>(fn: () => Promise<T>): Promise<T> {
    const next = this.queue.then(fn, fn);
    this.queue = next.catch(() => undefined);
    return next;
  }

  /** A tool changed files in a way we don't check per call (e.g. bash). */
  markDirty(): void {
    if (this.enabled) this.dirty = true;
  }

  /** Returns a note for the model when the edit introduced new violations. */
  afterEdit(signal?: AbortSignal): Promise<string | undefined> {
    if (!this.enabled) return Promise.resolve(undefined);
    this.dirty = true;
    return this.serial(async () => {
      const idx = await this.ctx.index({ signal, timeout: 120_000 });
      if (idx.code !== 0) return undefined; // fail open; the stop gate will report it
      const c = await this.ctx.check(this.baseline, { signal });
      if (!c.data) return undefined;
      const fresh = c.data.violations.filter((v) => !this.reported.has(violationKey(v)));
      fresh.forEach((v) => this.reported.add(violationKey(v)));
      if (!fresh.length) return undefined;
      return (
        `[ctx check] This edit introduced ${fresh.length} architecture rule violation(s):\n` +
        fresh.slice(0, 10).map(fmtViolation).join("\n") +
        "\nFix the dependency direction rather than editing .ctx/rules.toml."
      );
    });
  }

  /** Full on-demand check for the ctx_check tool. */
  checkNow(signal?: AbortSignal): Promise<string> {
    return this.serial(async () => {
      const idx = await this.ctx.index({ signal, timeout: 120_000 });
      if (idx.code !== 0) return `ctx index failed (${idx.stderr.trim().split("\n")[0] ?? idx.code}); gates not evaluated.`;
      const parts: string[] = [];
      const c = await this.ctx.check(this.baseline, { signal });
      if (c.error) parts.push(`ctx check error: ${c.error}`);
      else if (c.data) parts.push(c.data.violations.length ? `Architecture violations:\n${c.data.violations.slice(0, 15).map(fmtViolation).join("\n")}` : "Architecture rules: no violations.");
      if (this.baseline) {
        const s = await this.ctx.score(this.baseline, this.failOn, { signal });
        if (s.error) parts.push(`ctx score error: ${s.error}`);
        else if (s.data) parts.push(scoreText(s.data, this.failOn));
      }
      return parts.join("\n\n");
    });
  }

  /**
   * End-of-run gate. Returns the message to append and whether pi should run
   * one more turn, or undefined when there is nothing to report.
   */
  async atSettle(signal?: AbortSignal): Promise<{ text: string; failed: boolean; error: boolean; continue: boolean } | undefined> {
    if (!this.enabled || !this.dirty || !this.baseline) return undefined;
    this.dirty = false;
    const s = await this.serial(() => this.ctx.score(this.baseline!, this.failOn, { signal }));
    if (s.error || !s.data) {
      return { text: `[ctx gate] could not evaluate (${s.error ?? "no data"}); treat the gate as NOT passed.`, failed: false, error: true, continue: false };
    }
    const failed = s.data.failed_conditions.length > 0;
    const wantsContinue = failed && this.mode === "block" && this.continuations < this.maxContinuations;
    if (wantsContinue) this.continuations++;
    const verdict = failed
      ? this.mode === "block"
        ? wantsContinue
          ? "The quality gate FAILED. Fix the failed conditions before finishing."
          : "The quality gate still FAILS after the allowed retries; tell the user what remains."
        : "The quality gate reported findings (report mode, not blocking)."
      : "The quality gate passed.";
    return { text: `[ctx gate] ${verdict}\n${scoreText(s.data, this.failOn)}`, failed, error: false, continue: wantsContinue };
  }
}

/**
 * Identity of a violation that survives unrelated edits: rule plus the
 * endpoints' names and files, but no line numbers (messages embed `file:line`,
 * which shifts whenever code above it changes).
 */
export function violationKey(v: CheckViolation): string {
  const ep = (e: CheckViolation["from"]) => (e ? `${e.file ?? ""}#${e.qualified_name ?? e.name ?? ""}` : "");
  if (v.from || v.to || v.subject) return `${v.rule_id}|${ep(v.from)}|${ep(v.to)}|${ep(v.subject)}|${v.metric ?? ""}`;
  return `${v.rule_id}|${v.message.replace(/:\d+\b/g, "")}`;
}

function fmtViolation(v: CheckViolation): string {
  return `- ${v.message}${v.reason ? ` (${v.reason})` : ""}`;
}

function scoreText(
  s: { metrics: Record<string, number>; failed_conditions: string[]; files_changed: number; against: string | null },
  failOn: string,
): string {
  const m = Object.entries(s.metrics)
    .filter(([k]) => k !== "files_changed")
    .map(([k, v]) => `${k}=${v}`)
    .join(", ");
  const f = s.failed_conditions.length ? `Failed: ${s.failed_conditions.join(", ")}` : `Gate (${failOn}): clean`;
  return `Scorecard vs ${short(s.against)} — ${s.files_changed} files changed; ${m}. ${f}`;
}

function short(ref: string | null): string {
  return ref && /^[0-9a-f]{40}$/.test(ref) ? ref.slice(0, 10) : (ref ?? "?");
}
