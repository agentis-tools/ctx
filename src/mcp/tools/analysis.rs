//! Analysis-related MCP tools.

use rmcp::model::{CallToolResult, ContentBlock, ErrorCode, Tool};
use serde_json::Value;
use std::path::Path;

use std::collections::{HashSet, VecDeque};

use super::{parse_params, prefer_exact, schema_for, CallGraphParams, SmartContextParams};
use crate::db::{Database, EdgeKind, Symbol, SymbolKind};
use crate::mcp::server::CtxServer;

/// Helper to create an internal error.
fn internal_error(msg: impl Into<String>) -> rmcp::ErrorData {
    rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// Create the get_callers tool definition.
pub fn get_callers_tool() -> Tool {
    Tool::new(
        "get_callers",
        "Find the functions and methods that call a given function or method, following \
         resolved call edges up to `depth` levels (default 3). Calls ctx could not bind \
         to a definition are listed separately as possible callers.",
        schema_for::<CallGraphParams>(),
    )
}

/// Create the get_callees tool definition.
pub fn get_callees_tool() -> Tool {
    Tool::new(
        "get_callees",
        "List the calls a function or method makes and the definition each call \
         resolves to (or 'unresolved or external').",
        schema_for::<CallGraphParams>(),
    )
}

/// Create the smart_context tool definition.
pub fn smart_context_tool() -> Tool {
    Tool::new(
        "smart_context",
        "Intelligently select relevant files for a given task using semantic search \
         and call graph analysis. Returns the most relevant code for implementing \
         a feature, fixing a bug, or understanding a concept.",
        schema_for::<SmartContextParams>(),
    )
}

/// At most this many same-named definitions are expanded in one answer.
const MAX_TARGETS: usize = 5;

/// Resolve a function name to its definitions: functions *and* methods, exact
/// name (or qualified-name suffix) matches first, substring matches only when
/// nothing matches exactly. `find_symbols_filtered` is a substring search, so
/// taking its first row answered `getValue` with `TestContextSetGetValues`.
fn resolve_functions(
    db: &Database,
    name: &str,
    file: Option<&str>,
) -> crate::error::Result<Vec<Symbol>> {
    let candidates: Vec<Symbol> = db
        .find_symbols_filtered(name, 200, file, None)?
        .into_iter()
        .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        .collect();
    Ok(prefer_exact(candidates, name))
}

fn describe(sym: &Symbol) -> String {
    format!(
        "{} [{}] ({}:{})",
        sym.qualified_name.as_deref().unwrap_or(&sym.name),
        sym.kind.as_str(),
        sym.file_path,
        sym.line_start
    )
}

/// Callers of one definition: breadth-first over *resolved* call edges
/// (`target_id` = the definition), so edges that indexing, the LSP or
/// `ctx judge edges` bound elsewhere are not reported here. Same-named calls
/// the resolver left unbound are listed separately as possible callers.
fn callers_of(db: &Database, sym: &Symbol, depth: u32) -> crate::error::Result<String> {
    let mut out = String::new();
    let mut visited = HashSet::from([sym.id.clone()]);
    let mut queue = VecDeque::from([(sym.id.clone(), 0_u32)]);
    let mut found = 0;
    while let Some((target, dist)) = queue.pop_front() {
        if dist >= depth {
            continue;
        }
        let mut edges: Vec<_> = db
            .get_incoming_edges(&target)?
            .into_iter()
            .filter(|e| {
                e.kind == EdgeKind::Calls && e.target_id.as_deref() == Some(target.as_str())
            })
            .collect();
        edges.sort_by(|a, b| a.source_id.cmp(&b.source_id).then(a.line.cmp(&b.line)));
        for edge in edges {
            if !visited.insert(edge.source_id.clone()) {
                continue;
            }
            if let Some(caller) = db.get_symbol(&edge.source_id)? {
                found += 1;
                out.push_str(&format!(
                    "{}- {} ({}:{})\n",
                    "  ".repeat(dist as usize),
                    caller.qualified_name.as_deref().unwrap_or(&caller.name),
                    caller.file_path,
                    edge.line.unwrap_or(caller.line_start)
                ));
                if let Some(ref c) = edge.context {
                    out.push_str(&format!(
                        "{}  Call: {}\n",
                        "  ".repeat(dist as usize),
                        c.trim()
                    ));
                }
                queue.push_back((caller.id.clone(), dist + 1));
            }
        }
    }
    if found == 0 {
        out.push_str("(no resolved callers)\n");
    }
    let mut unresolved: Vec<_> = db
        .get_incoming_edges(&sym.name)?
        .into_iter()
        .filter(|e| e.kind == EdgeKind::Calls && e.target_id.is_none() && e.target_name == sym.name)
        .collect();
    unresolved.sort_by(|a, b| a.source_id.cmp(&b.source_id).then(a.line.cmp(&b.line)));
    if !unresolved.is_empty() {
        out.push_str(&format!(
            "Possible callers (call by this name that ctx could not bind to a definition, {}):\n",
            unresolved.len()
        ));
        for edge in unresolved.iter().take(20) {
            if let Some(caller) = db.get_symbol(&edge.source_id)? {
                out.push_str(&format!(
                    "- {} ({}:{})\n",
                    caller.qualified_name.as_deref().unwrap_or(&caller.name),
                    caller.file_path,
                    edge.line.unwrap_or(caller.line_start)
                ));
            }
        }
    }
    Ok(out)
}

/// Calls made by one definition, with the definition each resolved to.
fn callees_of(db: &Database, sym: &Symbol) -> crate::error::Result<String> {
    let edges: Vec<_> = db
        .get_outgoing_edges(&sym.id)?
        .into_iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .collect();
    if edges.is_empty() {
        return Ok("(no calls)\n".into());
    }
    let mut out = String::new();
    for edge in &edges {
        let target = match edge.target_id.as_deref() {
            Some(id) => db.get_symbol(id)?.map(|t| describe(&t)),
            None => None,
        };
        out.push_str(&format!(
            "- {} (line {}) -> {}\n",
            edge.target_name,
            edge.line.unwrap_or(0),
            target.unwrap_or_else(|| "unresolved or external".into())
        ));
    }
    Ok(out)
}

fn call_graph_answer(
    server: &CtxServer,
    params: &CallGraphParams,
    per_target: impl Fn(&Database, &Symbol) -> crate::error::Result<String>,
    heading: &str,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let text = server
        .with_db(|db| -> crate::error::Result<String> {
            let targets = resolve_functions(db, &params.function, params.file.as_deref())?;
            if targets.is_empty() {
                return Ok(format!(
                    "Function or method '{}' not found",
                    params.function
                ));
            }
            let mut out = String::new();
            if targets.len() > 1 {
                out.push_str(&format!(
                    "{} definitions match '{}'{}; pass `file` to pick one.\n\n",
                    targets.len(),
                    params.function,
                    if targets.len() > MAX_TARGETS {
                        format!(" (showing {MAX_TARGETS})")
                    } else {
                        String::new()
                    }
                ));
            }
            for sym in targets.iter().take(MAX_TARGETS) {
                out.push_str(&format!("{heading} {}:\n", describe(sym)));
                out.push_str(&per_target(db, sym)?);
                out.push('\n');
            }
            Ok(out)
        })
        .map_err(|e| internal_error(e.to_string()))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

/// Execute the get_callers tool.
pub async fn get_callers(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: CallGraphParams = parse_params(args)?;
    let depth = u32::try_from(params.depth.unwrap_or(3))
        .unwrap_or(0)
        .clamp(1, 5);
    call_graph_answer(
        server,
        &params,
        |db, sym| callers_of(db, sym, depth),
        "Callers of",
    )
}

/// Execute the get_callees tool.
pub async fn get_callees(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let params: CallGraphParams = parse_params(args)?;
    call_graph_answer(server, &params, callees_of, "Calls made by")
}

/// Execute the smart_context tool.
pub async fn smart_context(
    server: &CtxServer,
    args: Option<&serde_json::Map<String, Value>>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    use crate::embeddings::local::LocalProvider;
    use crate::embeddings::ollama::OllamaProvider;
    use crate::embeddings::openai::OpenAIProvider;
    use crate::embeddings::{Embedding, EmbeddingProvider, Provider};
    use crate::smart::{smart_context_with_embedding_with_options, SmartConfig};
    use crate::tokens::Encoding;

    let params: SmartContextParams = parse_params(args)?;

    // Check if embeddings exist
    let embedding_count = server
        .with_db(|db| db.count_embeddings())
        .map_err(|e| internal_error(e.to_string()))?;

    if embedding_count == 0 {
        return Err(internal_error(
            "No embeddings found. Run 'ctx embed' first to generate embeddings.",
        ));
    }

    // Check if analytics is available
    let has_analytics = server.with_analytics(|_| ()).is_some();
    if !has_analytics {
        return Err(internal_error(
            "Analytics not available. Run 'ctx index' first.",
        ));
    }

    // Configure smart context
    let config = SmartConfig {
        max_tokens: params.max_tokens.unwrap_or(8000),
        depth: params.depth.unwrap_or(2),
        top: params.top.unwrap_or(10),
        encoding: Encoding::default(),
    };

    // Resolve provider: explicit `provider` string wins, else the deprecated
    // `use_openai` bool, else the `.ctx/config.toml` default, else local. Network
    // providers embed asynchronously so they don't block the async runtime.
    // (Named `project_config` so it doesn't shadow the `SmartConfig` above.)
    let project_config =
        crate::config::CtxConfig::load(&std::env::current_dir().unwrap_or_default());
    let provider = match params.provider.as_deref() {
        Some("openai") => Provider::Openai,
        Some("ollama") => Provider::Ollama,
        Some("local") => Provider::Local,
        None => Provider::resolve(
            None,
            params.use_openai.unwrap_or(false),
            project_config.embedding.provider,
        ),
        Some(other) => {
            return Err(internal_error(format!(
                "Unknown provider '{}'. Expected: local, openai, or ollama.",
                other
            )))
        }
    };

    let task_embedding: Embedding = match provider {
        Provider::Openai => {
            let provider = OpenAIProvider::from_env().map_err(|e| {
                internal_error(format!(
                    "Failed to initialize OpenAI provider: {}. Set OPENAI_API_KEY environment variable.",
                    e
                ))
            })?;
            provider
                .embed_async(&params.task)
                .await
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
        Provider::Ollama => {
            let provider = OllamaProvider::from_config_async(
                project_config.embedding.model.as_deref(),
                project_config.embedding.host.as_deref(),
            )
            .await
            .map_err(|e| internal_error(format!("Failed to initialize Ollama provider: {}", e)))?;
            provider
                .embed_async(&params.task)
                .await
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
        Provider::Local => {
            // Local fastembed is CPU-bound; sync embed is fine.
            let provider = LocalProvider::new().map_err(|e| {
                internal_error(format!("Failed to initialize embedding model: {}", e))
            })?;
            provider
                .embed(&params.task)
                .map_err(|e| internal_error(format!("Failed to generate embedding: {}", e)))?
        }
    };

    // Run smart context selection with pre-computed embedding
    let result = {
        let db = server.db.lock().unwrap();
        let analytics = server
            .analytics
            .as_ref()
            .ok_or_else(|| internal_error("Analytics not available"))?
            .lock()
            .unwrap();

        smart_context_with_embedding_with_options(
            &db,
            &analytics,
            &params.task,
            &task_embedding,
            config,
            false,
        )
    }
    .map_err(|e| internal_error(format!("Smart context selection failed: {}", e)))?;

    let max_tokens = params.max_tokens.unwrap_or(8000);
    if max_tokens == 0 {
        return Err(internal_error("max_tokens must be greater than zero"));
    }

    if result.selected_files.is_empty() {
        if result.truncated {
            return Err(internal_error(format!(
                "max_tokens={} is too small for the selected context; increase max_tokens",
                max_tokens
            )));
        }
        return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "No relevant files found for task: \"{}\"",
            params.task
        ))]));
    }

    let root = server.root();
    let files: Vec<_> = result.selected_files.iter().collect();
    let output =
        render_smart_context_output(root, &params.task, &files, result.omitted_count, max_tokens)
            .map_err(internal_error)?;

    Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
}

