//! File discovery with glob patterns and layered ignore rules.
//!
//! Walks a project tree and returns the files that should be part of the
//! context or index, honoring (in order): hidden-file rules, `.gitignore`
//! at all levels, `.ignore`, `.contextignore`, ctx's built-in ignore list
//! (170+ patterns), and any custom include/ignore globs.
//!
//! The main entry point is [`discover_files`] with a [`WalkerConfig`];
//! [`FileFilter`] offers the same rules as a reusable per-file check
//! (used by watch mode).

use std::io;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::WalkBuilder;

use crate::default_ignores::DEFAULT_IGNORES;

/// Reusable matcher for positional file and directory patterns.
///
/// Patterns are ORed together. Non-glob values match a literal file or every
/// file below a literal directory, while glob values use the same
/// separator-aware matching as [`discover_files`]. A missing pattern list or
/// the conventional `.` pattern matches every repository-relative path.
#[derive(Debug, Clone)]
pub struct FilePatternFilter {
    root: PathBuf,
    globset: Option<GlobSet>,
    literal_paths: Vec<PathBuf>,
    match_all: bool,
}

impl FilePatternFilter {
    /// Compile positional patterns relative to `root`.
    pub fn new(root: &Path, patterns: &[String]) -> io::Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let match_all = patterns.is_empty() || patterns.iter().any(|pattern| pattern == ".");
        let globset = if match_all {
            None
        } else {
            build_include_globset(&root, patterns)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
        };
        let literal_paths = if match_all {
            Vec::new()
        } else {
            get_literal_paths(patterns)
                .into_iter()
                .map(|path| normalize_literal(&root, path))
                .collect()
        };

        Ok(Self {
            root,
            globset,
            literal_paths,
            match_all,
        })
    }

    /// Build a matcher that accepts every path.
    pub fn all(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            globset: None,
            literal_paths: Vec::new(),
            match_all: true,
        }
    }

    /// Return whether a repository-relative or absolute path is in scope.
    pub fn matches(&self, path: impl AsRef<Path>) -> bool {
        if self.match_all {
            return true;
        }

        let path = path.as_ref();
        let relative = if path.is_absolute() {
            match path.strip_prefix(&self.root) {
                Ok(relative) => relative,
                Err(_) => return false,
            }
        } else {
            path
        };

        self.literal_paths
            .iter()
            .any(|literal| relative == literal || relative.starts_with(literal))
            || self
                .globset
                .as_ref()
                .is_some_and(|globset| globset.is_match(relative))
    }
}

fn normalize_literal(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(&path).to_path_buf()
    } else {
        path
    }
}

/// File filter that can check if individual files should be included.
/// This is built once and reused for watch mode filtering.
///
/// This filter replicates the ignore sources used by WalkBuilder:
/// - Hidden files/directories (starting with `.`)
/// - .gitignore files (all levels)
/// - .git/info/exclude
/// - Global gitignore (~/.config/git/ignore or core.excludesFile)
/// - .ignore files (ripgrep-style, all levels)
/// - .contextignore files (all levels)
/// - Default ignores from ctx
/// - Custom ignore patterns from CLI
pub struct FileFilter {
    root: PathBuf,
    /// Matcher for default ignores and custom CLI ignores
    default_ignore_matcher: Gitignore,
    /// Combined matcher for .gitignore, .ignore, .git/info/exclude, global gitignore
    git_ignore_matcher: Option<Gitignore>,
    /// Combined matcher for .contextignore files at all levels
    contextignore_matcher: Option<Gitignore>,
    include_globset: Option<GlobSet>,
    /// Literal include paths, stored as relative paths for comparison
    include_literals_rel: Vec<PathBuf>,
    /// Literal include paths that were absolute, stored as absolute for comparison
    include_literals_abs: Vec<PathBuf>,
    use_gitignore: bool,
}

