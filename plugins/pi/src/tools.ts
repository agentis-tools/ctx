import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import type { Ctx } from "./ctx-cli.ts";
import { truncateToTokens } from "./budget.ts";
import { fmtSym } from "./workflows.ts";
import type { RepoMeta } from "./types.ts";

/**
 * Five model-callable tools with flat, single-string parameters.
 *
 * Small models call tools far more reliably when schemas are trivial and the
 * output is short plain text, so every tool takes one required string (plus
 * at most one optional one) and truncates its output to a token cap derived
 * from the live context window.
 */
export interface ToolDeps {
  getCtx: () => Ctx | undefined;
  getMeta: () => RepoMeta | undefined;
  /** Max tokens a single tool result may return. */
  resultTokens: () => number;
  /** Runs the post-edit gate on demand (ctx_check). */
  runCheck: (signal?: AbortSignal) => Promise<string>;
}

const text = (t: string) => ({ content: [{ type: "text" as const, text: t }], details: undefined });

function need(deps: ToolDeps): Ctx {
  const ctx = deps.getCtx();
  if (!ctx) throw new Error("ctx is not available in this session (is the ctx CLI installed and the repo indexed?)");
  return ctx;
}

export function registerTools(pi: ExtensionAPI, deps: ToolDeps): void {
  pi.registerTool({
    name: "ctx_find",
    label: "ctx find",
    description:
      "Find functions, types or methods in this repository by name or by what they do. Returns names with file:line. Use it instead of grep when looking for code.",
    promptSnippet: "Find code symbols by name or description (file:line results)",
    parameters: Type.Object({
      query: Type.String({ description: "A symbol name (e.g. parse_config) or a few words describing the code" }),
    }),
    async execute(_id, params, signal) {
      const ctx = need(deps);
      const hits = await ctx.search(params.query, 12, { signal });
      if (!hits.length) return text(`No symbols match "${params.query}". Try a shorter name or different words.`);
      const lines = hits.map((h) => `${fmtSym(h.symbol)}${h.brief ? ` — ${h.brief.split("\n")[0].slice(0, 90)}` : ""}`);
      return text(truncateToTokens(lines.join("\n"), deps.resultTokens()));
    },
  });

  pi.registerTool({
    name: "ctx_source",
    label: "ctx source",
    description:
      "Read the source code of one function, method or type by name. Cheaper than reading the whole file. Add `file` if several symbols share the name.",
    promptSnippet: "Read one symbol's source code by name",
    parameters: Type.Object({
      symbol: Type.String({ description: "Symbol name, optionally qualified (e.g. Indexer::index_file)" }),
      file: Type.Optional(Type.String({ description: "File path to disambiguate" })),
    }),
    async execute(_id, params, signal) {
      const ctx = need(deps);
      const r = await ctx.source(params.symbol, params.file, { signal });
      if (r.code !== 0 || /not found/i.test(r.stdout.slice(0, 200)) || !r.stdout.trim()) {
        return text(`Symbol "${params.symbol}" not found. Use ctx_find to get the exact name.`);
      }
      return text(truncateToTokens(r.stdout.trim(), deps.resultTokens()));
    },
  });

  pi.registerTool({
    name: "ctx_impact",
    label: "ctx impact",
    description:
      "List the callers of a function and everything transitively affected by changing it. Call this BEFORE renaming, deleting or changing the signature or behavior of existing code.",
    promptSnippet: "Show callers and blast radius of changing a function",
    parameters: Type.Object({
      symbol: Type.String({ description: "Function or method name, optionally qualified (e.g. Indexer::index_file)" }),
      file: Type.Optional(Type.String({ description: "File path, if several functions share the name" })),
    }),
    async execute(_id, params, signal) {
      const ctx = need(deps);
      const { callers, ambiguous } = await ctx.callers(params.symbol, 1, params.file, { signal });
      if (!callers.length && ambiguous.length) {
        // Never report "no callers" when ctx could not tell which symbol was meant.
        return text(
          truncateToTokens(
            `"${params.symbol}" is ambiguous; ${ambiguous.length} symbols share the name. Call ctx_impact again with one of these as symbol (and its file):\n` +
              ambiguous.slice(0, 20).map((s) => `- ${fmtSym(s)}`).join("\n"),
            deps.resultTokens(),
          ),
        );
      }
      const impact = await ctx.impact(params.symbol, { signal });
      if (!callers.length && !impact.total) {
        return text(`No callers of "${params.symbol}" found in the index. It may be an entry point, called dynamically, or called from outside the indexed files; verify with a text search before assuming it is unused.`);
      }
      const files = [...new Set(impact.impacted.map((c) => c.symbol.file))];
      const out =
        `Direct callers (${callers.length}):\n` +
        callers.map((c) => `- ${fmtSym(c.symbol)}${c.line ? ` at line ${c.line}` : ""}`).join("\n") +
        (impact.total ? `\n\nTransitively affected: ${impact.total} symbols in ${files.length} files:\n${files.join("\n")}` : "");
      return text(truncateToTokens(out, deps.resultTokens()));
    },
  });

  pi.registerTool({
    name: "ctx_similar",
    label: "ctx similar",
    description:
      "Before writing a new function, check whether similar code already exists. Describe what the new code should do in one sentence.",
    promptSnippet: "Find existing code similar to what you are about to write",
    parameters: Type.Object({
      description: Type.String({ description: "One sentence: what the new code should do" }),
    }),
    async execute(_id, params, signal) {
      const ctx = need(deps);
      const meta = deps.getMeta();
      const r = await ctx.similar(params.description, 8, meta?.embeddings !== "no", { signal });
      if (r.missingEmbeddings && meta) meta.embeddings = "no";
      const hits = r.hits.filter((h) => h.score >= 0.4);
      if (!hits.length) return text("Nothing similar found. Writing new code is reasonable.");
      const lines = hits.map((h) => `${fmtSym(h.symbol)}${h.fan_in ? `, ${h.fan_in} callers` : ""}${h.brief ? ` — ${h.brief.split("\n")[0].slice(0, 90)}` : ""}`);
      return text(truncateToTokens(`Existing candidates (read with ctx_source before reusing):\n${lines.join("\n")}`, deps.resultTokens()));
    },
  });

  pi.registerTool({
    name: "ctx_check",
    label: "ctx check",
    description:
      "Re-index and check your edits so far against the repository's architecture rules and quality gates. Use it before saying you are done.",
    promptSnippet: "Check your changes against architecture rules and quality gates",
    parameters: Type.Object({}),
    async execute(_id, _params, signal) {
      need(deps);
      return text(truncateToTokens(await deps.runCheck(signal), deps.resultTokens()));
    },
  });
}
