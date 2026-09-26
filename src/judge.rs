//! Model-assisted judgments over the index (opt-in).
//!
//! The first judgment is **edge resolution**: tree-sitter name matching binds a
//! call such as `map.get(k)` to whatever in-repo function happens to be called
//! `get`, and leaves calls it cannot disambiguate unresolved. For every call
//! edge whose bare name matches at least one in-repo function or method, ask a
//! decision model (TypeSafe Jev) to choose the callee among those candidates or
//! `external` (standard library / third-party crate), and rewrite the edge.
//!
//! Design constraints (they mirror the rest of ctx):
//! - **Opt-in and explicit.** Nothing runs unless `ctx judge edges` is invoked or
//!   `[judge] edges = true` is set; the API key only comes from `JEV_API_KEY`
//!   (never from a committed config file).
//! - **Cached, deterministic replay.** Every answer is stored in the
//!   `judgments` table keyed by a hash of exactly what the model was shown, so
//!   re-indexing re-applies answers for unchanged code without a network call,
//!   and `--offline` applies only cached answers.
//! - **Conservative.** Answers below `min_confidence` leave the edge as the
//!   heuristic resolver left it.
//! - **What leaves the machine**: the calling function's source (≤ 3 000 chars),
//!   its file's `use`/`import` lines, the call-site line and the candidates'
//!   names, paths and signatures.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::db::{Database, JudgeCandidate, JudgeEdge};
use crate::error::{CtxError, Result};

pub const DEFAULT_URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const DEFAULT_MODEL: &str = "jev-latest";
const QUESTION_ID: &str = "edge_callee.v1";
const MAX_CANDIDATES: usize = 30;
const MAX_CALLER_SOURCE: usize = 3000;

/// Options for [`judge_edges`].
#[derive(Debug, Clone)]
pub struct JudgeOptions {
    /// Apply cached answers only; never call the model.
    pub offline: bool,
    /// Ask but do not rewrite edges.
    pub dry_run: bool,
    /// Stop after this many *new* model calls (0 = no limit).
    pub limit: usize,
    /// Answers below this confidence leave the edge unchanged.
    pub min_confidence: f64,
    /// Parallel requests.
    pub concurrency: usize,
    pub model: String,
    pub url: String,
    pub verbose: bool,
}

impl Default for JudgeOptions {
    fn default() -> Self {
        Self {
            offline: false,
            dry_run: false,
            limit: 0,
            min_confidence: 0.7,
            concurrency: 8,
            model: DEFAULT_MODEL.into(),
            url: DEFAULT_URL.into(),
            verbose: false,
        }
    }
}

/// What a run did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct JudgeReport {
    pub considered: usize,
    pub cached: usize,
    pub asked: usize,
    pub failed: usize,
    pub skipped_uncached: usize,
    pub low_confidence: usize,
    pub bound: usize,
    pub rebound: usize,
    pub unbound_external: usize,
    pub unchanged: usize,
    pub input_tokens: u64,
    pub seconds: f64,
}

/// One prepared question: what we send, and how to map the answer back.
struct Prepared {
    edge: JudgeEdge,
    key: String,
    subject: String,
    body: Value,
    candidates: Vec<String>,
}

/// Deterministic, truth-blind candidate order: same file, same directory,
/// file stem mentioned in the imports, then the rest (stable by path/id).
fn rank_candidates<'a>(
    src_file: &str,
    cands: &[&'a JudgeCandidate],
    imports: &str,
) -> Vec<&'a JudgeCandidate> {
    let dir = |p: &str| {
        p.rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default()
    };
    let src_dir = dir(src_file);
    let mut v: Vec<&JudgeCandidate> = cands.to_vec();
    // C/C++: a header prototype and its definition are the same function for
    // a caller. Offering both splits the model's confidence between two
    // equivalent answers, so drop the prototype when a definition is present.
    let has_definition = |name: &str| {
        cands
            .iter()
            .any(|c| c.name == name && is_c_family(&c.file_path) && !is_c_header(&c.file_path))
    };
    v.retain(|c| !(is_c_header(&c.file_path) && has_definition(&c.name)));
    v.sort_by_key(|c| {
        let stem = c
            .file_path
            .rsplit('/')
            .next()
            .unwrap_or("")
            .split('.')
            .next()
            .unwrap_or("")
            .to_string();
        let tier = if c.file_path == src_file {
            0
        } else if dir(&c.file_path) == src_dir {
            1
        } else if !stem.is_empty() && imports.contains(&stem) {
            2
        } else {
            3
        };
        (tier, c.file_path.clone(), c.id.clone())
    });
    v.truncate(MAX_CANDIDATES);
    v
}

