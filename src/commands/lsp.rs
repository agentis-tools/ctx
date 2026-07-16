//! `ctx lsp` -- community-registry LSP server management CLI.
//!
//! Thin wrapper around [`ctx::lsp_registry`] (fetch curated manifests),
//! [`ctx::config_edit`] (format-preserving `.ctx/config.toml` writes), and
//! [`ctx::lsp::status`] (health probes):
//!
//! - `add` installs a curated `[lsp.<language>]` entry after confirmation,
//! - `list` shows configured (or `--available` registry) servers,
//! - `update` refreshes entries carrying `source = "registry"` provenance,
//! - `doctor` probes every configured server end to end.
//!
//! Exit codes: 0 = success (including "already configured" / "up to date"),
//! 1 = doctor found failures, 2 = operational error (network, unknown
//! language, refusal, prompt needed but stdin is not a TTY).

use std::io::{IsTerminal, Write};
use std::path::Path;

use ctx::config_edit::{
    from_registry, lsp_entry_extra_keys, lsp_entry_matches, lsp_entry_registry_owned,
    refresh_lsp_entry, registry_owned_languages, upsert_lsp_entry, LspConfigEntry, SOURCE_REGISTRY,
};
use ctx::error::{CtxError, Result};
use ctx::exit::Outcome;
use ctx::lsp::config::LspConfigLoad;
use ctx::lsp::status::{doctor_verbose, find_executable, LspHealthReport};
use ctx::lsp::LspConfig;
use ctx::lsp::LspServerConfig;
use ctx::lsp_registry::{
    fetch_index, fetch_language, install_hint_for_current_os, registry_base_url,
};

use crate::cli::LspCommand;

/// Run `ctx lsp <SUBCOMMAND>` in the current directory.
pub fn run_lsp(cmd: LspCommand, json: bool) -> Result<Outcome> {
    let root = std::env::current_dir()?;
    match cmd {
        LspCommand::Add {
            language,
            server,
            yes,
        } => run_add(&root, &language, server.as_deref(), yes, json),
        LspCommand::List { available } => run_list(&root, available, json),
        LspCommand::Update { language, yes } => run_update(&root, language.as_deref(), yes, json),
        LspCommand::Doctor => run_doctor(&root, json),
    }
}

// ============================================================================
// add
// ============================================================================