impl FileFilter {
    /// Build a file filter from walker configuration.
    ///
    /// This builds matchers that replicate WalkBuilder's ignore behavior,
    /// ensuring watch mode filtering is consistent with initial indexing.
    pub fn new(root: &Path, config: &WalkerConfig) -> io::Result<Self> {
        let root = if root.exists() {
            root.canonicalize()?
        } else {
            root.to_path_buf()
        };

        // Build matcher for default ignores and custom CLI ignores
        let default_ignore_matcher = build_ignore_matcher(&root, config);

        // Build combined git ignore matcher if enabled
        // This includes .gitignore, .ignore, .git/info/exclude, and global gitignore
        let git_ignore_matcher = if config.use_gitignore {
            build_combined_git_ignore_matcher(&root)
        } else {
            None
        };

        // Build contextignore matcher (all levels)
        let contextignore_matcher = build_all_contextignore_matcher(&root);

        // Build include patterns (normalize absolute globs relative to root)
        let include_globset = build_include_globset(&root, &config.include_patterns)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // Separate absolute and relative literal paths
        let literal_paths = get_literal_paths(&config.include_patterns);
        let mut include_literals_rel = Vec::new();
        let mut include_literals_abs = Vec::new();
        for p in literal_paths {
            if p.is_absolute() {
                // Store absolute path, canonicalized if possible
                let abs = p.canonicalize().unwrap_or(p);
                include_literals_abs.push(abs);
            } else {
                include_literals_rel.push(p);
            }
        }

        Ok(Self {
            root,
            default_ignore_matcher,
            git_ignore_matcher,
            contextignore_matcher,
            include_globset,
            include_literals_rel,
            include_literals_abs,
            use_gitignore: config.use_gitignore,
        })
    }

    /// Check if a file should be included.
    pub fn should_include(&self, file_path: &Path) -> bool {
        // Get relative path
        let rel_path = match file_path.strip_prefix(&self.root) {
            Ok(p) => p,
            Err(_) => return false,
        };

        let is_dir = file_path.is_dir();

        // Skip hidden files/directories (WalkBuilder does this by default)
        // Check if any component of the path starts with '.'
        for component in rel_path.components() {
            if let std::path::Component::Normal(name) = component {
                if let Some(name_str) = name.to_str() {
                    if name_str.starts_with('.') {
                        return false;
                    }
                }
            }
        }

        // Check default/custom ignores
        if self
            .default_ignore_matcher
            .matched_path_or_any_parents(rel_path, is_dir)
            .is_ignore()
        {
            return false;
        }

        // Check git-related ignores (.gitignore, .ignore, .git/info/exclude, global)
        if self.use_gitignore {
            if let Some(ref matcher) = self.git_ignore_matcher {
                if matcher
                    .matched_path_or_any_parents(rel_path, is_dir)
                    .is_ignore()
                {
                    return false;
                }
            }
        }

        // Check contextignore (all levels)
        if let Some(ref matcher) = self.contextignore_matcher {
            if matcher
                .matched_path_or_any_parents(rel_path, is_dir)
                .is_ignore()
            {
                return false;
            }
        }

        // Check include patterns
        let has_includes = self.include_globset.is_some()
            || !self.include_literals_rel.is_empty()
            || !self.include_literals_abs.is_empty();

        if has_includes {
            let mut matches = false;

            // Check relative literal paths
            for literal in &self.include_literals_rel {
                if rel_path.starts_with(literal) {
                    matches = true;
                    break;
                }
            }

            // Check absolute literal paths against the absolute file path
            if !matches {
                let abs_path = self.root.join(rel_path);
                for literal in &self.include_literals_abs {
                    if abs_path.starts_with(literal) || abs_path == *literal {
                        matches = true;
                        break;
                    }
                }
            }

            // Check glob patterns
            if !matches {
                if let Some(ref globset) = self.include_globset {
                    matches = globset.is_match(rel_path);
                }
            }

            if !matches {
                return false;
            }
        }

        true
    }
}

