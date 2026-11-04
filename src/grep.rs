//! Index-annotated text search (`ctx grep`).
//!
//! `ctx grep` is ripgrep's search engine (the `grep-searcher`, `grep-regex`
//! and `ignore` crates) with one addition: every match is tagged with what
//! the code-intelligence index knows about the identifier it hit.
//!
//! Text search and the call graph fail in opposite directions. Text search
//! has high recall and low precision: it finds every occurrence, including
//! every unrelated symbol that shares the name. The graph has high precision
//! and lower recall: what it binds is usually right, but sites it has no edge
//! for (interface members, dynamic dispatch, config, scripts) never appear,
//! and nothing signals their absence. `ctx grep` keeps text search's recall
//! (no match is ever dropped) and adds the graph's precision as labels, so
//! the reader can tell the matches it can trust from the ones it must check.
//!
//! # Tags
//!
//! The tag vocabulary is a contract ([`TAG_VOCABULARY_VERSION`]); models are
//! fine-tuned on it, so renaming a tag breaks them.
//!
//! | Tag          | Meaning                                                             |
//! |--------------|---------------------------------------------------------------------|
//! | `def`        | The identifier is the name of a symbol defined on this line.        |
//! | `ref`        | An index edge on this line references the identifier and resolves to a symbol. |
//! | `unresolved` | An index edge references the identifier but the resolver could not pick a target; `candidates` counts same-named symbols. |
//! | `unlinked`   | The file is indexed and current, but the index has no fact for this identifier on this line (comment, string, local, or a reference the parser does not model). Not evidence that the site is irrelevant. |
//! | `stale`      | The file changed since it was indexed; index facts are withheld.    |
//! | `unindexed`  | The file is not in the index (or there is no index).                |
//!
//! `def` and `ref` tags carry the `family` (trait, interface, or base type)
//! when the target is a member of one.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::Lossy;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::overrides::OverrideBuilder;
use ignore::types::TypesBuilder;
use ignore::WalkBuilder;
use serde::Serialize;

use crate::db::{Database, Edge, Symbol};
use crate::json::SymbolRef;
use crate::walker::FilePatternFilter;

/// Version of the tag vocabulary. Bump only for incompatible changes to tag
/// names or meanings; models trained on one version are bound to it.
pub const TAG_VOCABULARY_VERSION: u32 = 1;

/// Options for one `ctx grep` run.
#[derive(Debug, Clone, Default)]
pub struct GrepOptions {
    /// Regular expression (or literal, with `fixed_strings`).
    pub pattern: String,
    /// Treat the pattern as a literal string.
    pub fixed_strings: bool,
    /// Case-insensitive matching.
    pub ignore_case: bool,
    /// Case-insensitive unless the pattern contains an uppercase letter.
    pub smart_case: bool,
    /// Only match whole words.
    pub word: bool,
    /// ripgrep-style include/exclude globs (`!` prefix excludes).
    pub globs: Vec<String>,
    /// ripgrep file types to restrict to (e.g. `rust`, `py`).
    pub types: Vec<String>,
    /// Search hidden files and directories.
    pub hidden: bool,
    /// Respect `.gitignore`, `.ignore` and git excludes.
    pub use_gitignore: bool,
    /// Stop after this many matching lines (reported as truncated).
    pub max_results: Option<usize>,
    /// Annotate matches from the index. `false` gives plain grep output.
    pub annotate: bool,
}

/// What the index knows about the identifier at a match site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    Def,
    Ref,
    Unresolved,
    Unlinked,
    Stale,
    Unindexed,
}

impl TagKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TagKind::Def => "def",
            TagKind::Ref => "ref",
            TagKind::Unresolved => "unresolved",
            TagKind::Unlinked => "unlinked",
            TagKind::Stale => "stale",
            TagKind::Unindexed => "unindexed",
        }
    }

    /// All tags, in display order.
    pub const ALL: [TagKind; 6] = [
        TagKind::Def,
        TagKind::Ref,
        TagKind::Unresolved,
        TagKind::Unlinked,
        TagKind::Stale,
        TagKind::Unindexed,
    ];
}