fn run_add(
    root: &Path,
    language: &str,
    server: Option<&str>,
    yes: bool,
    json: bool,
) -> Result<Outcome> {
    // Existence/ownership is checked on the raw toml_edit document (not the
    // serde-typed config, which falls back to empty defaults on type errors)
    // so a hand-written entry is never touched — even one serde cannot type,
    // like `command = 3`. Refuse before any network call.
    let existing = lsp_entry_registry_owned(root, language)?;
    if existing == Some(false) {
        return Err(CtxError::Other(format!(
            "[lsp.{language}] already exists in .ctx/config.toml and is not registry-managed \
             (no `source = \"{SOURCE_REGISTRY}\"` provenance); fix or remove the entry, then \
             re-run 'ctx lsp add {language}'"
        )));
    }

    let base = registry_base_url();
    let index = fetch_index(&base)?;
    if !index.languages.contains_key(language) {
        let available: Vec<&str> = index.languages.keys().map(String::as_str).collect();
        let available = if available.is_empty() {
            "(none)".to_string()
        } else {
            available.join(", ")
        };
        return Err(CtxError::Other(format!(
            "language '{language}' is not in the LSP registry; available languages: {available}"
        )));
    }

    let entry = fetch_language(&base, language)?;
    let spec = entry.server(server)?;
    let proposed = from_registry(&entry, spec);
    let source_url = format!("{}/registry/{language}.toml", base.trim_end_matches('/'));

    // Registry-managed and identical to what we would write: nothing to do.
    if existing == Some(true) {
        if lsp_entry_matches(root, language, &proposed)? {
            if json {
                ctx::json::emit(
                    "lsp.add",
                    serde_json::json!({
                        "language": language,
                        "server": spec.name,
                        "status": "already_configured",
                    }),
                )?;
            } else {
                println!(
                    "[lsp.{language}] is already configured from the registry (server {}); \
                     nothing to do",
                    spec.name
                );
            }
            return Ok(Outcome::Clean);
        }
        return Err(CtxError::Other(format!(
            "[lsp.{language}] in .ctx/config.toml differs from the registry entry for server \
             '{name}'; remove the entry (or edit .ctx/config.toml), then re-run \
             'ctx lsp add {language} --server {name}'",
            name = spec.name
        )));
    }

    let install_hint = install_hint_for_current_os(spec).map(str::to_string);
    let binary_found = find_executable(&spec.command).is_some();

    if !json {
        println!("Registry entry for {language} ({source_url}):");
        println!(
            "  server:   {}{}",
            spec.name,
            if spec.recommended {
                " (recommended)"
            } else {
                ""
            }
        );
        println!("  command:  {}", command_line(&spec.command, &spec.args));
        if let Some(hint) = &install_hint {
            println!("  install:  {hint}");
        }
        if let Some(homepage) = &spec.homepage {
            println!("  homepage: {homepage}");
        }
        println!();
        println!("This writes [lsp.{language}] to .ctx/config.toml (backend \"hybrid\").");
    }

    if !confirm("Proceed?", yes, json)? {
        return Err(CtxError::Other(
            "aborted: nothing written to .ctx/config.toml".to_string(),
        ));
    }

    upsert_lsp_entry(root, language, &proposed)?;

    if !binary_found {
        eprintln!("warning: `{}` is not on PATH", spec.command);
        if let Some(hint) = &install_hint {
            eprintln!("         install it with: {hint}");
        }
        eprintln!("         then verify with 'ctx lsp doctor'");
    }

    if json {
        ctx::json::emit(
            "lsp.add",
            serde_json::json!({
                "language": language,
                "server": spec.name,
                "command": spec.command,
                "args": spec.args,
                "backend": proposed.backend,
                "source": proposed.source,
                "registry_url": source_url,
                "install_hint": install_hint,
                "homepage": spec.homepage,
                "binary_found": binary_found,
                "status": "added",
            }),
        )?;
    } else {
        println!(
            "wrote [lsp.{language}] to .ctx/config.toml (server {}, backend {})",
            spec.name, proposed.backend
        );
        println!("run 'ctx index' to reindex with the new language server");
    }
    Ok(Outcome::Clean)
}

// ============================================================================
// list
// ============================================================================

fn run_list(root: &Path, available: bool, json: bool) -> Result<Outcome> {
    let config = LspConfig::load(root);

    if available {
        let base = registry_base_url();
        let index = fetch_index(&base)?;
        if json {
            let languages: Vec<serde_json::Value> = index
                .languages
                .iter()
                .map(|(lang, info)| {
                    serde_json::json!({
                        "language": lang,
                        "recommended": info.recommended,
                        "servers": info.servers,
                        "configured": config.lsp.contains_key(lang),
                    })
                })
                .collect();
            ctx::json::emit(
                "lsp.list",
                serde_json::json!({
                    "available": true,
                    "registry": base,
                    "languages": languages,
                }),
            )?;
        } else if index.languages.is_empty() {
            println!("the LSP registry lists no languages ({base})");
        } else {
            println!("Available languages in the LSP registry ({base}):");
            for (lang, info) in &index.languages {
                let mark = if config.lsp.contains_key(lang) {
                    "  [configured]"
                } else {
                    ""
                };
                println!("  {lang}  (recommended server: {}){mark}", info.recommended);
            }
            println!();
            println!("install one with 'ctx lsp add <language>'");
        }
        return Ok(Outcome::Clean);
    }

    if json {
        let servers: Vec<serde_json::Value> = config
            .lsp
            .iter()
            .map(|(lang, cfg)| {
                serde_json::json!({
                    "language": lang,
                    "command": cfg.command,
                    "args": cfg.args,
                    "backend": cfg.backend.as_str(),
                    "source": source_label(cfg),
                    "source_server": cfg.source_server,
                })
            })
            .collect();
        ctx::json::emit(
            "lsp.list",
            serde_json::json!({ "available": false, "servers": servers }),
        )?;
    } else if config.lsp.is_empty() {
        println!("no LSP servers configured; try 'ctx lsp list --available'");
    } else {
        for (lang, cfg) in &config.lsp {
            let source = match (source_label(cfg), cfg.source_server.as_deref()) {
                ("registry", Some(server)) if !server.is_empty() => format!("registry ({server})"),
                (label, _) => label.to_string(),
            };
            println!(
                "{lang}: `{}` (backend {}, source {source})",
                command_line(&cfg.command, &cfg.args),
                cfg.backend.as_str()
            );
        }
    }
    Ok(Outcome::Clean)
}