/// Build a combined matcher for all git-related ignore sources.
/// This includes:
/// - .gitignore files at all levels
/// - .ignore files at all levels (ripgrep-style)
/// - .git/info/exclude
/// - core.excludesFile from git config (system, global, and local)
/// - Default global gitignore (~/.config/git/ignore)
fn build_combined_git_ignore_matcher(root: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);

    // Add all core.excludesFile values (system, global, local)
    for excludes_file in get_all_git_excludes_files(root) {
        let _ = builder.add(&excludes_file);
    }

    // Add default global gitignore as fallback
    if let Some(global_gitignore) = find_default_global_gitignore() {
        let _ = builder.add(&global_gitignore);
    }

    // Add .git/info/exclude if in a git repo
    if let Some(git_dir) = find_git_dir(root) {
        let exclude_path = git_dir.join("info").join("exclude");
        if exclude_path.exists() {
            let _ = builder.add(&exclude_path);
        }
    }

    // Walk up to find parent .gitignore/.ignore files (for nested repos)
    let mut current = root.parent();
    while let Some(parent) = current {
        for filename in &[".gitignore", ".ignore"] {
            let ignore_path = parent.join(filename);
            if ignore_path.exists() {
                let _ = builder.add(&ignore_path);
            }
        }
        // Stop if we hit a .git directory (repo root)
        if parent.join(".git").exists() {
            break;
        }
        current = parent.parent();
    }

    // Add root level .gitignore and .ignore
    for filename in &[".gitignore", ".ignore"] {
        let ignore_path = root.join(filename);
        if ignore_path.exists() {
            let _ = builder.add(&ignore_path);
        }
    }

    // Recursively find all nested .gitignore and .ignore files
    add_nested_ignore_files(&mut builder, root, &[".gitignore", ".ignore"]);

    builder.build().ok()
}

/// Build a matcher for .contextignore files at all levels.
fn build_all_contextignore_matcher(root: &Path) -> Option<Gitignore> {
    let mut builder = GitignoreBuilder::new(root);
    let mut found_any = false;

    // Add root .contextignore
    let contextignore_path = root.join(".contextignore");
    if contextignore_path.exists() && builder.add(&contextignore_path).is_none() {
        found_any = true;
    }

    // Recursively find nested .contextignore files
    fn add_nested(builder: &mut GitignoreBuilder, dir: &Path, found: &mut bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }

                let contextignore = path.join(".contextignore");
                if contextignore.exists() && builder.add(&contextignore).is_none() {
                    *found = true;
                }

                add_nested(builder, &path, found);
            }
        }
    }

    add_nested(&mut builder, root, &mut found_any);

    if found_any {
        builder.build().ok()
    } else {
        None
    }
}

/// Find the default global gitignore file location (fallback when core.excludesFile not set).
/// Checks in order:
/// 1. $XDG_CONFIG_HOME/git/ignore
/// 2. ~/.config/git/ignore
/// 3. ~/.gitignore_global (legacy)
fn find_default_global_gitignore() -> Option<PathBuf> {
    // Check XDG config location
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_config).join("git").join("ignore");
        if path.exists() {
            return Some(path);
        }
    }

    // Get home directory from environment
    let home = std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))?;

    // Check ~/.config/git/ignore
    let path = home.join(".config").join("git").join("ignore");
    if path.exists() {
        return Some(path);
    }

    // Also check ~/.gitignore_global (common legacy location)
    let legacy_path = home.join(".gitignore_global");
    if legacy_path.exists() {
        return Some(legacy_path);
    }

    None
}

/// Get all core.excludesFile values from git config (system, global, and local).
/// Returns paths in order of precedence (local overrides global overrides system).
fn get_all_git_excludes_files(root: &Path) -> Vec<PathBuf> {
    use std::process::Command;

    let mut excludes_files = Vec::new();

    // Check all three scopes: system, global, local
    // Local config is checked from the repository root
    for scope in &["--system", "--global", "--local"] {
        let mut cmd = Command::new("git");
        cmd.args(["config", scope, "core.excludesFile"]);

        // For local config, we need to run from the repo directory
        if *scope == "--local" {
            cmd.current_dir(root);
        }

        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout);
                let path_str = path_str.trim();

                if !path_str.is_empty() {
                    if let Some(path) = expand_tilde(path_str) {
                        if path.exists() {
                            excludes_files.push(path);
                        }
                    }
                }
            }
        }
    }

    excludes_files
}