/// One index annotation on a matching line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tag {
    pub tag: TagKind,
    /// The identifier the tag describes (absent for file-level tags).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Edge kind for `ref`/`unresolved` (`calls`, `uses`, `implements`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<String>,
    /// The symbol defined (`def`) or referenced (`ref`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SymbolRef>,
    /// Trait/interface/base type the target is a member of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// For `unresolved`: indexed symbols sharing the name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<i64>,
}

impl Tag {
    fn file_level(tag: TagKind) -> Self {
        Tag {
            tag,
            identifier: None,
            edge_kind: None,
            target: None,
            family: None,
            candidates: None,
        }
    }

    /// Display name of the target: its qualified name, or `file::name`.
    pub fn target_label(&self) -> Option<String> {
        self.target.as_ref().map(|t| match &t.qualified_name {
            Some(q) if !q.is_empty() => q.clone(),
            _ => format!("{}::{}", t.file, t.name),
        })
    }

    /// Compact single-line rendering, e.g. `ref calls Conn::close · family Closeable`.
    pub fn render(&self) -> String {
        let mut out = self.tag.as_str().to_string();
        match self.tag {
            TagKind::Def => {
                if let Some(label) = self.target_label() {
                    out.push(' ');
                    out.push_str(&label);
                }
            }
            TagKind::Ref => {
                if let Some(kind) = &self.edge_kind {
                    out.push(' ');
                    out.push_str(kind);
                }
                if let Some(label) = self.target_label() {
                    out.push(' ');
                    out.push_str(&label);
                }
            }
            TagKind::Unresolved | TagKind::Unlinked => {
                if let Some(identifier) = &self.identifier {
                    out.push(' ');
                    out.push_str(identifier);
                }
                if let Some(n) = self.candidates {
                    out.push_str(&format!(" · {n} candidates"));
                }
            }
            TagKind::Stale | TagKind::Unindexed => {}
        }
        if let Some(family) = &self.family {
            out.push_str(" · family ");
            out.push_str(family);
        }
        out
    }
}

/// A byte range of one match within its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// One matching line.
#[derive(Debug, Clone, Serialize)]
pub struct GrepMatch {
    pub file: String,
    pub line: u64,
    /// The line, without its trailing newline.
    pub text: String,
    pub submatches: Vec<Span>,
    /// Index annotations; empty when annotation is disabled.
    pub tags: Vec<Tag>,
}

/// Whether an index was consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    /// Annotations come from `.ctx/index.db`.
    Present,
    /// No index exists; every match is `unindexed`.
    Missing,
    /// Annotation was turned off (`--no-annotate`).
    Disabled,
}

/// Result of a `ctx grep` run.
#[derive(Debug, Clone, Serialize)]
pub struct GrepReport {
    pub pattern: String,
    pub index: IndexState,
    pub tag_vocabulary_version: u32,
    pub files_searched: usize,
    pub files_matched: usize,
    pub truncated: bool,
    pub matches: Vec<GrepMatch>,
}