/// `"registry"` for registry-provenance entries, `"manual"` otherwise.
fn source_label(cfg: &LspServerConfig) -> &'static str {
    if cfg.source.as_deref() == Some(SOURCE_REGISTRY) {
        "registry"
    } else {
        "manual"
    }
}

// ============================================================================
// update
// ============================================================================

fn run_update(root: &Path, language: Option<&str>, yes: bool, json: bool) -> Result<Outcome> {
    let owned = registry_owned_languages(root)?;
    let config = LspConfig::load(root);

    let targets: Vec<String> = match language {
        Some(lang) if owned.iter().any(|l| l == lang) => vec![lang.to_string()],
        Some(lang) if config.lsp.contains_key(lang) => {
            return Err(CtxError::Other(format!(
                "[lsp.{lang}] in .ctx/config.toml is user-owned (no `source = \"registry\"` \
                 provenance); 'ctx lsp update' only refreshes registry-installed entries — \
                 edit it manually instead"
            )));
        }
        Some(lang) => {
            return Err(CtxError::Other(format!(
                "no [lsp.{lang}] entry in .ctx/config.toml; run 'ctx lsp add {lang}' to \
                 install one"
            )));
        }
        None => owned,
    };

    if targets.is_empty() {
        if json {
            ctx::json::emit("lsp.update", serde_json::json!({ "languages": [] }))?;
        } else {
            println!("no registry-managed LSP entries in .ctx/config.toml; nothing to update");
        }
        return Ok(Outcome::Clean);
    }

    let base = registry_base_url();
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for lang in &targets {
        let current = config.lsp.get(lang).ok_or_else(|| {
            CtxError::Other(format!(
                "[lsp.{lang}] could not be loaded from .ctx/config.toml; fix the entry \
                 manually and re-run"
            ))
        })?;

        let entry = fetch_language(&base, lang)?;
        let source_server = current.source_server.as_deref().unwrap_or("");
        // Fall back to the recommended server when the provenance key is
        // missing/empty (older or hand-repaired entries).
        let spec = entry.server(if source_server.is_empty() {
            None
        } else {
            Some(source_server)
        })?;
        let fresh = from_registry(&entry, spec);
        let changes = diff_entry(current, &fresh);

        if changes.is_empty() {
            if !json {
                println!("{lang}: up to date (server {})", spec.name);
            }
            results.push(serde_json::json!({
                "language": lang,
                "server": spec.name,
                "status": "up_to_date",
            }));
            continue;
        }

        // Extra keys the user added to the table (timeout_ms, env, ...) are
        // preserved by the write below; say so in the consent output.
        let extra_keys = lsp_entry_extra_keys(root, lang)?;
        if !json {
            println!("{lang}: registry entry for server {} changed:", spec.name);
            for (key, from, to) in &changes {
                println!("  {key}: {from} -> {to}");
            }
            if !extra_keys.is_empty() {
                println!("  (preserving user keys: {})", extra_keys.join(", "));
            }
        }
        // Declining one language skips it and continues with the rest;
        // only a needed-but-impossible prompt (non-TTY without --yes)
        // aborts with exit 2, via `confirm` returning Err.
        if !confirm(&format!("Update [lsp.{lang}]?"), yes, json)? {
            skipped += 1;
            if !json {
                println!("skipped [lsp.{lang}]");
            }
            results.push(serde_json::json!({
                "language": lang,
                "server": spec.name,
                "status": "skipped",
            }));
            continue;
        }
        refresh_lsp_entry(root, lang, &fresh)?;
        updated += 1;
        if !json {
            println!("updated [lsp.{lang}] in .ctx/config.toml");
        }
        let change_map: serde_json::Map<String, serde_json::Value> = changes
            .iter()
            .map(|(key, from, to)| {
                (
                    key.to_string(),
                    serde_json::json!({ "from": from, "to": to }),
                )
            })
            .collect();
        results.push(serde_json::json!({
            "language": lang,
            "server": spec.name,
            "status": "updated",
            "changes": change_map,
            "preserved_keys": extra_keys,
        }));
    }

    if json {
        ctx::json::emit(
            "lsp.update",
            serde_json::json!({ "registry": base, "languages": results }),
        )?;
    } else if updated + skipped > 0 {
        println!("updated {updated}, skipped {skipped}");
    }
    Ok(Outcome::Clean)
}

