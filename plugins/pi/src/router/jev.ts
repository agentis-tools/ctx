import { TypeSafeClient, choice, noul, score } from "@typesafe-ai/sdk";
import type { RepoMeta, RouteDecision, SymbolRef, WorkflowId } from "../types.ts";
import { CATALOG, ROUTABLE } from "./catalog.ts";

/**
 * Jev (TypeSafe's System One model) as the ctx router.
 *
 * One request per user prompt asks four typed questions in parallel:
 *   - two Nouls whose mean gates whether the codebase matters at all
 *     (the two-phrasing gate follows TypeSafe's skill-suggestion recipe),
 *   - a Noul for "will this modify code" (arms the quality gates),
 *   - a Choice over the workflow catalog,
 *   - a Score for task size (scales the token budget).
 *
 * Privacy: the state holds the user's request, the previous request, and
 * repository counts. No source code is sent. Symbol disambiguation sends
 * only symbol names, kinds and file paths.
 */

/** The minimal slice of the SDK the router uses; tests pass a fake. */
export interface SystemOneLike {
  systemOne(request: { state: unknown; questions: Record<string, unknown> }, options?: { signal?: AbortSignal; timeout?: number }): PromiseLike<{
    answers: Record<string, any>;
    model?: string;
  }>;
}

export function createJevClient(apiKey: string): SystemOneLike {
  // Routing sits on the critical path of every prompt: fail fast and let the
  // keyword router take over instead of stalling the agent.
  return new TypeSafeClient({ apiKey, timeout: 4_000, retry: { maxRetries: 1, backoffMaxMs: 500 }, logLevel: "off" }) as unknown as SystemOneLike;
}

const MAX_REQUEST_CHARS = 2_000;

export interface JevRouteInput {
  prompt: string;
  previousPrompt?: string;
  meta?: RepoMeta;
}

export async function jevRoute(
  client: SystemOneLike,
  input: JevRouteInput,
  signal?: AbortSignal,
): Promise<RouteDecision> {
  const started = Date.now();
  const criteria = Object.fromEntries(ROUTABLE.map((w) => [w, CATALOG[w].description]));
  const state: Record<string, unknown> = {
    request: input.prompt.slice(0, MAX_REQUEST_CHARS),
    context: "A developer is talking to a coding agent that works inside a software repository.",
  };
  if (input.previousPrompt) state.previous_request = input.previousPrompt.slice(0, 500);
  if (input.meta) {
    state.repository = {
      files: input.meta.files,
      symbols: input.meta.symbols,
      functions: input.meta.functions,
      semantic_search_available: input.meta.embeddings === "yes",
    };
  }

  const res = await client.systemOne(
    {
      state,
      questions: {
        needs_codebase: noul(
          "Does handling this request require reading or searching this repository's source code?",
        ),
        about_repo: noul("Is this request about the code in the current repository (rather than general knowledge or prose)?"),
        will_modify: noul("Will completing this request require editing, adding or deleting code files?"),
        workflow: choice("Which workflow best fits the developer's request?", criteria),
        scope: score("How much of the codebase does this request involve?", [
          "one spot: a single function or line",
          "a few related files",
          "a whole subsystem or module",
          "repository-wide",
        ]),
      },
    },
    { signal },
  );

  const a = res.answers;
  const workflowAnswer = a.workflow as { choice: string; confidence: number; probabilities: Record<string, number> };
  const ranked = Object.entries(workflowAnswer.probabilities ?? {})
    .sort((x, y) => y[1] - x[1])
    .map(([w]) => w as WorkflowId);

  return {
    workflow: workflowAnswer.choice as WorkflowId,
    confidence: workflowAnswer.confidence,
    needsCodebase: (num(a.needs_codebase?.noul) + num(a.about_repo?.noul)) / 2,
    willModify: num(a.will_modify?.noul),
    scope: num(a.scope?.score),
    alternatives: ranked.filter((w) => w !== workflowAnswer.choice && w !== "none").slice(0, 2),
    source: "jev",
    latencyMs: Date.now() - started,
  };
}

/**
 * Ask Jev which of several same-named or related symbols the request targets.
 * Returns the chosen index and confidence, or undefined on failure.
 */
export async function jevPickSymbol(
  client: SystemOneLike,
  prompt: string,
  candidates: SymbolRef[],
  signal?: AbortSignal,
): Promise<{ index: number; confidence: number } | undefined> {
  if (candidates.length < 2) return candidates.length === 1 ? { index: 0, confidence: 1 } : undefined;
  const options = Object.fromEntries(
    candidates.slice(0, 20).map((s, i) => [`s${i}`, `${s.kind} ${s.qualified_name || s.name} in ${s.file}`]),
  );
  const res = await client.systemOne(
    {
      state: { request: prompt.slice(0, MAX_REQUEST_CHARS) },
      questions: { target: choice("Which code symbol is the main subject of the request?", options) },
    },
    { signal },
  );
  const t = res.answers.target as { choice: string; confidence: number };
  const index = Number(String(t.choice).slice(1));
  return Number.isInteger(index) ? { index, confidence: t.confidence } : undefined;
}

function num(v: unknown): number {
  return typeof v === "number" && Number.isFinite(v) ? v : 0;
}