fn is_c_header(path: &str) -> bool {
    [".h", ".hh", ".hpp", ".hxx"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

fn is_c_family(path: &str) -> bool {
    is_c_header(path)
        || [".c", ".cc", ".cpp", ".cxx"]
            .iter()
            .any(|ext| path.ends_with(ext))
}

fn import_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("use ")
                || l.starts_with("pub use ")
                || l.starts_with("mod ")
                || l.starts_with("pub mod ")
                || l.starts_with("import ")
                || l.starts_with("from ")
                || l.starts_with("#include")
        })
        .take(40)
        .map(str::to_string)
        .collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn instructions(bare: &str, rust: bool) -> String {
    if rust {
        format!(
            "Which definition does the call to `{bare}` on the call-site line resolve to? Consider Rust method \
             resolution: `x.{bare}(..)` calls a method on the type of `x`; `{bare}(..)` or `path::{bare}(..)` \
             calls a function in scope via the use declarations."
        )
    } else {
        format!(
            "Which definition does the call to `{bare}` on the call-site line resolve to? Consider the \
             language's scoping rules and the import declarations; `x.{bare}(..)` calls a method on the object `x`."
        )
    }
}

fn prepare(
    db: &Database,
    root: &Path,
    edges: Vec<JudgeEdge>,
    by_name: &HashMap<&str, Vec<&JudgeCandidate>>,
    model: &str,
) -> Result<Vec<Prepared>> {
    let mut files: HashMap<String, Option<String>> = HashMap::new();
    let mut sources: HashMap<String, Option<String>> = HashMap::new();
    let mut out = Vec::new();
    for edge in edges {
        let bare = edge
            .target_name
            .rsplit("::")
            .next()
            .unwrap_or(&edge.target_name)
            .to_string();
        let Some(cands) = by_name.get(bare.as_str()) else {
            continue;
        };
        let text = files
            .entry(edge.source_file.clone())
            .or_insert_with(|| std::fs::read_to_string(root.join(&edge.source_file)).ok());
        let Some(text) = text.as_deref() else {
            continue;
        };
        let imports = import_lines(text);
        let joined = imports.join(" ");
        let ranked = rank_candidates(&edge.source_file, cands, &joined);
        let line = text
            .lines()
            .nth(edge.line.saturating_sub(1) as usize)
            .unwrap_or("")
            .trim();
        let src = sources
            .entry(edge.source_id.clone())
            .or_insert_with(|| db.symbol_source(&edge.source_id).ok().flatten());
        let caller = edge
            .source_id
            .split_once("::")
            .map(|(_, r)| r)
            .unwrap_or(&edge.source_id);
        let caller = caller.rsplit_once('@').map(|(n, _)| n).unwrap_or(caller);

        let mut criteria = Map::new();
        for (i, c) in ranked.iter().enumerate() {
            criteria.insert(
                format!("c{i}"),
                Value::String(format!(
                    "{} {} in {} :: {}",
                    c.kind,
                    c.qualified_name.as_deref().unwrap_or(&c.name),
                    c.file_path,
                    truncate(c.signature.as_deref().unwrap_or(""), 160)
                )),
            );
        }
        criteria.insert(
            "external".into(),
            Value::String(
                "a function or method from the standard library or a third-party dependency (not defined in this repository)".into(),
            ),
        );
        let state = json!({
            "caller_file": edge.source_file,
            "caller": caller,
            "use_declarations": imports,
            "call_site_line": truncate(line, 300),
            "called_name": edge.target_name,
            "caller_source": truncate(src.as_deref().unwrap_or(""), MAX_CALLER_SOURCE),
        });
        let questions = json!({
            "callee": {"type": "choice", "instructions": instructions(&bare, edge.source_file.ends_with(".rs")), "criteria": criteria}
        });
        let body = json!({"model": model, "state": state, "questions": questions});
        let mut h = Sha256::new();
        h.update(QUESTION_ID.as_bytes());
        h.update(serde_json::to_vec(&body)?);
        let key: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let subject = format!("edge:{}:{}:{}", edge.source_id, edge.line, edge.target_name);
        out.push(Prepared {
            edge,
            key,
            subject,
            body,
            candidates: ranked.iter().map(|c| c.id.clone()).collect(),
        });
    }
    Ok(out)
}