/// Per-key diff between the configured entry and a freshly fetched one.
/// Values are rendered as display strings (shared by text and JSON output).
fn diff_entry(
    current: &LspServerConfig,
    fresh: &LspConfigEntry,
) -> Vec<(&'static str, String, String)> {
    let mut changes = Vec::new();
    let mut push = |key: &'static str, from: String, to: String| {
        if from != to {
            changes.push((key, from, to));
        }
    };
    push(
        "command",
        format!("{:?}", current.command),
        format!("{:?}", fresh.command),
    );
    push(
        "args",
        format!("{:?}", current.args),
        format!("{:?}", fresh.args),
    );
    push(
        "extensions",
        format!("{:?}", current.extensions),
        format!("{:?}", fresh.extensions),
    );
    push(
        "root_markers",
        format!("{:?}", current.root_markers),
        format!("{:?}", fresh.root_markers),
    );
    push(
        "capabilities",
        format!("{:?}", current.capabilities),
        format!("{:?}", fresh.capabilities),
    );
    push(
        "backend",
        format!("{:?}", current.backend.as_str()),
        format!("{:?}", fresh.backend),
    );
    push(
        "source_server",
        format!("{:?}", current.source_server.as_deref().unwrap_or("")),
        format!("{:?}", fresh.source_server),
    );
    changes
}

// ============================================================================
// doctor
// ============================================================================

fn run_doctor(root: &Path, json: bool) -> Result<Outcome> {
    // The diagnostic loader distinguishes "no config" from "config the
    // fault-tolerant loader would silently drop": a malformed file is a
    // finding, not an empty bill of health.
    let config = match LspConfig::load_diagnostic(root) {
        LspConfigLoad::Absent => LspConfig::default(),
        LspConfigLoad::Loaded(config) => config,
        LspConfigLoad::Malformed(error) => {
            if json {
                ctx::json::emit(
                    "lsp.doctor",
                    serde_json::json!({
                        "healthy": false,
                        "summary": { "pass": 0, "warn": 0, "fail": 1 },
                        "servers": [{
                            "status": "fail",
                            "error": format!(".ctx/config.toml cannot be loaded: {error}"),
                        }],
                    }),
                )?;
            } else {
                println!("FAIL .ctx/config.toml cannot be loaded:");
                for line in error.trim_end().lines() {
                    println!("     {line}");
                }
                println!(
                    "     hint: fix (or remove) the broken configuration, then re-run \
                     'ctx lsp doctor'"
                );
            }
            return Ok(Outcome::Findings);
        }
    };

    if config.lsp.is_empty() {
        if json {
            ctx::json::emit(
                "lsp.doctor",
                serde_json::json!({
                    "healthy": true,
                    "summary": { "pass": 0, "warn": 0, "fail": 0 },
                    "servers": [],
                }),
            )?;
        } else {
            println!("no LSP servers configured; try 'ctx lsp list --available'");
        }
        return Ok(Outcome::Clean);
    }

    // Blocks dropped by validation (empty command, missing extensions, ...)
    // count as failures alongside the probe reports.
    let (reports, dropped) = doctor_verbose(root, &config);
    let statuses: Vec<&'static str> = reports.iter().map(report_status).collect();
    let pass = statuses.iter().filter(|s| **s == "pass").count();
    let warn = statuses.iter().filter(|s| **s == "warn").count();
    let fail = statuses.iter().filter(|s| **s == "fail").count() + dropped.len();
    let healthy = fail == 0;
    let total = reports.len() + dropped.len();

    if json {
        let mut servers: Vec<serde_json::Value> = reports
            .iter()
            .zip(&statuses)
            .map(|(report, status)| {
                let mut value = serde_json::to_value(report).unwrap_or(serde_json::Value::Null);
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "status".to_string(),
                        serde_json::Value::String((*status).to_string()),
                    );
                }
                value
            })
            .collect();
        servers.extend(dropped.iter().map(|block| {
            serde_json::json!({
                "language": block.language,
                "status": "fail",
                "error": format!("invalid [lsp.{}] block: {}", block.language, block.reason),
            })
        }));
        ctx::json::emit(
            "lsp.doctor",
            serde_json::json!({
                "healthy": healthy,
                "summary": { "pass": pass, "warn": warn, "fail": fail },
                "servers": servers,
            }),
        )?;
    } else {
        for (report, status) in reports.iter().zip(&statuses) {
            print_report(report, status);
        }
        for block in &dropped {
            println!(
                "FAIL {}: invalid [lsp.{}] block in .ctx/config.toml: {}",
                block.language, block.language, block.reason
            );
            println!("     hint: fix or remove the block, then re-run 'ctx lsp doctor'");
        }
        println!();
        println!(
            "{total} server{}: {pass} pass, {warn} warn, {fail} fail",
            if total == 1 { "" } else { "s" }
        );
    }

    Ok(if healthy {
        Outcome::Clean
    } else {
        Outcome::Findings
    })
}

