//! Precise call edges from a SCIP index (opt-in).
//!
//! Tree-sitter extraction plus name resolution is fast enough to run after
//! every edit, but it binds calls by name. A SCIP index is produced by a
//! compiler-backed indexer (`scip-go`, `scip-python`, `scip-typescript`,
//! `rust-analyzer scip`, `scip-clang`) and records, for every reference, the
//! symbol it resolves to and where that symbol is defined. `ctx scip import`
//! reads such an index and, for each call edge, looks up the reference at the
//! call site:
//!
//! - the symbol is defined in the repository: the edge is bound to the ctx
//!   function/method whose span contains that definition;
//! - the symbol has no definition in the index: the call leaves the repository
//!   (standard library, dependency) and the edge is unbound;
//! - no reference at the call site, or the definition is not a function ctx
//!   knows: the edge is left as it was.
//!
//! Every edge SCIP answered gets provenance `scip`; `ctx judge edges` skips
//! those, so the model is only asked about what the indexer could not settle.
//! Answers are cached with the content hashes of the calling and the target
//! file, and `ctx index` re-applies them while both files are unchanged:
//! edits fall back to name resolution until the next SCIP run.
//!
//! The decoder below reads only the protobuf fields ctx needs, so SCIP support
//! adds no dependency.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::db::{CallableSpan, Database, ScipEdge, PROVENANCE_SCIP};
use crate::error::{CtxError, Result};

/// `SymbolRole.Definition` in scip.proto.
const ROLE_DEFINITION: i32 = 0x1;

/// A reference or definition in one document.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    /// 0-based line.
    pub line: u32,
    /// 0-based start character on that line.
    pub start: u32,
    pub symbol: String,
    pub roles: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub relative_path: String,
    pub occurrences: Vec<Occurrence>,
}

// ------------------------------------------------------------ protobuf reader

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Result<u64> {
        let mut out = 0u64;
        for shift in (0..64).step_by(7) {
            let b = *self
                .buf
                .get(self.pos)
                .ok_or_else(|| bad("truncated varint"))?;
            self.pos += 1;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(bad("varint too long"))
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.varint()? as usize;
        let end = self.pos.checked_add(len).filter(|e| *e <= self.buf.len());
        let end = end.ok_or_else(|| bad("truncated field"))?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Next (field number, wire type).
    fn key(&mut self) -> Result<(u32, u8)> {
        let k = self.varint()?;
        Ok(((k >> 3) as u32, (k & 7) as u8))
    }

    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => self.pos += 8,
            2 => {
                self.bytes()?;
            }
            5 => self.pos += 4,
            w => return Err(bad(&format!("unsupported wire type {w}"))),
        }
        if self.pos > self.buf.len() {
            return Err(bad("truncated field"));
        }
        Ok(())
    }
}

fn bad(msg: &str) -> CtxError {
    CtxError::Other(format!("invalid SCIP index: {msg}"))
}

