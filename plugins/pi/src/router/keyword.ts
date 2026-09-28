import type { RepoMeta, RouteDecision, WorkflowId } from "../types.ts";
import { extract } from "../symbols.ts";
import { CATALOG } from "./catalog.ts";

/**
 * Deterministic, offline router. Used when routing=keyword, when no Jev key is
 * configured, and as the fallback when a Jev call fails. It is intentionally
 * conservative: its confidences stay below Jev's typical values so the
 * orchestration picks lighter plans when it is unsure.
 */
export function keywordRoute(prompt: string, _meta?: RepoMeta): RouteDecision {
  const started = Date.now();
  const ex = extract(prompt);
  const codeSignals = ex.identifiers.length + ex.paths.length;

  // Priority order matters: "fix the failing test in foo_bar" is debug even
  // though it also says "fix"; "rename X everywhere" is change, not locate.
  const order: WorkflowId[] = ["debug", "review", "health", "change", "implement", "orient", "locate"];
  const hits = order.filter((w) => CATALOG[w].keywords.some((re) => re.test(prompt)));

  let workflow: WorkflowId = hits[0] ?? (codeSignals > 0 ? "locate" : "none");
  // "fix" alone is weak evidence for debug when the request is about adding something.
  if (workflow === "debug" && hits.includes("implement") && !/\b(fail|error|bug|panic|crash|exception)/i.test(prompt)) {
    workflow = "implement";
  }

  const mentionsRepo = /\b(this|the|our) (repo|repository|codebase|project|code)\b/i.test(prompt);
  const needsCodebase = workflow === "none" ? (mentionsRepo ? 0.5 : 0.1) : Math.min(0.9, 0.55 + 0.1 * codeSignals + (mentionsRepo ? 0.15 : 0));
  if (workflow === "none" && mentionsRepo) workflow = "orient";

  const willModify = ["implement", "change", "debug"].includes(workflow) ? 0.75 : 0.2;
  const scope = /\b(everywhere|all (callers|usages|files)|across|whole|entire|repo(sitory)?-wide)\b/i.test(prompt)
    ? 3
    : ex.identifiers.length > 2 || ex.paths.length > 1
      ? 2
      : 1;

  return {
    workflow,
    confidence: hits.length === 1 ? 0.6 : hits.length > 1 ? 0.5 : codeSignals > 0 ? 0.45 : 0.4,
    needsCodebase,
    willModify,
    scope,
    alternatives: hits.filter((w) => w !== workflow).slice(0, 2),
    source: "keyword",
    latencyMs: Date.now() - started,
  };
}
