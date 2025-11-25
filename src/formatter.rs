use std::path::Path;

use crate::walker::FileEntry;

/// Trait for formatting context output.
pub trait Formatter {
    /// Format the project tree block.
    fn format_tree(&self, tree: &str) -> String;

    /// Format a single file block.
    fn format_file(&self, entry: &FileEntry, content: &str) -> String;

    /// Wrap the tree block and files block into final output.
    fn wrap(&self, tree_block: Option<&str>, files_block: &str) -> String;

    /// Get the opening wrapper for streaming output.
    fn stream_start(&self, tree_block: Option<&str>) -> String;

    /// Get the closing wrapper for streaming output.
    fn stream_end(&self) -> String;

    /// Get the separator between file blocks.
    fn separator(&self) -> &'static str;
}

/// XML formatter.
pub struct XmlFormatter;

impl Formatter for XmlFormatter {
    fn format_tree(&self, tree: &str) -> String {
        format!("<project_tree>\n{}</project_tree>", tree)
    }

    fn format_file(&self, entry: &FileEntry, content: &str) -> String {
        let filename = entry
            .relative_path
            .file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();

        let path = format_path_for_output(&entry.relative_path);

        format!(
            "<file name=\"{}\" path=\"{}\">\n{}\n</file>",
            filename,
            path,
            content.trim()
        )
    }

    fn wrap(&self, tree_block: Option<&str>, files_block: &str) -> String {
        match tree_block {
            Some(tree) => format!(
                "<context>\n{}\n<project_files>\n{}\n</project_files>\n</context>",
                tree, files_block
            ),
            None => format!(
                "<context>\n<project_files>\n{}\n</project_files>\n</context>",
                files_block
            ),
        }
    }

    fn stream_start(&self, tree_block: Option<&str>) -> String {
        match tree_block {
            Some(tree) => format!("<context>\n{}\n<project_files>", tree),
            None => "<context>\n<project_files>".to_string(),
        }
    }

    fn stream_end(&self) -> String {
        "</project_files>\n</context>".to_string()
    }

    fn separator(&self) -> &'static str {
        "\n"
    }
}

/// Markdown formatter.
pub struct MarkdownFormatter;

impl Formatter for MarkdownFormatter {
    fn format_tree(&self, tree: &str) -> String {
        format!("## Project Tree\n\n```\n{}```", tree)
    }

    fn format_file(&self, entry: &FileEntry, content: &str) -> String {
        let path = format_path_for_output(&entry.relative_path);
        let extension = entry
            .relative_path
            .extension()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();

        format!("## {}\n\n```{}\n{}\n```", path, extension, content.trim())
    }

    fn wrap(&self, tree_block: Option<&str>, files_block: &str) -> String {
        match tree_block {
            Some(tree) => format!("# Project Context\n\n{}\n\n{}", tree, files_block),
            None => format!("# Project Context\n\n{}", files_block),
        }
    }

    fn stream_start(&self, tree_block: Option<&str>) -> String {
        match tree_block {
            Some(tree) => format!("# Project Context\n\n{}\n", tree),
            None => "# Project Context\n".to_string(),
        }
    }

    fn stream_end(&self) -> String {
        String::new()
    }

    fn separator(&self) -> &'static str {
        "\n\n"
    }
}

/// Plain text formatter.
pub struct PlainFormatter;

impl Formatter for PlainFormatter {
    fn format_tree(&self, tree: &str) -> String {
        format!("=== PROJECT TREE ===\n\n{}", tree)
    }

    fn format_file(&self, entry: &FileEntry, content: &str) -> String {
        let path = format_path_for_output(&entry.relative_path);
        format!("=== {} ===\n\n{}\n", path, content.trim())
    }

    fn wrap(&self, tree_block: Option<&str>, files_block: &str) -> String {
        match tree_block {
            Some(tree) => format!("{}\n{}", tree, files_block),
            None => files_block.to_string(),
        }
    }

    fn stream_start(&self, tree_block: Option<&str>) -> String {
        match tree_block {
            Some(tree) => format!("{}\n", tree),
            None => String::new(),
        }
    }

    fn stream_end(&self) -> String {
        String::new()
    }

    fn separator(&self) -> &'static str {
        "\n"
    }
}

/// Format a path for output (use forward slashes, prefix with /).
fn format_path_for_output(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");
    if path_str.starts_with('/') {
        path_str.to_string()
    } else {
        format!("/{}", path_str)
    }
}

/// Get a formatter instance based on format name.
pub fn get_formatter(format: &crate::cli::OutputFormat) -> Box<dyn Formatter> {
    use crate::cli::OutputFormat;

    match format {
        OutputFormat::Xml => Box::new(XmlFormatter),
        OutputFormat::Markdown | OutputFormat::Md => Box::new(MarkdownFormatter),
        OutputFormat::Plain => Box::new(PlainFormatter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_entry(rel_path: &str) -> FileEntry {
        FileEntry {
            absolute_path: PathBuf::from("/project").join(rel_path),
            relative_path: PathBuf::from(rel_path),
            size: 100,
        }
    }

    #[test]
    fn test_xml_formatter() {
        let formatter = XmlFormatter;
        let entry = make_entry("src/main.rs");
        let output = formatter.format_file(&entry, "fn main() {}");

        assert!(output.contains("<file name=\"main.rs\""));
        assert!(output.contains("path=\"/src/main.rs\""));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn test_markdown_formatter() {
        let formatter = MarkdownFormatter;
        let entry = make_entry("src/main.rs");
        let output = formatter.format_file(&entry, "fn main() {}");

        assert!(output.contains("## /src/main.rs"));
        assert!(output.contains("```rs"));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn test_plain_formatter() {
        let formatter = PlainFormatter;
        let entry = make_entry("src/main.rs");
        let output = formatter.format_file(&entry, "fn main() {}");

        assert!(output.contains("=== /src/main.rs ==="));
        assert!(output.contains("fn main() {}"));
    }
}
