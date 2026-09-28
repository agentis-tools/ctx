import type { RepoMeta, RouteDecision, RoutingMode, SymbolRef } from "../types.ts";
import { keywordRoute } from "./keyword.ts";
import { jevPickSymbol, jevRoute, type SystemOneLike } from "./jev.ts";

export { CATALOG, ROUTABLE } from "./catalog.ts";
export { createJevClient, type SystemOneLike } from "./jev.ts";

/**
 * Routes prompts with Jev when possible and falls back to the keyword router
 * on any failure (no key, network, rate limit, timeout). A routing failure
 * must never fail the user's turn.
 */
export class Router {
  /** Set after an auth failure so we stop retrying a bad key every prompt. */
  private jevDisabledReason: string | undefined;
  /** Circuit breaker: consecutive transient failures and when Jev may be retried. */
  private failures = 0;
  private pausedUntil = 0;
  private readonly now: () => number;

  private readonly mode: RoutingMode;
  private readonly client: SystemOneLike | undefined;

  constructor(mode: RoutingMode, client: SystemOneLike | undefined, now: () => number = Date.now) {
    this.mode = mode;
    this.client = client;
    this.now = now;
  }

  /** After this many consecutive network/timeout failures, pause Jev. */
  static readonly BREAKER_THRESHOLD = 2;
  static readonly BREAKER_PAUSE_MS = 5 * 60_000;

  get usesJev(): boolean {
    return this.mode === "jev" && this.client !== undefined && this.jevDisabledReason === undefined && this.now() >= this.pausedUntil;
  }

  status(): string {
    if (this.mode !== "jev") return this.mode;
    if (!this.client) return "keyword (no JEV_API_KEY)";
    if (this.jevDisabledReason) return `keyword (${this.jevDisabledReason})`;
    if (this.now() < this.pausedUntil) return "keyword (Jev unreachable; retrying later)";
    return "jev";
  }

  async route(
    input: { prompt: string; previousPrompt?: string; meta?: RepoMeta },
    signal?: AbortSignal,
  ): Promise<RouteDecision> {
    if (!this.usesJev) {
      const d = keywordRoute(input.prompt, input.meta);
      if (this.mode === "jev") d.fallbackReason = !this.client ? "no JEV_API_KEY" : (this.jevDisabledReason ?? "Jev paused after network failures");
      return d;
    }
    try {
      const d = await jevRoute(this.client!, input, signal);
      this.failures = 0;
      return d;
    } catch (err) {
      const reason = describe(err);
      if (/401|403|auth|permission/i.test(reason)) this.jevDisabledReason = "Jev key rejected";
      else if (++this.failures >= Router.BREAKER_THRESHOLD) {
        // A black-holed network would otherwise add the full timeout to every prompt.
        this.pausedUntil = this.now() + Router.BREAKER_PAUSE_MS;
        this.failures = 0;
      }
      const d = keywordRoute(input.prompt, input.meta);
      d.fallbackReason = reason;
      return d;
    }
  }

  /** Choose the target symbol among candidates; first candidate without Jev. */
  async pickSymbol(prompt: string, candidates: SymbolRef[], signal?: AbortSignal): Promise<number> {
    if (candidates.length < 2 || !this.usesJev) return 0;
    try {
      const r = await jevPickSymbol(this.client!, prompt, candidates, signal);
      return r && r.index < candidates.length ? r.index : 0;
    } catch {
      return 0;
    }
  }
}

function describe(err: unknown): string {
  const e = err as { status?: number; name?: string; message?: string };
  if (e?.status) return `Jev HTTP ${e.status}`;
  return `${e?.name ?? "Error"}: ${e?.message ?? String(err)}`.slice(0, 160);
}
