//! Structural (code-shape) rules for `ctx check`, matched with ast-grep.
//!
//! Graph rules say what may depend on what; structural rules say what code
//! may look like: "no `.unwrap()` in handlers", "no `console.log` outside
//! scripts". A rule is an ast-grep pattern (`$X.unwrap()`, `console.log($$$)`)
//! over ctx's own tree-sitter grammars, so there is no second parser.
//!
//! ```toml
//! [[rules.structural]]
//! id = "no-unwrap-in-handlers"
//! language = "rust"
//! pattern = "$X.unwrap()"
//! paths = ["src/handlers/**"]      # optional; default: every file of the language
//! exclude = ["src/handlers/tests/**"]
//! reason = "handlers must return errors, not panic"
//! ```
//!
//! With `ctx check --against REF` a match is reported only when it overlaps a
//! line the diff added or changed, so pre-existing matches in a touched file
//! are not blamed on the change.

use std::borrow::Cow;
use std::collections::HashMap;

use ast_grep_core::matcher::PatternBuilder;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_core::Language;
use ast_grep_core::{Pattern, PatternError};

use crate::error::{CtxError, Result};

/// One of ctx's grammars, as an ast-grep language.
#[derive(Clone)]
pub struct Grammar {
    ts: TSLanguage,
    /// Replacement for `$` in patterns where `$` cannot start an identifier
    /// (ast-grep's convention), so `$X` parses as an identifier.
    expando: char,
}

impl Grammar {
    /// The grammar for a ctx language name (`files.language`).
    pub fn for_language(name: &str) -> Option<Grammar> {
        let (ts, expando): (TSLanguage, char) = match name {
            "rust" => (tree_sitter_rust::LANGUAGE.into(), 'µ'),
            "go" => (tree_sitter_go::LANGUAGE.into(), 'µ'),
            "python" => (tree_sitter_python::LANGUAGE.into(), 'µ'),
            // C/C++ accept `µ` in identifiers as an extension; ast-grep uses U+10000
            "c" => (tree_sitter_c::LANGUAGE.into(), '𐀀'),
            "cpp" => (tree_sitter_cpp::LANGUAGE.into(), '𐀀'),
            "typescript" => (tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), '$'),
            "tsx" => (tree_sitter_typescript::LANGUAGE_TSX.into(), '$'),
            "javascript" => (tree_sitter_javascript::LANGUAGE.into(), '$'),
            _ => return None,
        };
        Some(Grammar { ts, expando })
    }
}

impl Language for Grammar {
    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        if self.expando == '$' {
            return Cow::Borrowed(query);
        }
        // `$X` / `$$$ARGS` -> `µX` / `µµµARGS`, as ast-grep's own language pack does
        // A run of `$` is a metavariable when a name follows it (`$X`, `$$$ARGS`,
        // `$_`) or when it is the anonymous multi-match `$$$`.
        let chars: Vec<char> = query.chars().collect();
        let mut out = String::with_capacity(query.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] != '$' {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            let run = chars[i..].iter().take_while(|c| **c == '$').count();
            let next = chars.get(i + run);
            let meta = run == 3 || next.is_some_and(|n| *n == '_' || n.is_ascii_uppercase());
            let c = if meta { self.expando } else { '$' };
            out.extend(std::iter::repeat_n(c, run));
            i += run;
        }
        Cow::Owned(out)
    }
    fn expando_char(&self) -> char {
        self.expando
    }
    fn kind_to_id(&self, kind: &str) -> u16 {
        self.ts.id_for_node_kind(kind, true)
    }
    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.ts.field_id_for_name(field).map(|f| f.get())
    }
    fn build_pattern(
        &self,
        builder: &PatternBuilder,
    ) -> std::result::Result<Pattern, PatternError> {
        builder.build(|src| StrDoc::try_new(src, self.clone()))
    }
}

impl LanguageExt for Grammar {
    fn get_ts_language(&self) -> TSLanguage {
        self.ts.clone()
    }
}

/// A compiled structural pattern.
pub struct CompiledPattern {
    grammar: Grammar,
    pattern: Pattern,
}

impl std::fmt::Debug for CompiledPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CompiledPattern")
    }
}

/// A match: 1-based start and end line, and the matched text's first line.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub line_start: u32,
    pub line_end: u32,
    pub snippet: String,
}

impl CompiledPattern {
    /// Compile `pattern` for `language`; errors name the rule for the user.
    pub fn new(language: &str, pattern: &str) -> Result<CompiledPattern> {
        let grammar = Grammar::for_language(language).ok_or_else(|| {
            CtxError::Other(format!(
                "structural rules: unsupported language '{language}' \
                 (supported: rust, go, python, c, cpp, typescript, tsx, javascript)"
            ))
        })?;
        let pattern = Pattern::try_new(pattern, grammar.clone()).map_err(|e| {
            CtxError::Other(format!(
                "structural rules: invalid {language} pattern '{pattern}': {e}"
            ))
        })?;
        Ok(CompiledPattern { grammar, pattern })
    }

