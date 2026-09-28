import type { WorkflowId } from "../types.ts";

/**
 * The routing vocabulary. `description` is what Jev sees as a Choice option;
 * `keywords` drive the offline keyword router. Keep descriptions short,
 * concrete and mutually exclusive: they are the classifier's only definition
 * of each class.
 */
export const CATALOG: Record<WorkflowId, { description: string; keywords: RegExp[] }> = {
  none: {
    description:
      "Not about this codebase: general knowledge, chit-chat, writing prose, or a task that needs no source code from this repository",
    keywords: [],
  },
  orient: {
    description:
      "Understand the repository as a whole: overview, architecture, module layout, where to start, how the project is organized",
    keywords: [
      /\b(overview|architecture|high[- ]level|structure|organi[sz]ed|layout|walk ?through|onboard|tour|where (do|should) i start|how is (this|the) (repo|project|code))/i,
    ],
  },
  locate: {
    description:
      "Find or explain specific existing code: where something is defined or handled, how a function or feature works, what a symbol does",
    keywords: [
      /\b(where (is|are|does)|which (file|function|module)|how does|what does|explain|look up|locate|defined|definition|implemented|handled)\b/i,
    ],
  },
  implement: {
    description:
      "Add new functionality: implement a feature, add a command, option, endpoint or helper, write new code that does not exist yet",
    keywords: [
      /\b(add|implement|introduce|create|build|support|new (feature|command|option|flag|endpoint|helper|function))\b/i,
    ],
  },
  change: {
    description:
      "Modify existing code that has callers: rename, refactor, change a signature or behavior, move or delete code, update usages",
    keywords: [
      /\b(rename|refactor|change|modify|replace|move|extract|inline|delete|remove|deprecate|signature|update (all )?(usages|callers)|everywhere)\b/i,
    ],
  },
  debug: {
    description:
      "Diagnose and fix a bug: a failing test, an error message, a panic or exception, wrong output, a regression",
    keywords: [
      /\b(fail(s|ing|ed)?|error|bug|broken|crash|panic|exception|traceback|stack ?trace|regression|wrong|incorrect|doesn'?t work|not working|fix)\b/i,
    ],
  },
  review: {
    description:
      "Review pending changes: review a diff, branch or pull request, check whether the current changes are safe or ready",
    keywords: [/\b(review|diff|pull request|\bpr\b|branch|my changes|these changes|ready to merge|before (i )?merge)\b/i],
  },
  health: {
    description:
      "Assess code quality: hotspots, complexity, duplication, technical debt, architecture rule violations, maintainability trends",
    keywords: [/\b(hotspot|complexity|duplicat|tech(nical)? debt|maintainab|code quality|code smell|coupling|architecture rules?)\b/i],
  },
};

export const ROUTABLE: WorkflowId[] = Object.keys(CATALOG) as WorkflowId[];