struct Answer {
    choice: String,
    confidence: f64,
    probabilities: Option<String>,
    model_version: String,
    input_tokens: u64,
}

fn ask(client: &reqwest::blocking::Client, url: &str, key: &str, body: &Value) -> Result<Answer> {
    let mut last = String::new();
    for attempt in 0..4u64 {
        let resp = client.post(url).bearer_auth(key).json(body).send();
        match resp {
            Ok(r) if r.status().is_success() => {
                let v: Value = r.json()?;
                let a = &v["answers"]["callee"];
                let choice = a["choice"]
                    .as_str()
                    .ok_or_else(|| CtxError::Other("judge: response without a choice".into()))?;
                return Ok(Answer {
                    choice: choice.to_string(),
                    confidence: a["confidence"].as_f64().unwrap_or(0.0),
                    probabilities: a.get("probabilities").map(|p| p.to_string()),
                    model_version: v["model"].as_str().unwrap_or("unknown").to_string(),
                    input_tokens: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
                });
            }
            Ok(r) => {
                let status = r.status();
                last = format!("HTTP {status}");
                if !(status.as_u16() == 429 || status.is_server_error()) {
                    break;
                }
            }
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(750 * (attempt + 1)));
    }
    Err(CtxError::Other(format!("judge: request failed: {last}")))
}

