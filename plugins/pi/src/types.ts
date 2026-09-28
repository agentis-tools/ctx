/**
 * Shared types for the ctx pi extension.
 *
 * Kept free of runtime imports so every module (and the tests) can use them
 * without loading pi or the TypeSafe SDK.
 */

/** Who decides which ctx workflow runs for a prompt. */
export type RoutingMode = "jev" | "keyword" | "model";

/** What happens when the session's changes trip a ctx quality gate. */
export type GateMode = "off" | "report" | "block";

/** Whether the set of active ctx tools changes per prompt. */
export type ToolActivation = "stable" | "routed";

/**
 * The workflows the router can pick. Each maps to a cookbook recipe and a
 * deterministic plan of ctx commands (see workflows.ts).
 */
export const WORKFLOW_IDS = [
  "none",
  "orient",
  "locate",
  "implement",
  "change",
  "debug",
  "review",
  "health",
] as const;
export type WorkflowId = (typeof WORKFLOW_IDS)[number];

/** Repository facts that are safe to send to the router (no source code). */
export interface RepoMeta {
  files: number;
  symbols: number;
  functions: number;
  /** "no" once a semantic command has reported missing embeddings. */
  embeddings: "yes" | "no" | "unknown";
  git: boolean;
}

export interface RouteDecision {
  workflow: WorkflowId;
  /** Router confidence in `workflow`, 0..1. */
  confidence: number;
  /** Probability the request needs knowledge of this codebase, 0..1. */
  needsCodebase: number;
  /** Probability the request will modify code files, 0..1. */
  willModify: number;
  /** Expected size, 0 (one spot) .. 3 (repository-wide). */
  scope: number;
  /** Next most likely workflows, most likely first. */
  alternatives: WorkflowId[];
  source: "jev" | "keyword";
  latencyMs: number;
  /** Set when a preferred router failed and this decision is a fallback. */
  fallbackReason?: string;
}

/** A symbol as ctx reports it in JSON output (`SymbolRef`). */
export interface SymbolRef {
  name: string;
  qualified_name?: string | null;
  kind: string;
  file: string;
  line_start: number;
  line_end: number;
  visibility?: string;
}

/** Result of running one ctx subprocess. */
export interface ExecResult {
  stdout: string;
  stderr: string;
  code: number;
  killed: boolean;
}

/** Subprocess runner; matches `pi.exec` so the extension can pass it through. */
export type Exec = (
  command: string,
  args: string[],
  options?: { signal?: AbortSignal; timeout?: number; cwd?: string },
) => Promise<ExecResult>;

/** One block of model-facing context produced by a workflow. */
export interface Section {
  title: string;
  body: string;
}