/// `fail` when the binary is missing or the handshake failed, `warn` when
/// requested capabilities were not advertised, `pass` otherwise.
fn report_status(report: &LspHealthReport) -> &'static str {
    if !report.binary_found || !report.handshake_ok {
        "fail"
    } else if !report.missing_capabilities.is_empty() {
        "warn"
    } else {
        "pass"
    }
}

fn print_report(report: &LspHealthReport, status: &str) {
    let tag = status.to_uppercase();
    if !report.binary_found {
        println!(
            "{tag} {}: `{}` not found on PATH",
            report.language, report.command
        );
        println!(
            "     hint: install the server (see 'ctx lsp list --available'), then re-run \
             'ctx lsp doctor'"
        );
        return;
    }
    if !report.handshake_ok {
        println!(
            "{tag} {}: `{}` failed the initialize handshake{}",
            report.language,
            report.command,
            report
                .error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
        for line in &report.stderr {
            println!("     stderr: {line}");
        }
        return;
    }

    let server = match (&report.server_name, &report.server_version) {
        (Some(name), Some(version)) => format!(" ({name} {version})"),
        (Some(name), None) => format!(" ({name})"),
        _ => String::new(),
    };
    if report.missing_capabilities.is_empty() {
        println!(
            "{tag} {}: `{}` handshake ok{server}, capabilities ok",
            report.language, report.command
        );
    } else {
        println!(
            "{tag} {}: `{}` handshake ok{server}, missing capabilities: {}",
            report.language,
            report.command,
            report.missing_capabilities.join(", ")
        );
        println!(
            "     hint: the server does not advertise everything the config requests; \
             extraction may be degraded"
        );
    }
}

// ============================================================================
// shared helpers
// ============================================================================

/// Render a command plus its arguments as one shell-style line.
fn command_line(command: &str, args: &[String]) -> String {
    let mut line = command.to_string();
    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }
    line
}

/// Ask for confirmation on stdin. `--yes` bypasses the prompt; JSON mode and
/// non-interactive stdin refuse instead of hanging (exit code 2 via `Err`).
fn confirm(question: &str, yes: bool, json: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if json {
        return Err(CtxError::Other(
            "--json mode never prompts; pass --yes to confirm".to_string(),
        ));
    }
    if !std::io::stdin().is_terminal() {
        return Err(CtxError::Other(
            "stdin is not a TTY; pass --yes to confirm non-interactively".to_string(),
        ));
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