/// Expand ~ to home directory in a path string.
fn expand_tilde(path_str: &str) -> Option<PathBuf> {
    if let Some(stripped) = path_str.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .ok()
            .or_else(|| std::env::var("USERPROFILE").ok())?;
        Some(PathBuf::from(home).join(stripped))
    } else {
        Some(PathBuf::from(path_str))
    }
}

/// Find the .git directory for a repository.
fn find_git_dir(root: &Path) -> Option<PathBuf> {
    let git_dir = root.join(".git");
    if git_dir.is_dir() {
        return Some(git_dir);
    }

    // Walk up to find .git in parent directories
    let mut current = root.parent();
    while let Some(parent) = current {
        let git_dir = parent.join(".git");
        if git_dir.is_dir() {
            return Some(git_dir);
        }
        current = parent.parent();
    }

    None
}

/// Recursively add nested ignore files to the builder.
fn add_nested_ignore_files(builder: &mut GitignoreBuilder, dir: &Path, filenames: &[&str]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden directories and common non-source directories
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }

            // Check for ignore files in this directory
            for filename in filenames {
                let ignore_path = path.join(filename);
                if ignore_path.exists() {
                    let _ = builder.add(&ignore_path);
                }
            }

            // Recurse into subdirectory
            add_nested_ignore_files(builder, &path, filenames);
        }
    }
}

/// Represents a discovered file with its metadata.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub size: u64,
}

/// Configuration for the file walker.
#[derive(Debug, Clone)]
pub struct WalkerConfig {
    pub use_gitignore: bool,
    pub use_default_ignores: bool,
    pub custom_ignores: Vec<String>,
    pub include_patterns: Vec<String>,
}

impl WalkerConfig {
    /// Whether the include patterns actually narrow the walk. A lone `.`
    /// (or `./`) is the CLI's "whole repository" default, not a scope.
    pub fn has_scoping_includes(&self) -> bool {
        self.include_patterns
            .iter()
            .any(|p| p.trim_end_matches('/') != ".")
    }
}

impl Default for WalkerConfig {
    fn default() -> Self {
        Self {
            use_gitignore: true,
            use_default_ignores: true,
            custom_ignores: Vec::new(),
            include_patterns: Vec::new(),
        }
    }
}

/// Check if a pattern is a glob pattern (contains wildcards).
fn is_glob_pattern(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
}

/// Build a GlobSet from include patterns.
/// Absolute glob patterns are normalized by stripping the root prefix.
fn build_include_globset(
    root: &Path,
    patterns: &[String],
) -> Result<Option<GlobSet>, globset::Error> {
    let glob_patterns: Vec<&String> = patterns.iter().filter(|p| is_glob_pattern(p)).collect();

    if glob_patterns.is_empty() {
        return Ok(None);
    }

    let root_str = root.to_string_lossy();

    let mut builder = GlobSetBuilder::new();
    for pattern in glob_patterns {
        // Normalize absolute patterns by stripping the root prefix
        // e.g., /tmp/proj/src/**/*.rs -> src/**/*.rs when root is /tmp/proj
        let normalized = if pattern.starts_with('/') {
            // Try to strip root prefix from absolute pattern
            if let Some(rel) = pattern.strip_prefix(root_str.as_ref()) {
                rel.trim_start_matches('/').to_string()
            } else if let Some(rel) = pattern.strip_prefix(&format!("{}/", root_str)) {
                rel.to_string()
            } else {
                // Pattern is absolute but doesn't match root - keep as-is
                // (this won't match anything, but that's correct behavior)
                pattern.clone()
            }
        } else {
            pattern.clone()
        };

        // Use literal_separator(true) so that `*` doesn't match `/`
        // This makes `src/*.rs` match only files directly in src/, not subdirectories
        let glob = GlobBuilder::new(&normalized)
            .literal_separator(true)
            .build()?;
        builder.add(glob);
    }
    Ok(Some(builder.build()?))
}

