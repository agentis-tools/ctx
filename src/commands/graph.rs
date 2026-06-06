//! Graph visualization command.
//!
//! Handles dependency graph generation in various formats (DOT, Mermaid, JSON).

use std::collections::{HashMap, HashSet};
use std::env;

use ctx::analytics;
use ctx::error::Result;

/// Output file dependencies in DOT format.
fn output_file_deps_dot(deps: &[(String, String, i64)]) {
    println!("digraph dependencies {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box, style=filled, fillcolor=lightblue];");
    println!("  edge [color=gray];");

    let mut nodes: HashSet<String> = HashSet::new();
    for (src, tgt, _) in deps {
        nodes.insert(src.clone());
        if tgt != "external" {
            nodes.insert(tgt.clone());
        }
    }

    for node in &nodes {
        let short_name = node.split('/').last().unwrap_or(node);
        println!("  \"{}\" [label=\"{}\"];", node, short_name);
    }

    for (src, tgt, count) in deps {
        if tgt != "external" {
            let weight = (*count as f64).sqrt().ceil() as i64;
            println!("  \"{}\" -> \"{}\" [penwidth={}];", src, tgt, weight.max(1));
        }
    }
    println!("}}");
}

/// Output file dependencies in Mermaid format.
fn output_file_deps_mermaid(deps: &[(String, String, i64)]) {
    println!("```mermaid");
    println!("graph LR");
    for (i, (src, tgt, _)) in deps.iter().enumerate() {
        if tgt != "external" {
            let src_short = src.split('/').last().unwrap_or(src);
            let tgt_short = tgt.split('/').last().unwrap_or(tgt);
            println!("  A{}[{}] --> B{}[{}]", i, src_short, i, tgt_short);
        }
    }
    println!("```");
}

/// Output file dependencies in JSON format.
fn output_file_deps_json(deps: &[(String, String, i64)]) -> Result<()> {
    let nodes: Vec<_> = deps
        .iter()
        .flat_map(|(src, tgt, _)| vec![src.clone(), tgt.clone()])
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|n| n != "external")
        .collect();

    let edges: Vec<_> = deps
        .iter()
        .filter(|(_, tgt, _)| tgt != "external")
        .map(|(src, tgt, count)| serde_json::json!({"source": src, "target": tgt, "weight": count}))
        .collect();

    let graph = serde_json::json!({"type": "file_dependencies", "nodes": nodes, "edges": edges});
    println!("{}", serde_json::to_string_pretty(&graph)?);
    Ok(())
}

/// Output call graph in DOT format.
fn output_call_graph_dot(graph: &[(String, String, String, String)]) {
    println!("digraph call_graph {{");
    println!("  rankdir=LR;");
    println!("  node [shape=ellipse];");

    let mut files: HashMap<String, Vec<String>> = HashMap::new();
    for (src_file, src_name, tgt_file, tgt_name) in graph {
        files
            .entry(src_file.clone())
            .or_default()
            .push(src_name.clone());
        files
            .entry(tgt_file.clone())
            .or_default()
            .push(tgt_name.clone());
    }

    for (i, (file, symbols)) in files.iter().enumerate() {
        let short_file = file.split('/').last().unwrap_or(file);
        println!("  subgraph cluster_{} {{", i);
        println!("    label=\"{}\";", short_file);
        println!("    style=filled;");
        println!("    color=lightgrey;");
        for sym in symbols.iter().collect::<HashSet<_>>() {
            println!("    \"{}\";", sym);
        }
        println!("  }}");
    }

    for (_, src_name, _, tgt_name) in graph {
        println!("  \"{}\" -> \"{}\";", src_name, tgt_name);
    }
    println!("}}");
}

/// Output call graph in Mermaid format.
fn output_call_graph_mermaid(graph: &[(String, String, String, String)]) {
    println!("```mermaid");
    println!("graph LR");
    for (_, src_name, _, tgt_name) in graph {
        println!(
            "  {}[{}] --> {}[{}]",
            src_name.replace("::", "_"),
            src_name,
            tgt_name.replace("::", "_"),
            tgt_name
        );
    }
    println!("```");
}

/// Output call graph in JSON format.
fn output_call_graph_json(
    graph: &[(String, String, String, String)],
) -> Result<()> {
    let nodes: Vec<_> = graph
        .iter()
        .flat_map(|(sf, sn, tf, tn)| {
            vec![
                serde_json::json!({"name": sn, "file": sf}),
                serde_json::json!({"name": tn, "file": tf}),
            ]
        })
        .collect();

    let edges: Vec<_> = graph
        .iter()
        .map(|(_, src, _, tgt)| serde_json::json!({"source": src, "target": tgt}))
        .collect();

    let result = serde_json::json!({"type": "call_graph", "nodes": nodes, "edges": edges});
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// Generate a dependency graph visualization.
pub fn run_graph(
    output: &str,
    by_file: bool,
    filter: Option<String>,
    depth: i32,
) -> Result<()> {
    let root = env::current_dir()?;
    let analytics = analytics::Analytics::open(&root)?;

    let filter_files: Option<Vec<&str>> = filter
        .as_ref()
        .map(|f| f.split(',').map(|s| s.trim()).collect());

    if by_file {
        let deps = analytics.file_dependencies()?;
        let deps: Vec<_> = if let Some(ref filters) = filter_files {
            deps.into_iter()
                .filter(|(src, tgt, _)| filters.iter().any(|f| src.contains(f) || tgt.contains(f)))
                .collect()
        } else {
            deps
        };

        match output {
            "dot" => output_file_deps_dot(&deps),
            "mermaid" => output_file_deps_mermaid(&deps),
            "json" => output_file_deps_json(&deps)?,
            _ => {
                println!("File Dependency Graph");
                println!("{}", "=".repeat(80));
                for (src, tgt, count) in &deps {
                    println!("{} -> {} ({} calls)", src, tgt, count);
                }
            }
        }
    } else {
        let graph = analytics.full_call_graph(depth)?;
        let graph: Vec<_> = if let Some(ref filters) = filter_files {
            graph
                .into_iter()
                .filter(|(src_file, _, tgt_file, _)| {
                    filters
                        .iter()
                        .any(|f| src_file.contains(f) || tgt_file.contains(f))
                })
                .collect()
        } else {
            graph
        };

        match output {
            "dot" => output_call_graph_dot(&graph),
            "mermaid" => output_call_graph_mermaid(&graph),
            "json" => output_call_graph_json(&graph)?,
            _ => {
                println!("Symbol Call Graph");
                println!("{}", "=".repeat(80));
                for (src_file, src_name, tgt_file, tgt_name) in &graph {
                    println!("{} ({}) -> {} ({})", src_name, src_file, tgt_name, tgt_file);
                }
            }
        }
    }

    Ok(())
}