fn string(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Decode `Index.documents` (field 2) from a serialized SCIP index.
pub fn decode_index(buf: &[u8]) -> Result<Vec<Document>> {
    let mut r = Reader::new(buf);
    let mut docs = Vec::new();
    while !r.done() {
        let (field, wire) = r.key()?;
        if field == 2 && wire == 2 {
            docs.push(decode_document(r.bytes()?)?);
        } else {
            r.skip(wire)?;
        }
    }
    Ok(docs)
}

fn decode_document(buf: &[u8]) -> Result<Document> {
    let mut r = Reader::new(buf);
    let mut doc = Document::default();
    while !r.done() {
        let (field, wire) = r.key()?;
        match (field, wire) {
            (1, 2) => doc.relative_path = string(r.bytes()?),
            (2, 2) => {
                if let Some(o) = decode_occurrence(r.bytes()?)? {
                    doc.occurrences.push(o);
                }
            }
            _ => r.skip(wire)?,
        }
    }
    Ok(doc)
}

fn decode_occurrence(buf: &[u8]) -> Result<Option<Occurrence>> {
    let mut r = Reader::new(buf);
    let mut range: Vec<i64> = Vec::new();
    let (mut symbol, mut roles) = (String::new(), 0i32);
    while !r.done() {
        let (field, wire) = r.key()?;
        match (field, wire) {
            (1, 2) => {
                // packed repeated int32
                let mut p = Reader::new(r.bytes()?);
                while !p.done() {
                    range.push(p.varint()? as i64);
                }
            }
            (1, 0) => range.push(r.varint()? as i64),
            // typed_range (newer indexers): SingleLineRange{line, start, end} = 8,
            // MultiLineRange{start_line, start_char, end_line, end_char} = 9.
            // Missing proto3 fields are 0.
            (8 | 9, 2) => {
                let mut t = Reader::new(r.bytes()?);
                let (mut line, mut start) = (0i64, 0i64);
                while !t.done() {
                    let (f, w) = t.key()?;
                    match (f, w) {
                        (1, 0) => line = t.varint()? as i64,
                        (2, 0) => start = t.varint()? as i64,
                        _ => t.skip(w)?,
                    }
                }
                range = vec![line, start, start];
            }
            (2, 2) => symbol = string(r.bytes()?),
            (3, 0) => roles = r.varint()? as i32,
            _ => r.skip(wire)?,
        }
    }
    // range is [startLine, startChar, endChar] or [startLine, startChar, endLine, endChar]
    if range.len() < 3 || symbol.is_empty() {
        return Ok(None);
    }
    Ok(Some(Occurrence {
        line: range[0] as u32,
        start: range[1] as u32,
        symbol,
        roles,
    }))
}

// ------------------------------------------------------------ symbols

/// `local N` symbols are document-scoped (variables, closures); never a
/// cross-file definition.
fn is_local(symbol: &str) -> bool {
    symbol.starts_with("local ")
}

/// The name of a SCIP symbol's last descriptor: `...bytesconv`/StringToBytes().`
/// gives `StringToBytes`, `...Engine#handleHTTPRequest().` gives
/// `handleHTTPRequest`, `...format(+1).` gives `format`, and a backtick-escaped
/// name is unescaped.
pub fn descriptor_name(symbol: &str) -> Option<String> {
    let mut s = symbol.trim_end();
    // descriptor suffix: `.` term/method, `#` type, `/` namespace, `:` meta, `!` macro
    s = s.strip_suffix(['.', '#', '/', ':', '!'])?;
    // method disambiguator `(...)`
    if s.ends_with(')') {
        let open = s.rfind('(')?;
        s = &s[..open];
    }
    if let Some(inner) = s.strip_suffix('`') {
        // find the opening backtick, skipping escaped (doubled) ones
        let b = inner.as_bytes();
        let mut i = b.len();
        while i > 0 {
            i -= 1;
            if b[i] == b'`' {
                if i > 0 && b[i - 1] == b'`' {
                    i -= 1;
                    continue;
                }
                return Some(inner[i + 1..].replace("``", "`"));
            }
        }
        return None;
    }
    let start = s
        .rfind(['/', '#', '.', ' ', ':', '`', '!', ')', ']'])
        .map_or(0, |i| i + 1);
    let name = &s[start..];
    (!name.is_empty()).then(|| name.to_string())
}

/// `(name, version)` of a symbol's package (`scheme manager name version descriptors`).
fn package(symbol: &str) -> Option<(&str, &str)> {
    let mut it = symbol.splitn(5, ' ');
    let (_scheme, _manager, name, version) = (it.next()?, it.next()?, it.next()?, it.next()?);
    it.next()?;
    Some((name, version))
}

/// The bare callee name of an edge target (`path::to::f`, `obj.f`, `f`).
fn bare(target_name: &str) -> &str {
    let t = target_name.rsplit("::").next().unwrap_or(target_name);
    t.rsplit('.').next().unwrap_or(t)
}

// ------------------------------------------------------------ import

#[derive(Debug, Default, Clone, Serialize)]
pub struct ScipReport {
    /// Documents in the SCIP index.
    pub documents: usize,
    /// Call edges ctx has with a location.
    pub call_edges: usize,
    /// Edges SCIP answered (bound, rebound, unbound or confirmed).
    pub answered: usize,
    pub bound: usize,
    pub rebound: usize,
    pub unbound_external: usize,
    pub unchanged: usize,
    /// The calling file is not in the SCIP index.
    pub file_not_indexed: usize,
    /// No reference with the callee's name at the call site.
    pub no_reference: usize,
    /// Defined in the repository, but not inside a ctx function/method of that name.
    pub unmapped_definition: usize,
    /// Documents whose file is not in the ctx index (paths differ, or excluded).
    pub unknown_documents: usize,
    /// Call edges left unchanged because the reference points into a package
    /// with the same name as one defined in this index but another version:
    /// usually the project is also installed (site-packages, node_modules) and
    /// the indexer resolved imports to that copy instead of the repository.
    pub shadowed_package_calls: usize,
    /// The shadowing package, e.g. `Flask 3.1.3` (first one seen).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_package: Option<String>,
    pub seconds: f64,
}

/// References on each 0-based line of a document: (start character, symbol).
type LineRefs<'a> = HashMap<u32, Vec<(u32, &'a str)>>;