/// Render MCP smart context without exceeding the tool's declared token
/// budget. Files are kept whole; a file that would overflow the complete
/// response is omitted and the next ranked candidate is tried.
fn render_smart_context_output(
    root: &Path,
    task: &str,
    files: &[&crate::smart::FileSelection],
    already_omitted: usize,
    max_tokens: usize,
) -> std::result::Result<String, String> {
    let mut selected = Vec::new();

    for file in files {
        let mut candidate = selected.clone();
        candidate.push(*file);
        let omitted = already_omitted + files.len() - candidate.len();
        let output = format_smart_context_output(root, task, &candidate, omitted);
        let tokens =
            crate::tokens::count_tokens_with_encoding(&output, crate::tokens::Encoding::default())
                .map_err(|error| format!("failed to count MCP smart context tokens: {error}"))?;
        if tokens <= max_tokens {
            selected = candidate;
        }
    }

    if selected.is_empty() {
        return Err(format!(
            "max_tokens={} is too small for the smart_context response; increase max_tokens",
            max_tokens
        ));
    }

    let omitted = already_omitted + files.len() - selected.len();
    let output = format_smart_context_output(root, task, &selected, omitted);
    let tokens =
        crate::tokens::count_tokens_with_encoding(&output, crate::tokens::Encoding::default())
            .map_err(|error| format!("failed to count MCP smart context tokens: {error}"))?;
    if tokens > max_tokens {
        return Err(format!(
            "failed to fit smart_context response within max_tokens={} ({} tokens)",
            max_tokens, tokens
        ));
    }
    Ok(output)
}

