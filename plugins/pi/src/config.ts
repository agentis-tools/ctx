import type { GateMode, RoutingMode, ToolActivation } from "./types.ts";

/**
 * Resolved extension configuration.
 *
 * Precedence: pi CLI flag > environment variable > default. Flags are
 * registered in index.ts (`pi --ctx-routing model`, ...); environment
 * variables make the same settings available to scripts and CI.
 */
export interface Config {
  /** jev (default) | keyword | model. See README "Routing modes". */
  routing: RoutingMode;
  /** Jev/TypeSafe API key; routing silently degrades to keyword without one. */
  jevApiKey: string | undefined;
  /** Below this Jev "needs the codebase" probability nothing is injected. */
  gateThreshold: number;
  /** Below this workflow confidence the router falls back to a light plan. */
  minConfidence: number;
  /** stable (default, keeps the prompt cache warm) | routed. */
  tools: ToolActivation;
  /** off | report (default) | block. */
  gate: GateMode;
  /** `ctx score --fail-on` expression evaluated at the end of a run. */
  failOn: string;
  /** How many automatic continuations `block` may request per run. */
  maxGateContinuations: number;
  /** How many earlier ctx context messages stay in the model's context. */
  keepContextMessages: number;
  /** Show injected context in the transcript (it is always sent to the model). */
  showContext: boolean;
  /** Path or name of the ctx binary. */
  ctxBin: string;
  /** Upper bound for one injection, in tokens, regardless of window size. */
  maxInjectTokens: number;
}

export const DEFAULTS: Omit<Config, "jevApiKey"> = {
  routing: "jev",
  gateThreshold: 0.3,
  minConfidence: 0.45,
  tools: "stable",
  gate: "report",
  failOn: "check_violations>0",
  maxGateContinuations: 1,
  keepContextMessages: 2,
  showContext: false,
  ctxBin: "ctx",
  maxInjectTokens: 6000,
};

export type FlagReader = (name: string) => boolean | string | undefined;
export type Env = Record<string, string | undefined>;

const ROUTING: readonly RoutingMode[] = ["jev", "keyword", "model"];
const GATES: readonly GateMode[] = ["off", "report", "block"];
const TOOLS: readonly ToolActivation[] = ["stable", "routed"];

function pick<T extends string>(allowed: readonly T[], ...values: unknown[]): T | undefined {
  for (const v of values) {
    if (typeof v === "string" && (allowed as readonly string[]).includes(v.trim().toLowerCase())) {
      return v.trim().toLowerCase() as T;
    }
  }
  return undefined;
}

function num(value: string | undefined, fallback: number, min: number, max: number): number {
  if (value === undefined || value.trim() === "") return fallback;
  const n = Number(value);
  return Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : fallback;
}

function bool(value: unknown): boolean | undefined {
  if (typeof value === "boolean") return value;
  if (typeof value !== "string") return undefined;
  const v = value.trim().toLowerCase();
  if (["1", "true", "yes", "on"].includes(v)) return true;
  if (["0", "false", "no", "off"].includes(v)) return false;
  return undefined;
}

export function resolveConfig(flag: FlagReader, env: Env): Config {
  // CTX_GATE_BLOCKING=1 is the existing ctx harness convention (Claude/Codex
  // stop hooks); honour it so one setting works across harnesses.
  const legacyBlock = bool(env.CTX_GATE_BLOCKING) ? "block" : undefined;
  return {
    routing: pick(ROUTING, flag("ctx-routing"), env.CTX_PI_ROUTING) ?? DEFAULTS.routing,
    // JEV_API_KEY is what `ctx judge` reads; TYPESAFE_API_KEY is the SDK default.
    jevApiKey: nonEmpty(env.JEV_API_KEY) ?? nonEmpty(env.TYPESAFE_API_KEY),
    gateThreshold: num(env.CTX_PI_GATE_THRESHOLD, DEFAULTS.gateThreshold, 0, 1),
    minConfidence: num(env.CTX_PI_MIN_CONFIDENCE, DEFAULTS.minConfidence, 0, 1),
    tools: pick(TOOLS, flag("ctx-tools"), env.CTX_PI_TOOLS) ?? DEFAULTS.tools,
    gate: pick(GATES, flag("ctx-gate"), env.CTX_PI_GATE, legacyBlock) ?? DEFAULTS.gate,
    failOn: nonEmpty(env.CTX_PI_FAIL_ON) ?? DEFAULTS.failOn,
    maxGateContinuations: Math.round(
      num(env.CTX_PI_MAX_GATE_CONTINUATIONS, DEFAULTS.maxGateContinuations, 0, 5),
    ),
    keepContextMessages: Math.round(
      num(env.CTX_PI_KEEP_CONTEXT, DEFAULTS.keepContextMessages, 1, 20),
    ),
    showContext: bool(flag("ctx-show")) ?? bool(env.CTX_PI_SHOW_CONTEXT) ?? DEFAULTS.showContext,
    ctxBin: nonEmpty(env.CTX_BIN) ?? DEFAULTS.ctxBin,
    maxInjectTokens: Math.round(
      num(env.CTX_PI_MAX_INJECT_TOKENS, DEFAULTS.maxInjectTokens, 200, 50_000),
    ),
  };
}

function nonEmpty(v: string | undefined): string | undefined {
  return v !== undefined && v.trim() !== "" ? v.trim() : undefined;
}