/// Get literal paths from patterns (non-glob patterns).
fn get_literal_paths(patterns: &[String]) -> Vec<PathBuf> {
    patterns
        .iter()
        .filter(|p| !is_glob_pattern(p))
        .map(PathBuf::from)
        .collect()
}

/// Build a Gitignore matcher from default ignores and custom ignores.
/// This uses the same ignore crate machinery as the walker.
fn build_ignore_matcher(root: &Path, config: &WalkerConfig) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);

    // Add default ignores
    if config.use_default_ignores {
        for pattern in DEFAULT_IGNORES {
            // GitignoreBuilder::add expects gitignore-style patterns
            let _ = builder.add_line(None, pattern);
        }
    }

    // Add custom ignores
    for pattern in &config.custom_ignores {
        let _ = builder.add_line(None, pattern);
    }

    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Check if a file should be included based on walker configuration.
///
/// This is a convenience function that creates a FileFilter and checks a single file.
/// For checking multiple files, use FileFilter directly for better performance.
///
/// This function checks all ignore sources: default ignores, custom ignores,
/// .gitignore (if enabled), and .contextignore.
#[cfg(test)]
pub fn should_include_file(root: &Path, file_path: &Path, config: &WalkerConfig) -> bool {
    // Build a filter and check - this is less efficient than reusing a FileFilter
    // but maintains the simple API for tests
    match FileFilter::new(root, config) {
        Ok(filter) => filter.should_include(file_path),
        Err(_) => false,
    }
}

/// Walk directories and discover files based on configuration.
pub fn discover_files(root: &Path, config: &WalkerConfig) -> io::Result<Vec<FileEntry>> {
    let root = root.canonicalize()?;
    let mut entries = Vec::new();

    // Build ignore matcher for default and custom ignores
    let ignore_matcher = build_ignore_matcher(&root, config);

    // Build include glob set for filtering (only applies to root walk, not literal paths)
    let include_globset = build_include_globset(&root, &config.include_patterns)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    // Get literal paths (these are walked separately and bypass glob filtering)
    let literal_paths: Vec<PathBuf> = get_literal_paths(&config.include_patterns)
        .into_iter()
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .collect();

    // Determine if we need to walk from root (for glob patterns)
    let has_globs = config.include_patterns.iter().any(|p| is_glob_pattern(p));

    // Helper to process a single walk
    let mut process_walk = |start_path: &Path, apply_glob_filter: bool| -> io::Result<()> {
        if !start_path.exists() {
            eprintln!("Warning: path does not exist: {}", start_path.display());
            return Ok(());
        }

        let mut builder = WalkBuilder::new(start_path);

        // Configure gitignore handling
        builder.git_ignore(config.use_gitignore);
        builder.git_global(config.use_gitignore);
        builder.git_exclude(config.use_gitignore);

        // Look for .contextignore file
        builder.add_custom_ignore_filename(".contextignore");

        // Walk the directory
        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("Warning: {}", err);
                    continue;
                }
            };

            // Skip directories
            let file_type = match entry.file_type() {
                Some(ft) => ft,
                None => continue,
            };
            if file_type.is_dir() {
                continue;
            }

            let abs_path = entry.path().to_path_buf();

            // Calculate relative path from root
            let rel_path = match abs_path.strip_prefix(&root) {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            };

            // Apply default/custom ignore filtering
            if ignore_matcher
                .matched_path_or_any_parents(&rel_path, false)
                .is_ignore()
            {
                continue;
            }

            // Apply include glob filter only when walking from root for glob patterns
            // Literal paths bypass this filter - they're explicitly included
            if apply_glob_filter {
                if let Some(ref globset) = include_globset {
                    if !globset.is_match(&rel_path) {
                        continue;
                    }
                }
            }

            // Skip binary files (check for null bytes)
            if is_binary_file(&abs_path) {
                continue;
            }

            // Get file size
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            entries.push(FileEntry {
                absolute_path: abs_path,
                relative_path: rel_path,
                size,
            });
        }
        Ok(())
    };

    if config.include_patterns.is_empty() {
        // No patterns - walk everything from root
        process_walk(&root, false)?;
    } else {
        // Walk from root with glob filtering if there are glob patterns
        if has_globs {
            process_walk(&root, true)?;
        }

        // Walk literal paths WITHOUT glob filtering (they're explicitly included)
        for literal_path in &literal_paths {
            process_walk(literal_path, false)?;
        }
    }

    // Sort by relative path for consistent output
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    // Remove duplicates (can happen with overlapping patterns)
    entries.dedup_by(|a, b| a.absolute_path == b.absolute_path);

    // A scoped run that selects nothing is almost always a mistyped pattern;
    // say so instead of silently producing an empty result.
    if entries.is_empty() && config.has_scoping_includes() {
        eprintln!(
            "Warning: include patterns matched no files: {}",
            config.include_patterns.join(", ")
        );
    }

    Ok(entries)
}