fn format_smart_context_output(
    root: &Path,
    task: &str,
    files: &[&crate::smart::FileSelection],
    omitted: usize,
) -> String {
    let total_tokens: usize = files.iter().map(|file| file.token_count).sum();
    let mut output = format!("Smart context for: \"{}\"\n\n", task);
    output.push_str(&format!(
        "Selected {} files ({} tokens){}:\n\n",
        files.len(),
        total_tokens,
        if omitted > 0 {
            format!(", {} omitted due to token limit", omitted)
        } else {
            String::new()
        }
    ));

    for file in files {
        output.push_str(&format!(
            "- {} (relevance: {:.0}%, {} tokens)\n",
            file.path,
            file.relevance_score * 100.0,
            file.token_count
        ));
        for reason in &file.reasons {
            output.push_str(&format!("  - {:?}\n", reason));
        }
    }

    output.push_str("\n---\n\nSelected file contents:\n\n");
    for file in files {
        let path = root.join(&file.path);
        if let Ok(content) = std::fs::read_to_string(&path) {
            output.push_str(&format!("// === {} ===\n\n", file.path));
            output.push_str(&content);
            output.push_str("\n\n");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_callers_tool_definition() {
        let tool = get_callers_tool();
        assert_eq!(tool.name.as_ref(), "get_callers");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_get_callees_tool_definition() {
        let tool = get_callees_tool();
        assert_eq!(tool.name.as_ref(), "get_callees");
        assert!(tool.description.is_some());
    }

    #[test]
    fn test_smart_context_tool_definition() {
        let tool = smart_context_tool();
        assert_eq!(tool.name.as_ref(), "smart_context");
        assert!(tool.description.is_some());
    }

    fn project(src: &str) -> (tempfile::TempDir, CtxServer) {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), src).unwrap();
        let mut indexer = crate::index::Indexer::with_config(
            dir.path(),
            false,
            crate::walker::WalkerConfig::default(),
        )
        .unwrap();
        indexer.index().unwrap();
        let server = CtxServer::new(dir.path().to_path_buf()).unwrap();
        (dir, server)
    }

    fn text(result: &CallToolResult) -> String {
        match &result.content[0] {
            ContentBlock::Text(t) => t.text.clone(),
            _ => panic!("expected text"),
        }
    }

    fn args(v: serde_json::Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    const SRC: &str = r#"
pub struct Node;
impl Node {
    pub fn get_value(&self) -> u32 { 1 }
}
pub fn lookup(n: &Node) -> u32 { n.get_value() }
pub fn outer(n: &Node) -> u32 { lookup(n) }
pub fn test_node_get_value_twice(n: &Node) -> u32 { 2 }
"#;

    #[tokio::test]
    async fn callers_find_methods_and_prefer_exact_names() {
        let (_d, server) = project(SRC);
        let out = text(
            &get_callers(
                &server,
                Some(&args(serde_json::json!({"function": "get_value"}))),
            )
            .await
            .unwrap(),
        );
        // The method is found (kind filter used to exclude methods) and the
        // substring match `test_node_get_value_twice` is not chosen instead.
        assert!(
            out.contains("Callers of") && out.contains("get_value [method]"),
            "{out}"
        );
        assert!(!out.contains("test_node_get_value_twice"), "{out}");
        assert!(out.contains("lookup"), "{out}");
        // depth: outer calls lookup, which calls get_value
        assert!(out.contains("outer"), "{out}");
        let shallow = text(
            &get_callers(
                &server,
                Some(&args(
                    serde_json::json!({"function": "get_value", "depth": 1}),
                )),
            )
            .await
            .unwrap(),
        );
        assert!(
            shallow.contains("lookup") && !shallow.contains("outer"),
            "{shallow}"
        );
    }

    #[tokio::test]
    async fn callees_report_the_resolved_definition() {
        let (_d, server) = project(SRC);
        let out = text(
            &get_callees(
                &server,
                Some(&args(serde_json::json!({"function": "lookup"}))),
            )
            .await
            .unwrap(),
        );
        assert!(
            out.contains("get_value") && out.contains("[method] (src/lib.rs:"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn unknown_function_is_reported() {
        let (_d, server) = project(SRC);
        let out = text(
            &get_callers(
                &server,
                Some(&args(serde_json::json!({"function": "nope_nothing"}))),
            )
            .await
            .unwrap(),
        );
        assert!(out.contains("not found"), "{out}");
    }
}