impl GrepReport {
    /// Number of matching lines carrying each tag (a line with two `ref`
    /// tags counts once under `ref`).
    pub fn tag_counts(&self) -> BTreeMap<TagKind, usize> {
        let mut counts = BTreeMap::new();
        for m in &self.matches {
            let mut kinds: Vec<TagKind> = m.tags.iter().map(|t| t.tag).collect();
            kinds.sort();
            kinds.dedup();
            for kind in kinds {
                *counts.entry(kind).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Distinct `def`/`ref` targets with how often each was hit, most
    /// frequent first. Tells a reader whether a name binds to one symbol
    /// or to several unrelated ones.
    pub fn targets(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for m in &self.matches {
            for t in &m.tags {
                if matches!(t.tag, TagKind::Def | TagKind::Ref) {
                    if let Some(label) = t.target_label() {
                        *counts.entry(label).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut targets: Vec<_> = counts.into_iter().collect();
        targets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        targets
    }
}

/// Run a search rooted at `root`.
///
/// `scope` restricts which root-relative paths are searched (the CLI's
/// positional paths/globs). `db` is the index to annotate from; pass `None`
/// when there is no index (matches are tagged `unindexed`). When
/// `options.annotate` is false the index is not consulted at all.
pub fn search(
    root: &Path,
    scope: &FilePatternFilter,
    options: &GrepOptions,
    db: Option<&Database>,
) -> io::Result<GrepReport> {
    let matcher = build_matcher(options)?;
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .line_number(true)
        .build();

    let index = if !options.annotate {
        IndexState::Disabled
    } else if db.is_some() {
        IndexState::Present
    } else {
        IndexState::Missing
    };

    let mut report = GrepReport {
        pattern: options.pattern.clone(),
        index,
        tag_vocabulary_version: TAG_VOCABULARY_VERSION,
        files_searched: 0,
        files_matched: 0,
        truncated: false,
        matches: Vec::new(),
    };
    let mut annotator = db.filter(|_| options.annotate).map(Annotator::new);

    for path in walk(root, options)? {
        let relative = relative_path(root, &path);
        if !scope.matches(&relative) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        report.files_searched += 1;

        let mut lines: Vec<(u64, String)> = Vec::new();
        let sink = Lossy(|line_number, line| {
            lines.push((line_number, line.trim_end_matches(['\n', '\r']).to_string()));
            Ok(true)
        });
        if searcher.search_slice(&matcher, &bytes, sink).is_err() || lines.is_empty() {
            continue;
        }
        report.files_matched += 1;

        let file_facts = match (&mut annotator, options.annotate) {
            (Some(annotator), _) => Some(annotator.facts_for(&relative, &bytes)),
            (None, true) => Some(FileFacts::Unindexed),
            (None, false) => None,
        };

        for (line_number, text) in lines {
            if options
                .max_results
                .is_some_and(|max| report.matches.len() >= max)
            {
                report.truncated = true;
                return Ok(report);
            }
            let submatches = find_spans(&matcher, &text)?;
            let tags = match (&file_facts, &mut annotator) {
                (Some(facts), annotator) => {
                    tag_line(facts, annotator.as_mut(), line_number, &text, &submatches)
                }
                (None, _) => Vec::new(),
            };
            report.matches.push(GrepMatch {
                file: relative.clone(),
                line: line_number,
                text,
                submatches,
                tags,
            });
        }
    }
    Ok(report)
}

fn build_matcher(options: &GrepOptions) -> io::Result<RegexMatcher> {
    RegexMatcherBuilder::new()
        .case_insensitive(options.ignore_case)
        .case_smart(options.smart_case)
        .word(options.word)
        .fixed_strings(options.fixed_strings)
        .line_terminator(Some(b'\n'))
        .build(&options.pattern)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))
}

/// Files to search, in deterministic order, honoring ripgrep's default
/// filters (hidden files, `.gitignore`, `.ignore`) rather than the index's
/// own include rules, so files outside the index still show up (as
/// `unindexed`) instead of silently disappearing.
fn walk(root: &Path, options: &GrepOptions) -> io::Result<Vec<PathBuf>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!options.hidden)
        .git_ignore(options.use_gitignore)
        .git_global(options.use_gitignore)
        .git_exclude(options.use_gitignore)
        .ignore(options.use_gitignore)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b));

    if !options.globs.is_empty() {
        let mut overrides = OverrideBuilder::new(root);
        for glob in &options.globs {
            overrides.add(glob).map_err(invalid_input)?;
        }
        builder.overrides(overrides.build().map_err(invalid_input)?);
    }
    if !options.types.is_empty() {
        let mut types = TypesBuilder::new();
        types.add_defaults();
        for name in &options.types {
            types.select(name);
        }
        builder.types(types.build().map_err(invalid_input)?);
    }

    let mut paths = Vec::new();
    for entry in builder.build() {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_some_and(|t| t.is_file()) {
            paths.push(entry.into_path());
        }
    }
    Ok(paths)
}

fn invalid_input(e: ignore::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn find_spans(matcher: &RegexMatcher, text: &str) -> io::Result<Vec<Span>> {
    let mut spans = Vec::new();
    matcher
        .find_iter(text.as_bytes(), |m| {
            if m.start() < m.end() {
                spans.push(Span {
                    start: m.start(),
                    end: m.end(),
                });
            }
            true
        })
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(spans)
}

/// What the index holds for one file, loaded once per file with matches.
enum FileFacts {
    Unindexed,
    Stale,
    Current {
        defs: Vec<Symbol>,
        refs: Vec<(Edge, Option<Symbol>)>,
    },
}

struct Annotator<'a> {
    db: &'a Database,
    families: HashMap<String, Option<String>>,
    candidates: HashMap<String, i64>,
}

impl<'a> Annotator<'a> {
    fn new(db: &'a Database) -> Self {
        Annotator {
            db,
            families: HashMap::new(),
            candidates: HashMap::new(),
        }
    }