/// Options for [`import`].
#[derive(Debug, Clone, Default)]
pub struct ScipOptions {
    pub dry_run: bool,
}

/// What SCIP says about one call edge.
#[derive(Debug, Clone, PartialEq)]
enum Answer {
    /// Bound to this in-repo definition (file, 1-based line).
    InRepo(String, u32),
    External,
}

/// Import a SCIP index file into the ctx index at `db`.
pub fn import(db: &Database, index_path: &Path, opts: &ScipOptions) -> Result<ScipReport> {
    let t0 = Instant::now();
    let bytes = std::fs::read(index_path)
        .map_err(|e| CtxError::Other(format!("cannot read {}: {e}", index_path.display())))?;
    let docs = decode_index(&bytes)?;
    let mut report = ScipReport {
        documents: docs.len(),
        ..Default::default()
    };
    let hashes = db.file_hashes()?;
    report.unknown_documents = docs
        .iter()
        .filter(|d| !hashes.contains_key(&d.relative_path))
        .count();

    // symbol -> (file, 0-based line) of its definition
    let mut defs: HashMap<&str, (&str, u32)> = HashMap::new();
    for d in &docs {
        for o in &d.occurrences {
            if o.roles & ROLE_DEFINITION != 0 && !is_local(&o.symbol) {
                defs.entry(o.symbol.as_str())
                    .or_insert((d.relative_path.as_str(), o.line));
            }
        }
    }
    // packages that have definitions in this index: name (lowercased) -> version
    let mut own: HashMap<String, &str> = HashMap::new();
    for sym in defs.keys() {
        if let Some((name, version)) = package(sym) {
            own.entry(name.to_lowercase()).or_insert(version);
        }
    }
    // file -> line -> references on that line (start char, symbol)
    let mut refs: HashMap<&str, LineRefs> = HashMap::new();
    for d in &docs {
        let lines = refs.entry(d.relative_path.as_str()).or_default();
        for o in &d.occurrences {
            if o.roles & ROLE_DEFINITION == 0 && !is_local(&o.symbol) {
                lines
                    .entry(o.line)
                    .or_default()
                    .push((o.start, o.symbol.as_str()));
            }
        }
    }

    let spans = db.callable_spans()?;
    let edges = db.call_edges_for_judging()?;
    let extra_lines = db.call_edge_extra_lines()?;
    report.call_edges = edges.len();
    let mut cache: Vec<ScipEdge> = Vec::new();
    let mut answered_ids: Vec<i64> = Vec::new();
    let mut changes: Vec<(i64, Option<String>)> = Vec::new();

    for e in &edges {
        let Some(lines) = refs.get(e.source_file.as_str()) else {
            report.file_not_indexed += 1;
            continue;
        };
        let name = bare(&e.target_name);
        let Some(first) = e.line.checked_sub(1) else {
            report.no_reference += 1;
            continue;
        };
        // The edge's position is the start of the call expression; the callee
        // identifier is the first reference with its name at or after it. In a
        // multi-line expression (`Router::new()\n    .route(..)`) it can be on a
        // later line of the expression.
        let last = first + extra_lines.get(&e.edge_id).copied().unwrap_or(0);
        let pick = (first..=last).find_map(|l| {
            lines.get(&l).and_then(|refs| {
                refs.iter()
                    .filter(|(start, s)| {
                        (l > first || *start >= e.col)
                            && descriptor_name(s).as_deref() == Some(name)
                    })
                    .min_by_key(|(start, _)| *start)
            })
        });
        let Some((_, symbol)) = pick else {
            report.no_reference += 1;
            continue;
        };
        let answer = match defs.get(symbol) {
            None => {
                // A reference into another version of the project's own package
                // is not evidence that the call leaves the repository: leave it.
                if let Some((name, version)) = package(symbol) {
                    if own.get(&name.to_lowercase()).is_some_and(|v| *v != version) {
                        report.shadowed_package_calls += 1;
                        report
                            .shadowed_package
                            .get_or_insert_with(|| format!("{name} {version}"));
                        continue;
                    }
                }
                Answer::External
            }
            Some((file, line0)) => Answer::InRepo(file.to_string(), line0 + 1),
        };
        let target = match &answer {
            Answer::External => None,
            Answer::InRepo(file, line) => match containing(&spans, file, *line, name) {
                Some(s) => Some(s.id.clone()),
                None => {
                    report.unmapped_definition += 1;
                    continue;
                }
            },
        };
        report.answered += 1;
        match (&e.target_id, &target) {
            (a, b) if a == b => report.unchanged += 1,
            (None, Some(_)) => report.bound += 1,
            (Some(_), Some(_)) => report.rebound += 1,
            (Some(_), None) => report.unbound_external += 1,
            (None, None) => {}
        }
        if e.target_id != target {
            changes.push((e.edge_id, target.clone()));
        }
        answered_ids.push(e.edge_id);
        let (target_file, target_line) = match answer {
            Answer::InRepo(f, l) => (Some(f), Some(l)),
            Answer::External => (None, None),
        };
        cache.push(ScipEdge {
            file_path: e.source_file.clone(),
            file_hash: hashes.get(&e.source_file).cloned().unwrap_or_default(),
            line: e.line,
            col: e.col,
            target_name: e.target_name.clone(),
            target_file_hash: target_file.as_ref().and_then(|f| hashes.get(f).cloned()),
            target_file,
            target_line,
        });
    }

    if !opts.dry_run {
        db.in_transaction(|| {
            for (id, target) in &changes {
                db.retarget_edge(*id, target.as_deref())?;
            }
            db.set_edge_provenance(&answered_ids, PROVENANCE_SCIP)?;
            db.replace_scip_edges(&cache)?;
            Ok(())
        })?;
    }
    report.seconds = t0.elapsed().as_secs_f64();
    Ok(report)
}

