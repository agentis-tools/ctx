/**
 * Pull likely code references out of a natural-language request.
 *
 * Jev cannot extract strings, only choose among options, so candidate
 * identifiers are found here and then verified against the ctx index (and,
 * when several match, disambiguated by Jev as a Choice).
 */

export interface Extracted {
  /** Identifier-shaped tokens, most specific first. */
  identifiers: string[];
  /** File or directory paths mentioned in the request. */
  paths: string[];
  /** Remaining meaningful words, for keyword search. */
  terms: string[];
}

const STOP = new Set(
  [
    "a an and are as at be but by can could do does for from has have how i if in into is it its " +
    "me my of on or our please should so that the their them then there these this those to up us " +
    "use using was we what when where which while who why will with would you your make add fix " +
    "change update write create find show tell explain code file files function functions method " +
    "class struct test tests repo repository codebase project new all any also just like need want " +
    "sure some more most other than too very get set run work works working does doesn don't isn't " +
    "it's let's here now able about after again before being below between both each few further " +
    "only own same such through under until",
  ]
    .join("")
    .split(" ")
    .filter(Boolean),
);

const PATH_RE = /(?:^|[\s`'"(])((?:\.{0,2}\/)?(?:[\w.-]+\/)+[\w.-]*|[\w-]+\.(?:rs|ts|tsx|js|jsx|mjs|cjs|py|go|java|kt|rb|php|c|cc|cpp|h|hpp|cs|swift|scala|toml|json|ya?ml|md))(?=$|[\s`'"),:;])/g;
const BACKTICK_RE = /`([^`\n]{1,120})`/g;
const IDENT_RE = /[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.|#)[A-Za-z_][A-Za-z0-9_]*)*/g;

/** Identifier shape that is unlikely to be an English word. */
function looksLikeCode(tok: string): boolean {
  if (/::|#/.test(tok)) return true;
  if (/\./.test(tok) && !/\.(?:$|\s)/.test(tok)) return /^[A-Za-z_]\w*\.[A-Za-z_]\w*/.test(tok) && !/^(e\.g|i\.e|etc)\b/i.test(tok);
  if (/_/.test(tok) && /[a-z]/i.test(tok)) return true; // snake_case / SCREAMING_CASE
  if (/[a-z][A-Z]/.test(tok)) return true; // camelCase / PascalCase with a hump
  if (/^[A-Z][a-z]+[A-Z]/.test(tok)) return true;
  return false;
}

export function extract(prompt: string): Extracted {
  const identifiers: string[] = [];
  const paths: string[] = [];
  const seen = new Set<string>();
  const push = (arr: string[], v: string) => {
    const key = v.toLowerCase();
    if (!seen.has(key)) {
      seen.add(key);
      arr.push(v);
    }
  };

  for (const m of prompt.matchAll(PATH_RE)) {
    const p = m[1].replace(/[.,;:]+$/, "");
    if (p.length > 2 && !/^https?:/.test(p)) push(paths, p);
  }

  // Backticked spans are the strongest signal; keep their identifier core.
  for (const m of prompt.matchAll(BACKTICK_RE)) {
    const inner = m[1].trim();
    if (/[\/]/.test(inner) || /\.\w{1,5}$/.test(inner)) continue; // a path, handled above
    const core = inner.match(IDENT_RE)?.[0];
    if (core && core.length > 1 && !STOP.has(core.toLowerCase())) push(identifiers, core.replace(/\(\)$/, ""));
  }

  const withoutPaths = paths.reduce((s, p) => s.split(p).join(" "), prompt);
  for (const m of withoutPaths.matchAll(IDENT_RE)) {
    const tok = m[0].replace(/[.]+$/, "");
    if (tok.length > 2 && looksLikeCode(tok)) push(identifiers, tok);
  }

  const terms: string[] = [];
  for (const w of withoutPaths.toLowerCase().match(/[a-z][a-z0-9_]{2,}/g) ?? []) {
    if (!STOP.has(w) && !seen.has(w)) {
      seen.add(w);
      terms.push(w);
    }
  }

  return { identifiers: identifiers.slice(0, 8), paths: paths.slice(0, 4), terms: terms.slice(0, 8) };
}

/** Last path segment of a qualified identifier (`a::b::c` -> `c`). */
export function leafName(id: string): string {
  const parts = id.split(/::|\.|#/);
  return parts[parts.length - 1] ?? id;
}