    /// All matches in `source`.
    pub fn find_all(&self, source: &str) -> Vec<Match> {
        self.find_in(&self.grammar.ast_grep(source))
    }

    /// All matches in an already parsed file (see [`parse`]); the tree must
    /// come from the same language.
    pub fn find_in(&self, tree: &Tree) -> Vec<Match> {
        tree.root()
            .find_all(&self.pattern)
            .map(|m| {
                let text = m.text();
                Match {
                    line_start: m.start_pos().line() as u32 + 1,
                    line_end: m.end_pos().line() as u32 + 1,
                    snippet: text
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(120)
                        .collect(),
                }
            })
            .collect()
    }
}

/// A parsed file, shared by every structural rule of its language.
pub type Tree = ast_grep_core::AstGrep<StrDoc<Grammar>>;

/// Parse `source` once with the grammar for `language`.
pub fn parse(language: &str, source: &str) -> Option<Tree> {
    Grammar::for_language(language).map(|g| g.ast_grep(source))
}

/// Changed line ranges (1-based, inclusive) per file, from `git diff -U0`.
pub type ChangedLines = HashMap<String, Vec<(u32, u32)>>;

/// Parse `git diff -U0` output into added/changed line ranges per new-side file.
pub fn parse_changed_lines(diff: &str) -> ChangedLines {
    let mut out: ChangedLines = HashMap::new();
    let mut file: Option<String> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").map(str::to_string);
        } else if let (Some(f), Some(hunk)) = (&file, line.strip_prefix("@@ ")) {
            // @@ -a,b +c,d @@
            if let Some(new) = hunk.split_whitespace().find(|p| p.starts_with('+')) {
                let mut it = new[1..].split(',');
                let start: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let count: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                if count > 0 {
                    out.entry(f.clone())
                        .or_default()
                        .push((start, start + count - 1));
                }
            }
        }
    }
    out
}

/// Whether a match overlaps any changed range.
pub fn overlaps(m: &Match, ranges: &[(u32, u32)]) -> bool {
    ranges
        .iter()
        .any(|(s, e)| m.line_start <= *e && *s <= m.line_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_each_language() {
        let cases = [
            (
                "rust",
                "$X.unwrap()",
                "fn f() { let a = g().unwrap(); let b = h()?; }",
                1,
            ),
            (
                "go",
                "panic($$$)",
                "package p\nfunc f() {\n\tpanic(\"x\")\n\tpanic(err)\n}\n",
                2,
            ),
            (
                "python",
                "print($$$)",
                "def f():\n    print('a')\n    log('b')\n",
                1,
            ),
            (
                "typescript",
                "console.log($$$)",
                "function f(x: number) { console.log(x); console.error(x) }",
                1,
            ),
            (
                "javascript",
                "console.log($$$)",
                "function f(x) { console.log(x, 1) }",
                1,
            ),
            // like the ast-grep CLI, a bare `f($$$)` parses as a C declaration, so
            // C patterns name their arguments
            (
                "c",
                "strcpy($A, $B)",
                "void f(char *a, char *b) { strcpy(a, b); strncpy(a, b, 3); }",
                1,
            ),
            (
                "cpp",
                "strcpy($A, $B)",
                "void f(char *a, const char *b) { strcpy(a, b); }",
                1,
            ),
        ];
        for (lang, pat, src, want) in cases {
            let p = CompiledPattern::new(lang, pat).unwrap();
            assert_eq!(p.find_all(src).len(), want, "{lang}: {pat}");
        }
    }

    #[test]
    fn reports_lines_and_snippet() {
        let p = CompiledPattern::new("rust", "$X.unwrap()").unwrap();
        let m = p.find_all("fn f() {\n    let a = g()\n        .unwrap();\n}\n");
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].line_start, m[0].line_end), (2, 3));
        assert_eq!(m[0].snippet, "g()");
    }

    #[test]
    fn rejects_unknown_language_and_bad_pattern() {
        assert!(CompiledPattern::new("cobol", "x").is_err());
        assert!(CompiledPattern::new("rust", "").is_err());
    }

    #[test]
    fn changed_lines_from_diff() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,0 +4,2 @@ fn x\n+a\n+b\n@@ -10 +12 @@\n-c\n+d\n@@ -20,2 +22,0 @@\n-e\n-f\n+++ /dev/null\n";
        let c = parse_changed_lines(diff);
        assert_eq!(c["src/a.rs"], vec![(4, 5), (12, 12)]);
        let m = Match {
            line_start: 5,
            line_end: 7,
            snippet: String::new(),
        };
        assert!(overlaps(&m, &c["src/a.rs"]));
        let m = Match {
            line_start: 6,
            line_end: 11,
            snippet: String::new(),
        };
        assert!(!overlaps(&m, &c["src/a.rs"]));
    }
}