/// The innermost function/method named `name` in `file` whose span contains
/// `line` (1-based); falls back to one starting on `line` whatever its name
/// (constructors, `impl` blocks reported under another name).
fn containing<'a>(
    spans: &'a [CallableSpan],
    file: &str,
    line: u32,
    name: &str,
) -> Option<&'a CallableSpan> {
    let inside = |s: &&CallableSpan| {
        s.file_path == file && s.line_start <= line && line <= s.line_end.max(s.line_start)
    };
    spans
        .iter()
        .filter(inside)
        .filter(|s| s.name == name)
        .min_by_key(|s| s.line_end.saturating_sub(s.line_start))
        .or_else(|| spans.iter().filter(inside).find(|s| s.line_start == line))
}

/// Re-apply cached SCIP answers after a re-index, for edges whose calling file
/// and target file still have the hashes they had at import. Returns the number
/// of edges answered. No-op when no import has run.
pub fn replay(db: &Database) -> Result<usize> {
    let cached = db.scip_edges()?;
    if cached.is_empty() {
        return Ok(0);
    }
    let hashes = db.file_hashes()?;
    let spans = db.callable_spans()?;
    let key = |f: &str, l: u32, c: u32, n: &str| (f.to_string(), l, c, n.to_string());
    let fresh: HashMap<_, &ScipEdge> = cached
        .iter()
        .filter(|c| hashes.get(&c.file_path) == Some(&c.file_hash))
        .filter(|c| match (&c.target_file, &c.target_file_hash) {
            (Some(f), Some(h)) => hashes.get(f) == Some(h),
            (None, _) => true,
            (Some(_), None) => false,
        })
        .map(|c| (key(&c.file_path, c.line, c.col, &c.target_name), c))
        .collect();
    let mut answered = Vec::new();
    let mut changes = Vec::new();
    for e in db.call_edges_for_judging()? {
        let Some(c) = fresh.get(&key(&e.source_file, e.line, e.col, &e.target_name)) else {
            continue;
        };
        let target = match (&c.target_file, c.target_line) {
            (Some(f), Some(l)) => match containing(&spans, f, l, bare(&c.target_name)) {
                Some(s) => Some(s.id.clone()),
                None => continue,
            },
            _ => None,
        };
        if e.target_id != target {
            changes.push((e.edge_id, target));
        }
        answered.push(e.edge_id);
    }
    db.in_transaction(|| {
        for (id, target) in &changes {
            db.retarget_edge(*id, target.as_deref())?;
        }
        db.set_edge_provenance(&answered, PROVENANCE_SCIP)?;
        Ok(())
    })?;
    Ok(answered.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn field_bytes(field: u32, data: &[u8], out: &mut Vec<u8>) {
        varint(u64::from(field << 3 | 2), out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    pub(crate) fn occurrence(range: &[u32], symbol: &str, roles: i32) -> Vec<u8> {
        let mut packed = Vec::new();
        for r in range {
            varint(u64::from(*r), &mut packed);
        }
        let mut o = Vec::new();
        field_bytes(1, &packed, &mut o);
        field_bytes(2, symbol.as_bytes(), &mut o);
        if roles != 0 {
            varint(3 << 3, &mut o);
            varint(roles as u64, &mut o);
        }
        o
    }

    pub(crate) fn document(path: &str, occs: &[Vec<u8>]) -> Vec<u8> {
        let mut d = Vec::new();
        field_bytes(1, path.as_bytes(), &mut d);
        for o in occs {
            field_bytes(2, o, &mut d);
        }
        d
    }

    pub(crate) fn index(docs: &[Vec<u8>]) -> Vec<u8> {
        let mut i = Vec::new();
        field_bytes(1, b"\x0a\x00", &mut i); // metadata, skipped
        for d in docs {
            field_bytes(2, d, &mut i);
        }
        i
    }

    #[test]
    fn decodes_documents_and_occurrences() {
        let buf = index(&[document(
            "a.go",
            &[
                occurrence(&[3, 5, 12], "scip-go gomod x v1 `x`/F().", 1),
                occurrence(&[9, 2, 4, 1], "scip-go gomod x v1 `x`/G().", 0),
            ],
        )]);
        let docs = decode_index(&buf).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].relative_path, "a.go");
        assert_eq!(docs[0].occurrences[0].line, 3);
        assert_eq!(docs[0].occurrences[0].roles, 1);
        assert_eq!(docs[0].occurrences[1].start, 2);
    }

    #[test]
    fn decodes_typed_single_line_ranges() {
        // Occurrence{symbol=2, roles=3, single_line_range=8 {line=14, start=8, end=13}}
        let mut o = Vec::new();
        field_bytes(2, b"scip-go gomod x v1 `x`/F().", &mut o);
        field_bytes(8, &[0x08, 14, 0x10, 8, 0x18, 13], &mut o);
        let docs = decode_index(&index(&[document("a.go", &[o])])).unwrap();
        assert_eq!(
            (docs[0].occurrences[0].line, docs[0].occurrences[0].start),
            (14, 8)
        );
        // line 0 is omitted on the wire in proto3
        let mut o = Vec::new();
        field_bytes(2, b"s", &mut o);
        field_bytes(8, &[0x10, 3, 0x18, 5], &mut o);
        let docs = decode_index(&index(&[document("a.go", &[o])])).unwrap();
        assert_eq!(
            (docs[0].occurrences[0].line, docs[0].occurrences[0].start),
            (0, 3)
        );
    }

    #[test]
    fn rejects_truncated_input() {
        let mut buf = index(&[document("a.go", &[occurrence(&[1, 2, 3], "s", 0)])]);
        buf.truncate(buf.len() - 3);
        assert!(decode_index(&buf).is_err());
    }

    #[test]
    fn descriptor_names() {
        let cases = [
            ("scip-go gomod github.com/gin-gonic/gin v1 `github.com/gin-gonic/gin/internal/bytesconv`/StringToBytes().", "StringToBytes"),
            ("scip-go gomod github.com/gin-gonic/gin v1 `github.com/gin-gonic/gin`/Engine#handleHTTPRequest().", "handleHTTPRequest"),
            ("scip-python python flask 3.1 `flask.app`/Flask#run().", "run"),
            ("scip-typescript npm hono 4 src/`hono-base.ts`/Hono#fetch.", "fetch"),
            ("cxx . . $ fmt/v11/format(+1).", "format"),
            ("rust-analyzer cargo ctx 0.4.0 db/schema/Database#open().", "open"),
            ("scip-typescript npm x 1 `weird``name`().", "weird`name"),
            ("rust-analyzer cargo cookie 0.18.1 impl#[`Cookie<'c>`]parse_encoded().", "parse_encoded"),
            ("rust-analyzer cargo alloc https://github.com/rust-lang/rust/library/alloc str/impl#[str][ToOwned]to_owned().", "to_owned"),
        ];
        for (sym, want) in cases {
            assert_eq!(descriptor_name(sym).as_deref(), Some(want), "{sym}");
        }
        assert_eq!(bare("path::to::run"), "run");
        assert_eq!(bare("self.run"), "run");
    }

    // Two Rust files; `run` calls `helper()` (in-repo, same name as a decoy in
    // b.rs) and `len()` (external). The name resolver cannot tell the two
    // `helper`s apart; the SCIP index says a.rs's call goes to b.rs::helper.
    const A: &str = "pub fn helper() {}\npub fn run() {\n    helper();\n    v.len();\n}\n";
    const B: &str = "pub fn helper() {}\npub fn len() {}\n";

    fn fixture() -> (tempfile::TempDir, crate::index::Indexer) {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), A).unwrap();
        std::fs::write(dir.path().join("src/b.rs"), B).unwrap();
        let mut ix = crate::index::Indexer::with_config(
            dir.path(),
            false,
            crate::walker::WalkerConfig::default(),
        )
        .unwrap();
        ix.index().unwrap();
        (dir, ix)
    }

    fn scip_file(dir: &Path) -> std::path::PathBuf {
        let helper_b = "rust-analyzer cargo t 0.1 b/helper().";
        let len_ext = "rust-analyzer cargo core 1 slice/impl#[`[T]`]len().";
        let buf = index(&[
            document(
                "src/a.rs",
                &[
                    occurrence(&[0, 7, 13], "rust-analyzer cargo t 0.1 a/helper().", 1),
                    occurrence(&[2, 4, 10], helper_b, 0),
                    occurrence(&[3, 6, 9], len_ext, 0),
                ],
            ),
            document("src/b.rs", &[occurrence(&[0, 7, 13], helper_b, 1)]),
        ]);
        let p = dir.join("index.scip");
        std::fs::write(&p, buf).unwrap();
        p
    }

    fn target_of(db: &Database, name: &str) -> Option<String> {
        db.call_edges_for_judging()
            .unwrap()
            .into_iter()
            .find(|e| e.source_file == "src/a.rs" && bare(&e.target_name) == name)
            .unwrap_or_else(|| panic!("no call to {name}"))
            .target_id
    }

    #[test]
    fn import_binds_unbinds_and_marks_provenance() {
        let (dir, ix) = fixture();
        let db = ix.database();
        let path = scip_file(dir.path());
        let r = import(db, &path, &ScipOptions::default()).unwrap();
        assert_eq!(r.answered, 2, "{r:?}");
        let helper = target_of(db, "helper").expect("bound");
        assert!(helper.starts_with("src/b.rs::"), "{helper}");
        assert_eq!(target_of(db, "len"), None, "external call is unbound");
        assert_eq!(
            db.edges_with_provenance(crate::db::PROVENANCE_SCIP)
                .unwrap()
                .len(),
            2
        );
        let counts = db.call_edge_provenance_counts().unwrap();
        assert!(
            counts.iter().any(|(s, n)| s == "scip" && *n == 2),
            "{counts:?}"
        );
    }

    #[test]
    fn calls_into_an_installed_copy_of_the_project_are_left_alone() {
        let (dir, ix) = fixture();
        let db = ix.database();
        let before = target_of(db, "helper");
        // the call resolves to `T 9.9` (an installed copy), while the index defines `t 0.1`
        let buf = index(&[document(
            "src/a.rs",
            &[
                occurrence(&[0, 7, 13], "rust-analyzer cargo t 0.1 a/helper().", 1),
                occurrence(&[2, 4, 10], "rust-analyzer cargo T 9.9 b/helper().", 0),
            ],
        )]);
        let p = dir.path().join("shadow.scip");
        std::fs::write(&p, buf).unwrap();
        let r = import(db, &p, &ScipOptions::default()).unwrap();
        assert_eq!(r.shadowed_package_calls, 1);
        assert_eq!(r.shadowed_package.as_deref(), Some("T 9.9"));
        assert_eq!(target_of(db, "helper"), before, "edge untouched");
    }

    #[test]
    fn dry_run_changes_nothing() {
        let (dir, ix) = fixture();
        let db = ix.database();
        let before = target_of(db, "len");
        let r = import(db, &scip_file(dir.path()), &ScipOptions { dry_run: true }).unwrap();
        assert_eq!(r.answered, 2);
        assert_eq!(target_of(db, "len"), before);
        assert!(db
            .edges_with_provenance(crate::db::PROVENANCE_SCIP)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn replay_keeps_answers_for_unchanged_files_only() {
        let (dir, mut ix) = fixture();
        import(
            ix.database(),
            &scip_file(dir.path()),
            &ScipOptions::default(),
        )
        .unwrap();
        // re-index without changes: the resolver may rebind; replay restores SCIP's answers
        ix.index().unwrap();
        assert_eq!(replay(ix.database()).unwrap(), 2);
        assert!(target_of(ix.database(), "helper")
            .unwrap()
            .starts_with("src/b.rs::"));
        assert_eq!(target_of(ix.database(), "len"), None);
        // edit the calling file: its cached answers no longer apply
        std::fs::write(dir.path().join("src/a.rs"), format!("{A}// edited\n")).unwrap();
        ix.index().unwrap();
        assert_eq!(replay(ix.database()).unwrap(), 0);
        assert!(ix
            .database()
            .edges_with_provenance(crate::db::PROVENANCE_SCIP)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn judge_skips_edges_scip_answered() {
        let (dir, ix) = fixture();
        let opts = crate::judge::JudgeOptions {
            offline: true,
            ..Default::default()
        };
        let before = crate::judge::judge_edges(dir.path(), ix.database(), &opts).unwrap();
        import(
            ix.database(),
            &scip_file(dir.path()),
            &ScipOptions::default(),
        )
        .unwrap();
        let after = crate::judge::judge_edges(dir.path(), ix.database(), &opts).unwrap();
        assert_eq!(after.skipped_precise, 2);
        assert!(
            after.considered < before.considered,
            "{before:?} -> {after:?}"
        );
    }
}
