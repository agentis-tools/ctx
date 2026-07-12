//! Embedded templates for `ctx harness init`.
//!
//! All generated files come from raw template files embedded into the binary
//! with `include_str!`. Templates use `{{TOKEN}}` placeholders:
//!
//! - `{{CTX_VERSION}}` -- this binary's crate version
//! - `{{AUTHOR_NAME}}` -- crate authors with `<email>` stripped
//! - `{{DEFAULT_BRANCH}}` -- `origin/HEAD` short name, falling back to `main`
//!
//! The checksum header (see [`super::checksum`]) is applied *after*
//! rendering, when the final content is known.

use std::path::Path;
use std::process::Command;

use super::checksum::{finalize, style_for_path};

pub const SESSION_START_SH: &str = include_str!("templates/session-start.sh");
pub const POST_TOOL_USE_SH: &str = include_str!("templates/post-tool-use.sh");
pub const STOP_SH: &str = include_str!("templates/stop.sh");
pub const RULES_TOML: &str = include_str!("templates/rules.toml");
pub const SETTINGS_SNIPPET_JSON: &str = include_str!("templates/settings-snippet.json");
pub const CLAUDE_MD_BLOCK_MD: &str = include_str!("templates/claude-md-block.md");
pub const PLUGIN_JSON: &str = include_str!("templates/plugin.json");
pub const HOOKS_JSON: &str = include_str!("templates/hooks.json");
pub const MCP_JSON: &str = include_str!("templates/mcp.json");
pub const MARKETPLACE_JSON: &str = include_str!("templates/marketplace.json");
pub const PLUGIN_SETTINGS_JSON: &str = include_str!("templates/plugin-settings.json");
pub const SKILL_MD: &str = include_str!("templates/SKILL.md");
pub const PLUGIN_README_MD: &str = include_str!("templates/plugin-README.md");
pub const CODEX_SESSION_START_SH: &str = include_str!("templates/codex-session-start.sh");
pub const CODEX_POST_TOOL_USE_SH: &str = include_str!("templates/codex-post-tool-use.sh");
pub const CODEX_STOP_SH: &str = include_str!("templates/codex-stop.sh");
pub const CODEX_HOOKS_JSON: &str = include_str!("templates/codex-hooks.json");
pub const CODEX_PLUGIN_HOOKS_JSON: &str = include_str!("templates/codex-plugin-hooks.json");
pub const CODEX_PLUGIN_JSON: &str = include_str!("templates/codex-plugin.json");
pub const CODEX_PLUGIN_MCP_JSON: &str = include_str!("templates/codex-plugin-mcp.json");
pub const CODEX_MARKETPLACE_JSON: &str = include_str!("templates/codex-marketplace.json");
pub const AGENTS_MD_BLOCK_MD: &str = include_str!("templates/agents-md-block.md");
pub const CODEX_PLUGIN_README_MD: &str = include_str!("templates/codex-plugin-README.md");

/// This binary's version (the value baked into `{{CTX_VERSION}}`).
pub const CTX_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Replace every `{{KEY}}` in `tmpl` with its value.
pub fn render(tmpl: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tmpl.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// Crate authors with `<email>` parts stripped (`a <x@y>:b` -> `a, b`).
pub fn author_name() -> String {
    env!("CARGO_PKG_AUTHORS")
        .split(':')
        .map(|author| match author.split_once('<') {
            Some((name, _)) => name.trim(),
            None => author.trim(),
        })
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The repository's default branch: `git symbolic-ref --short
/// refs/remotes/origin/HEAD` with the `origin/` prefix stripped, falling
/// back to `main` when git or the remote ref is unavailable.
pub fn default_branch(root: &Path) -> String {
    let output = Command::new("git")
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .current_dir(root)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let name = name.strip_prefix("origin/").unwrap_or(&name).to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "main".to_string()
}

/// The standard token set for rendering, with this binary's version.
pub fn standard_vars<'a>(default_branch: &'a str, author: &'a str) -> Vec<(&'static str, &'a str)> {
    vec![
        ("CTX_VERSION", CTX_VERSION),
        ("AUTHOR_NAME", author),
        ("DEFAULT_BRANCH", default_branch),
    ]
}

/// Render a hook script template with an explicit `{{CTX_VERSION}}` and
/// return the finalized (headered + checksummed) content.
///
/// `name` is one of `session-start`, `post-tool-use`, `stop`. Used by tests
/// to bake an arbitrary version into the compat guard (fail-open behavior)
/// without depending on the crate version.
pub fn render_hook_with_version(name: &str, version: &str, default_branch: &str) -> Option<String> {
    let tmpl = match name {
        "session-start" => SESSION_START_SH,
        "post-tool-use" => POST_TOOL_USE_SH,
        "stop" => STOP_SH,
        _ => return None,
    };
    let rendered = render(
        tmpl,
        &[("CTX_VERSION", version), ("DEFAULT_BRANCH", default_branch)],
    );
    let rel = format!("{name}.sh");
    Some(finalize(&rendered, style_for_path(&rel), version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_replaces_all_tokens() {
        let out = render(
            "v{{CTX_VERSION}} by {{AUTHOR_NAME}} on {{DEFAULT_BRANCH}}",
            &[
                ("CTX_VERSION", "1.0.0"),
                ("AUTHOR_NAME", "Jane"),
                ("DEFAULT_BRANCH", "main"),
            ],
        );
        assert_eq!(out, "v1.0.0 by Jane on main");
    }

    #[test]
    fn test_author_name_strips_email() {
        // The crate's own authors field must render to a non-empty name
        // without angle brackets.
        let name = author_name();
        assert!(!name.is_empty());
        assert!(!name.contains('<'), "author: {}", name);
        assert!(!name.contains('@'), "author: {}", name);
    }

    #[test]
    fn test_default_branch_falls_back_to_main() {
        // A temp dir with no git repo (and no origin) falls back to main.
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(default_branch(temp.path()), "main");
    }

    #[test]
    fn test_render_hook_with_version_bakes_guard() {
        let content = render_hook_with_version("stop", "9.9.9", "main").unwrap();
        assert!(content.contains("ctx harness compat --require \"9.9.9\""));
        assert!(content.contains("generated by ctx v9.9.9"));
        assert!(!content.contains("{{"), "unrendered token in: {}", content);
        assert!(render_hook_with_version("nope", "1.0.0", "main").is_none());
    }
}