/// Check if a file is likely binary by looking for null bytes.
fn is_binary_file(path: &Path) -> bool {
    use std::fs::File;
    use std::io::Read;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut buffer = [0u8; 8192];
    let bytes_read = match file.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return false,
    };

    buffer[..bytes_read].contains(&0)
}

/// Format file size in human-readable format.
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_pattern_matching() {
        // Test that glob patterns match correctly
        let root = Path::new("/project");
        let patterns = vec!["src/*.rs".to_string()];
        let globset = build_include_globset(root, &patterns).unwrap().unwrap();

        // Should match files directly in src/
        assert!(globset.is_match(Path::new("src/main.rs")));
        assert!(globset.is_match(Path::new("src/cli.rs")));

        // Should NOT match files in subdirectories
        assert!(!globset.is_match(Path::new("src/analytics/mod.rs")));
        assert!(!globset.is_match(Path::new("src/db/mod.rs")));
    }

    #[test]
    fn test_glob_pattern_recursive() {
        // Test recursive glob pattern
        let root = Path::new("/project");
        let patterns = vec!["src/**/*.rs".to_string()];
        let globset = build_include_globset(root, &patterns).unwrap().unwrap();

        // Should match all .rs files under src/
        assert!(globset.is_match(Path::new("src/main.rs")));
        assert!(globset.is_match(Path::new("src/analytics/mod.rs")));
        assert!(globset.is_match(Path::new("src/db/mod.rs")));

        // Should NOT match files outside src/
        assert!(!globset.is_match(Path::new("tests/test.rs")));
    }

    #[test]
    fn test_glob_pattern_absolute() {
        // Test absolute glob patterns are normalized
        let root = Path::new("/project");
        let patterns = vec!["/project/src/**/*.rs".to_string()];
        let globset = build_include_globset(root, &patterns).unwrap().unwrap();

        // Should match relative paths after normalization
        assert!(globset.is_match(Path::new("src/main.rs")));
        assert!(globset.is_match(Path::new("src/db/mod.rs")));

        // Should NOT match files outside src/
        assert!(!globset.is_match(Path::new("tests/test.rs")));
    }

    #[test]
    fn file_pattern_filter_supports_literals_globs_or_and_dot() {
        let root = Path::new("/project");
        let filter =
            FilePatternFilter::new(root, &["src".to_string(), "tests/**/*.rs".to_string()])
                .unwrap();

        assert!(filter.matches("src/main.rs"));
        assert!(filter.matches("tests/unit/parser.rs"));
        assert!(!filter.matches("docs/guide.md"));

        let file = FilePatternFilter::new(root, &["Cargo.toml".to_string()]).unwrap();
        assert!(file.matches("Cargo.toml"));
        assert!(!file.matches("Cargo.lock"));

        let all = FilePatternFilter::new(root, &[".".to_string()]).unwrap();
        assert!(all.matches("any/nested/path.rs"));
    }

    #[test]
    fn test_should_include_file_default_ignores() {
        let root = Path::new("/project");
        let config = WalkerConfig::default();

        // Default ignores should exclude node_modules, .git, target, etc.
        assert!(!should_include_file(
            root,
            Path::new("/project/node_modules/foo.js"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/.git/config"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/target/debug/main"),
            &config
        ));

        // Normal files should be included
        assert!(should_include_file(
            root,
            Path::new("/project/src/main.rs"),
            &config
        ));
    }

    #[test]
    fn test_should_include_file_custom_ignores() {
        let root = Path::new("/project");
        let config = WalkerConfig {
            use_gitignore: true,
            use_default_ignores: false, // Disable default ignores
            custom_ignores: vec!["vendor/".to_string(), "generated/".to_string()],
            include_patterns: vec![],
        };

        // Custom ignores should work
        assert!(!should_include_file(
            root,
            Path::new("/project/vendor/lib.rs"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/generated/types.rs"),
            &config
        ));

        // Other files should be included
        assert!(should_include_file(
            root,
            Path::new("/project/src/main.rs"),
            &config
        ));
    }

    #[test]
    fn test_should_include_file_include_patterns() {
        let root = Path::new("/project");
        let config = WalkerConfig {
            use_gitignore: true,
            use_default_ignores: true,
            custom_ignores: vec![],
            include_patterns: vec!["src/**/*.rs".to_string()],
        };

        // Only src/**/*.rs should match
        assert!(should_include_file(
            root,
            Path::new("/project/src/main.rs"),
            &config
        ));
        assert!(should_include_file(
            root,
            Path::new("/project/src/db/mod.rs"),
            &config
        ));

        // Other files should NOT match
        assert!(!should_include_file(
            root,
            Path::new("/project/tests/test.rs"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/build.rs"),
            &config
        ));
    }

    #[test]
    fn test_should_include_file_literal_directory() {
        // A bare directory path (no glob syntax) scopes like `<dir>/**`
        let root = Path::new("/project");
        let config = WalkerConfig {
            use_gitignore: true,
            use_default_ignores: true,
            custom_ignores: vec![],
            include_patterns: vec!["src".to_string()],
        };

        assert!(should_include_file(
            root,
            Path::new("/project/src/main.rs"),
            &config
        ));
        assert!(should_include_file(
            root,
            Path::new("/project/src/db/mod.rs"),
            &config
        ));

        assert!(!should_include_file(
            root,
            Path::new("/project/lib/util.rs"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/build.rs"),
            &config
        ));
    }

    #[test]
    fn test_should_include_file_wildcard_ignores() {
        let root = Path::new("/project");
        let config = WalkerConfig::default();

        // Wildcard patterns like *.png, *.lock should be ignored
        assert!(!should_include_file(
            root,
            Path::new("/project/assets/logo.png"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/Cargo.lock"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/package-lock.json"),
            &config
        ));

        // But similar-named files that don't match should be fine
        assert!(should_include_file(
            root,
            Path::new("/project/src/lockfile.rs"),
            &config
        ));
    }

    #[test]
    fn test_should_include_file_no_false_positives() {
        let root = Path::new("/project");
        let config = WalkerConfig::default();

        // Hidden files (starting with .) should be filtered - matching WalkBuilder default
        assert!(!should_include_file(
            root,
            Path::new("/project/.env"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/.envrc"),
            &config
        ));
        assert!(!should_include_file(
            root,
            Path::new("/project/.github/workflows/ci.yml"),
            &config
        ));

        // Non-hidden files with similar names should be included
        assert!(should_include_file(
            root,
            Path::new("/project/src/env.rs"),
            &config
        ));
        assert!(should_include_file(
            root,
            Path::new("/project/src/dotenv.rs"),
            &config
        ));
    }
}