    fn facts_for(&mut self, relative: &str, bytes: &[u8]) -> FileFacts {
        let Ok(Some(stored)) = self.db.get_file_hash(relative) else {
            return FileFacts::Unindexed;
        };
        let current = std::str::from_utf8(bytes).map(crate::index::compute_hash);
        if current.as_deref() != Ok(stored.as_str()) {
            // Changed since indexing, or an extraction recorded as degraded:
            // line-level facts would be claims about a file that moved.
            return FileFacts::Stale;
        }
        FileFacts::Current {
            defs: self.db.get_file_symbols(relative).unwrap_or_default(),
            refs: self
                .db
                .edges_originating_in_file(relative)
                .unwrap_or_default(),
        }
    }

    fn family(&mut self, symbol: &Symbol) -> Option<String> {
        if let Some(cached) = self.families.get(&symbol.id) {
            return cached.clone();
        }
        let family = self.db.interface_family(symbol).ok().flatten();
        self.families.insert(symbol.id.clone(), family.clone());
        family
    }

    fn candidates(&mut self, name: &str) -> i64 {
        if let Some(n) = self.candidates.get(name) {
            return *n;
        }
        let n = self.db.count_symbols_named(name).unwrap_or(0);
        self.candidates.insert(name.to_string(), n);
        n
    }
}

/// Annotate one matching line.
fn tag_line(
    facts: &FileFacts,
    mut annotator: Option<&mut Annotator>,
    line: u64,
    text: &str,
    submatches: &[Span],
) -> Vec<Tag> {
    let (defs, refs) = match facts {
        FileFacts::Unindexed => return vec![Tag::file_level(TagKind::Unindexed)],
        FileFacts::Stale => return vec![Tag::file_level(TagKind::Stale)],
        FileFacts::Current { defs, refs } => (defs, refs),
    };

    let mut tags: Vec<Tag> = Vec::new();
    for (identifier, col) in identifiers_at(text, submatches) {
        let mut found = false;

        for def in defs
            .iter()
            .filter(|d| d.line_start as u64 == line && d.name == identifier)
        {
            let family = annotator.as_mut().and_then(|a| a.family(def));
            push_unique(
                &mut tags,
                Tag {
                    tag: TagKind::Def,
                    identifier: Some(identifier.clone()),
                    edge_kind: None,
                    target: Some(SymbolRef::from(def)),
                    family,
                    candidates: None,
                },
            );
            found = true;
        }

        if let Some((edge, target)) = closest_ref(refs, line, &identifier, col) {
            let tag = match target {
                Some(target) => Tag {
                    tag: TagKind::Ref,
                    identifier: Some(identifier.clone()),
                    edge_kind: Some(edge.kind.as_str().to_string()),
                    target: Some(SymbolRef::from(target)),
                    family: annotator.as_mut().and_then(|a| a.family(target)),
                    candidates: None,
                },
                None => Tag {
                    tag: TagKind::Unresolved,
                    identifier: Some(identifier.clone()),
                    edge_kind: Some(edge.kind.as_str().to_string()),
                    target: None,
                    family: None,
                    candidates: annotator.as_mut().map(|a| a.candidates(&identifier)),
                },
            };
            push_unique(&mut tags, tag);
            found = true;
        }

        if !found {
            push_unique(
                &mut tags,
                Tag {
                    tag: TagKind::Unlinked,
                    identifier: Some(identifier),
                    edge_kind: None,
                    target: None,
                    family: None,
                    candidates: None,
                },
            );
        }
    }

    // `unlinked` answers "does the index know anything about this match?".
    // Once any identifier in the match is linked, the rest (keywords,
    // receivers, locals) are noise.
    if tags.iter().any(|t| t.tag != TagKind::Unlinked) {
        tags.retain(|t| t.tag != TagKind::Unlinked);
    }
    if tags.is_empty() {
        // The match touched no identifier (punctuation, whitespace): the
        // index cannot speak to it.
        tags.push(Tag::file_level(TagKind::Unlinked));
    }
    tags
}

/// Render a line's tags for text output: `ref calls a::b; def c::d`.
/// Unlinked identifiers are listed under one `unlinked` tag.
pub fn render_tags(tags: &[Tag]) -> String {
    let mut rendered: Vec<String> = Vec::new();
    let unlinked: Vec<&str> = tags
        .iter()
        .filter(|t| t.tag == TagKind::Unlinked)
        .filter_map(|t| t.identifier.as_deref())
        .collect();
    let mut unlinked_done = false;
    for tag in tags {
        if tag.tag == TagKind::Unlinked && !unlinked.is_empty() {
            if !unlinked_done {
                rendered.push(format!("unlinked {}", unlinked.join(", ")));
                unlinked_done = true;
            }
            continue;
        }
        rendered.push(tag.render());
    }
    rendered.join("; ")
}

fn push_unique(tags: &mut Vec<Tag>, tag: Tag) {
    if !tags.contains(&tag) {
        tags.push(tag);
    }
}

/// Among edges on `line` that reference `identifier`, the one whose start
/// column is closest before the identifier. Edges record where the whole
/// reference expression starts (e.g. the receiver of a method call), which
/// is at or before the name itself.
fn closest_ref<'f>(
    refs: &'f [(Edge, Option<Symbol>)],
    line: u64,
    identifier: &str,
    col: usize,
) -> Option<&'f (Edge, Option<Symbol>)> {
    let candidates: Vec<_> = refs
        .iter()
        .filter(|(e, _)| e.line.map(u64::from) == Some(line) && e.target_name == identifier)
        .collect();
    candidates
        .iter()
        .filter(|(e, _)| e.col.is_none_or(|c| c as usize <= col))
        .max_by_key(|(e, _)| e.col.unwrap_or(0))
        .or_else(|| candidates.first())
        .copied()
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Identifiers that overlap any submatch, with their start columns, in line
/// order. A match inside an identifier (`clos` in `close`) is widened to
/// the whole identifier.
fn identifiers_at(text: &str, submatches: &[Span]) -> Vec<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_ident_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        let end = i;
        let overlaps = submatches.iter().any(|s| s.start < end && start < s.end);
        let starts_like_identifier = !bytes[start].is_ascii_digit();
        if overlaps && starts_like_identifier {
            if let Some(word) = text.get(start..end) {
                out.push((word.to_string(), start));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_widened_and_filtered() {
        let text = "    self.close(42); other.closed()";
        let spans = vec![Span { start: 9, end: 13 }];
        assert_eq!(identifiers_at(text, &spans), vec![("close".to_string(), 9)]);

        let all = vec![Span {
            start: 0,
            end: text.len(),
        }];
        let names: Vec<_> = identifiers_at(text, &all)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["self", "close", "other", "closed"]);
    }

    #[test]
    fn match_without_identifier_is_unlinked() {
        let facts = FileFacts::Current {
            defs: Vec::new(),
            refs: Vec::new(),
        };
        let tags = tag_line(&facts, None, 1, "a + b", &[Span { start: 2, end: 3 }]);
        assert_eq!(tags, vec![Tag::file_level(TagKind::Unlinked)]);
    }

    #[test]
    fn unlinked_identifiers_are_dropped_once_anything_links() {
        let def = Symbol {
            id: "src/a.rs::embed@3".into(),
            file_path: "src/a.rs".into(),
            name: "embed".into(),
            qualified_name: None,
            kind: crate::db::SymbolKind::Method,
            visibility: crate::db::Visibility::Private,
            signature: None,
            brief: None,
            docstring: None,
            line_start: 3,
            line_end: 5,
            col_start: 4,
            col_end: 5,
            parent_id: None,
            source: None,
        };
        let facts = FileFacts::Current {
            defs: vec![def],
            refs: Vec::new(),
        };
        let text = "    fn embed(&self) {";
        let spans = [Span { start: 4, end: 12 }];
        let tags = tag_line(&facts, None, 3, text, &spans);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag, TagKind::Def);
        assert_eq!(render_tags(&tags), "def src/a.rs::embed");

        // Same text on a line with no facts: one merged `unlinked` tag.
        let tags = tag_line(&facts, None, 9, text, &spans);
        assert_eq!(render_tags(&tags), "unlinked fn, embed");
    }

    #[test]
    fn file_level_states_short_circuit() {
        let spans = [Span { start: 0, end: 3 }];
        assert_eq!(
            tag_line(&FileFacts::Stale, None, 1, "foo()", &spans),
            vec![Tag::file_level(TagKind::Stale)]
        );
        assert_eq!(
            tag_line(&FileFacts::Unindexed, None, 1, "foo()", &spans),
            vec![Tag::file_level(TagKind::Unindexed)]
        );
    }

    fn edge(name: &str, line: u32, col: u32, kind: crate::db::EdgeKind) -> Edge {
        Edge {
            source_id: "caller".into(),
            target_id: None,
            target_name: name.into(),
            kind,
            line: Some(line),
            col: Some(col),
            context: None,
        }
    }

    #[test]
    fn closest_ref_prefers_the_edge_starting_nearest_before_the_name() {
        use crate::db::EdgeKind::Calls;
        let refs = vec![
            (edge("close", 3, 4, Calls), None),
            (edge("close", 3, 16, Calls), None),
        ];
        // `a.close(); b.close()` with the second `close` at col 18.
        let (picked, _) = closest_ref(&refs, 3, "close", 18).unwrap();
        assert_eq!(picked.col, Some(16));
        let (picked, _) = closest_ref(&refs, 3, "close", 6).unwrap();
        assert_eq!(picked.col, Some(4));
        assert!(closest_ref(&refs, 4, "close", 6).is_none());
    }

    #[test]
    fn render_is_compact_and_stable() {
        let tag = Tag {
            tag: TagKind::Ref,
            identifier: Some("close".into()),
            edge_kind: Some("calls".into()),
            target: Some(SymbolRef {
                name: "close".into(),
                qualified_name: Some("Conn::close".into()),
                kind: "method".into(),
                file: "src/conn.rs".into(),
                line_start: 1,
                line_end: 2,
            }),
            family: Some("Closeable".into()),
            candidates: None,
        };
        assert_eq!(tag.render(), "ref calls Conn::close · family Closeable");

        let unresolved = Tag {
            tag: TagKind::Unresolved,
            identifier: Some("handler".into()),
            edge_kind: Some("calls".into()),
            target: None,
            family: None,
            candidates: Some(4),
        };
        assert_eq!(unresolved.render(), "unresolved handler · 4 candidates");
    }
}