/// Resolve ambiguous call edges with the decision model; see module docs.
pub fn judge_edges(root: &Path, db: &Database, opts: &JudgeOptions) -> Result<JudgeReport> {
    let t0 = Instant::now();
    db.ensure_judgments_table()?;
    let all = db.callable_candidates()?;
    let mut by_name: HashMap<&str, Vec<&JudgeCandidate>> = HashMap::new();
    for c in &all {
        by_name.entry(c.name.as_str()).or_default().push(c);
    }
    let edges = db.call_edges_for_judging()?;
    let prepared = prepare(db, root, edges, &by_name, &opts.model)?;
    let mut report = JudgeReport {
        considered: prepared.len(),
        ..Default::default()
    };

    // 1. cached answers
    let mut answers: HashMap<usize, (String, f64)> = HashMap::new();
    let mut todo: Vec<usize> = Vec::new();
    for (i, p) in prepared.iter().enumerate() {
        match db.get_judgment(&p.key)? {
            Some(j) => {
                report.cached += 1;
                answers.insert(i, (j.answer, j.p));
            }
            None => todo.push(i),
        }
    }
    if opts.offline {
        report.skipped_uncached = todo.len();
        todo.clear();
    }
    if opts.limit > 0 && todo.len() > opts.limit {
        report.skipped_uncached = todo.len() - opts.limit;
        todo.truncate(opts.limit);
    }

    // 2. ask the model for the rest (parallel), persisting as answers arrive
    if !todo.is_empty() {
        let key = std::env::var("JEV_API_KEY").map_err(|_| {
            CtxError::Other(
                "judge: JEV_API_KEY is not set (use --offline to apply cached answers only)".into(),
            )
        })?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        let (tx, rx) = mpsc::channel::<(usize, Result<Answer>)>();
        let queue = std::sync::Mutex::new(todo.clone().into_iter());
        std::thread::scope(|s| {
            for _ in 0..opts.concurrency.max(1) {
                let tx = tx.clone();
                let (queue, client, key, prepared) = (&queue, &client, &key, &prepared);
                s.spawn(move || loop {
                    let next = queue.lock().unwrap().next();
                    let Some(i) = next else { break };
                    let r = ask(client, &opts.url, key, &prepared[i].body);
                    if tx.send((i, r)).is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let mut done = 0usize;
            for (i, r) in rx {
                done += 1;
                match r {
                    Ok(a) => {
                        report.asked += 1;
                        report.input_tokens += a.input_tokens;
                        let p = &prepared[i];
                        if let Err(e) = db.put_judgment(
                            &p.key,
                            &p.subject,
                            QUESTION_ID,
                            &a.model_version,
                            &a.choice,
                            a.confidence,
                            a.probabilities.as_deref(),
                            now,
                        ) {
                            eprintln!("Warning: judge: could not cache answer: {e}");
                        }
                        answers.insert(i, (a.choice, a.confidence));
                    }
                    Err(e) => {
                        report.failed += 1;
                        if opts.verbose {
                            eprintln!("Warning: {e}");
                        }
                    }
                }
                if opts.verbose && done.is_multiple_of(250) {
                    eprintln!("judge: {done}/{} asked", todo.len());
                }
            }
        });
    }

    // 3. apply, in one transaction
    let mut changes: Vec<(i64, Option<String>, Option<String>)> = Vec::new();
    for (i, p) in prepared.iter().enumerate() {
        let Some((choice, conf)) = answers.get(&i) else {
            continue;
        };
        if *conf < opts.min_confidence {
            report.low_confidence += 1;
            continue;
        }
        let new_target = if choice == "external" {
            None
        } else if let Some(idx) = choice
            .strip_prefix('c')
            .and_then(|n| n.parse::<usize>().ok())
        {
            match p.candidates.get(idx) {
                Some(id) => Some(id.clone()),
                None => continue,
            }
        } else {
            continue;
        };
        if new_target == p.edge.target_id {
            report.unchanged += 1;
            continue;
        }
        match (&p.edge.target_id, &new_target) {
            (None, Some(_)) => report.bound += 1,
            (Some(_), Some(_)) => report.rebound += 1,
            (Some(_), None) => report.unbound_external += 1,
            (None, None) => {}
        }
        changes.push((p.edge.edge_id, p.edge.target_id.clone(), new_target));
    }
    if !opts.dry_run {
        db.in_transaction(|| {
            for (edge_id, _, target) in &changes {
                db.retarget_edge(*edge_id, target.as_deref())?;
            }
            Ok(())
        })?;
    }
    report.seconds = t0.elapsed().as_secs_f64();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, file: &str) -> JudgeCandidate {
        JudgeCandidate {
            id: id.into(),
            name: "get".into(),
            qualified_name: None,
            kind: "function".into(),
            file_path: file.into(),
            signature: None,
        }
    }

    #[test]
    fn candidates_rank_same_file_then_dir_then_imports() {
        let a = cand("a", "src/x/other.rs");
        let b = cand("b", "src/y/util.rs");
        let c = cand("c", "src/x/here.rs");
        let d = cand("d", "src/z/zzz.rs");
        let v = [&d, &b, &a, &c];
        let ranked = rank_candidates("src/x/here.rs", &v, "use crate::y::util;");
        let ids: Vec<&str> = ranked.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b", "d"]);
    }

    #[test]
    fn c_header_prototype_is_dropped_when_definition_is_a_candidate() {
        let h = cand("h", "src/jv.h");
        let c = cand("c", "src/jv.c");
        let ranked = rank_candidates("src/builtin.c", &[&h, &c], "#include \"jv.h\"");
        let ids: Vec<&str> = ranked.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["c"]);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn import_lines_cover_rust_python_ts() {
        let t =
            "use std::fs;\nfn x() {}\nfrom a import b\nimport { c } from './c'\n#include <d.h>\n";
        assert_eq!(import_lines(t).len(), 4);
    }
}
