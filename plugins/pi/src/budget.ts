/**
 * Token budgeting for injected context.
 *
 * Small local models are the design target: an 8K-window model must not have
 * a third of its window taken by "helpful" context. The budget is a fraction
 * of the *free* window, scaled by the router's estimate of task size, and
 * clamped to [MIN, cap].
 */

export const MIN_INJECT_TOKENS = 400;

/** Fraction of the free context window one injection may use. */
const FREE_WINDOW_SHARE = 0.15;

/** Rough, tokenizer-free estimate (≈4 chars/token for code and English). */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4);
}

/**
 * @param contextWindow model context window in tokens (undefined: assume 32K)
 * @param usedTokens    tokens already in the conversation (undefined: 0)
 * @param scope         router size estimate, 0 (one spot) .. 3 (repo-wide)
 * @param cap           configured hard upper bound
 */
export function injectionBudget(
  contextWindow: number | undefined,
  usedTokens: number | null | undefined,
  scope: number,
  cap: number,
): number {
  const window = contextWindow && contextWindow > 0 ? contextWindow : 32_000;
  const free = Math.max(0, window - (usedTokens ?? 0));
  const scale = 0.7 + 0.2 * clamp(scope, 0, 3); // 0.7 .. 1.3
  const raw = free * FREE_WINDOW_SHARE * scale;
  return Math.round(clamp(raw, MIN_INJECT_TOKENS, Math.max(MIN_INJECT_TOKENS, cap)));
}

/** Cut text to roughly `tokens`, on a line boundary, with a visible marker. */
export function truncateToTokens(text: string, tokens: number): string {
  const maxChars = Math.max(0, tokens * 4);
  if (text.length <= maxChars) return text;
  const cut = text.lastIndexOf("\n", maxChars);
  const head = text.slice(0, cut > maxChars * 0.5 ? cut : maxChars);
  const dropped = estimateTokens(text) - estimateTokens(head);
  return `${head}\n… [truncated ~${dropped} tokens]`;
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}
