//! Output formatters for generated context.
//!
//! Renders discovered files into one of four formats, selected via
//! [`OutputFormat`]: XML (default, best for most LLMs), Markdown, JSON,
//! or plain text. Use [`get_formatter`] to obtain the right
//! [`Formatter`] implementation for a format.

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

impl XmlFormatter {
    /// Escape special XML characters in text content.
    /// Order matters: & must be escaped first to avoid double-escaping.
    fn escape_xml_text(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// Escape special characters in XML attribute values.
    /// Includes quotes in addition to text escapes.
    fn escape_xml_attr(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

impl Formatter for XmlFormatter {
    fn format_tree(&self, tree: &str) -> String {
        format!(
            "<project_tree>\n{}</project_tree>",
            Self::escape_xml_text(tree)
        )
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
            Self::escape_xml_attr(&filename),
            Self::escape_xml_attr(&path),
            Self::escape_xml_text(content.trim())
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

/// JSON formatter.
///
/// Outputs structured JSON with the format:
/// ```json
/// {
///   "tree": "...",
///   "files": [
///     { "name": "main.rs", "path": "/src/main.rs", "content": "..." }
///   ]
/// }
/// ```
///
/// Note: JSON streaming outputs newline-delimited JSON objects (NDJSON) for each file,
/// since partial JSON arrays aren't valid JSON.
pub struct JsonFormatter;

impl JsonFormatter {
    /// Escape a string for JSON (handles control characters, quotes, backslashes)
    fn escape_json_string(s: &str) -> String {
        serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s))
    }
}

impl Formatter for JsonFormatter {
    fn format_tree(&self, tree: &str) -> String {
        // For JSON, tree is embedded in the wrapper, not standalone
        Self::escape_json_string(tree)
    }

    fn format_file(&self, entry: &FileEntry, content: &str) -> String {
        let filename = entry
            .relative_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let path = format_path_for_output(&entry.relative_path);

        // Create a JSON object for this file
        format!(
            r#"{{"name":{},"path":{},"content":{}}}"#,
            Self::escape_json_string(&filename),
            Self::escape_json_string(&path),
            Self::escape_json_string(content.trim())
        )
    }

    fn wrap(&self, tree_block: Option<&str>, files_block: &str) -> String {
        // files_block contains comma-separated JSON objects
        // Wrap them in an array and add tree if present
        match tree_block {
            Some(tree) => format!(
                r#"{{"tree":{},"files":[{}]}}"#,
                tree, // Already JSON-escaped from format_tree
                files_block
            ),
            None => format!(r#"{{"files":[{}]}}"#, files_block),
        }
    }

    fn stream_start(&self, tree_block: Option<&str>) -> String {
        // For streaming, output NDJSON format (one JSON object per line)
        // Start with a metadata object containing the tree
        match tree_block {
            Some(tree) => format!(r#"{{"type":"tree","content":{}}}"#, tree),
            None => String::new(),
        }
    }

    fn stream_end(&self) -> String {
        // For NDJSON, no closing tag needed
        String::new()
    }

    fn separator(&self) -> &'static str {
        // For NDJSON streaming, use newline separator
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

/// Output format for context generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// XML format (default)
    #[default]
    Xml,
    /// Markdown format
    Markdown,
    /// Plain text format
    Plain,
    /// JSON format
    Json,
}

impl OutputFormat {}

impl std::str::FromStr for OutputFormat {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xml" => Ok(Self::Xml),
            "markdown" | "md" => Ok(Self::Markdown),
            "plain" => Ok(Self::Plain),
            "json" => Ok(Self::Json),
            _ => Err(()),
        }
    }
}

/// Get a formatter instance based on format.
pub fn get_formatter(format: OutputFormat) -> Box<dyn Formatter> {
    match format {
        OutputFormat::Xml => Box::new(XmlFormatter),
        OutputFormat::Markdown => Box::new(MarkdownFormatter),
        OutputFormat::Plain => Box::new(PlainFormatter),
        OutputFormat::Json => Box::new(JsonFormatter),
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
    fn test_xml_formatter_escapes_content() {
        let formatter = XmlFormatter;
        let entry = make_entry("src/test.rs");
        // Content with XML special characters
        let content = r#"if x < 10 && y > 5 { println!("<tag>"); }"#;
        let output = formatter.format_file(&entry, content);

        // Verify special characters are escaped
        assert!(output.contains("x &lt; 10"));
        assert!(output.contains("&amp;&amp;"));
        assert!(output.contains("y &gt; 5"));
        assert!(output.contains("&lt;tag&gt;"));
        // Raw characters should NOT appear in content
        assert!(!output.contains("< 10"));
        assert!(!output.contains("> 5"));
    }

    #[test]
    fn test_xml_formatter_escapes_attributes() {
        let formatter = XmlFormatter;
        // File path with special characters (edge case)
        let entry = make_entry("src/test&file.rs");
        let output = formatter.format_file(&entry, "content");

        // Verify ampersand in filename is escaped
        assert!(output.contains("name=\"test&amp;file.rs\""));
        assert!(output.contains("path=\"/src/test&amp;file.rs\""));
    }

    #[test]
    fn test_xml_formatter_escapes_quotes_in_attrs() {
        // Test the escape function directly since filenames with quotes are rare
        let escaped = XmlFormatter::escape_xml_attr(r#"file"name'test"#);
        assert_eq!(escaped, "file&quot;name&apos;test");
    }

    #[test]
    fn test_xml_formatter_escapes_tree() {
        let formatter = XmlFormatter;
        let tree = "src/\n  <generated>/\n  test&file.rs";
        let output = formatter.format_tree(tree);

        assert!(output.contains("&lt;generated&gt;"));
        assert!(output.contains("test&amp;file.rs"));
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

    #[test]
    fn test_json_formatter_file() {
        let formatter = JsonFormatter;
        let entry = make_entry("src/main.rs");
        let output = formatter.format_file(&entry, "fn main() {}");

        // Parse as JSON to verify validity
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON");
        assert_eq!(parsed["name"], "main.rs");
        assert_eq!(parsed["path"], "/src/main.rs");
        assert_eq!(parsed["content"], "fn main() {}");
    }

    #[test]
    fn test_json_formatter_escapes_special_chars() {
        let formatter = JsonFormatter;
        let entry = make_entry("src/test.rs");
        // Content with quotes, backslashes, newlines, and angle brackets
        let content = r#"let s = "hello\nworld"; // <test>"#;
        let output = formatter.format_file(&entry, content);

        // Parse as JSON to verify validity (will fail if escaping is broken)
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON");
        assert_eq!(parsed["content"], content);
    }

    #[test]
    fn test_json_formatter_wrap_with_tree() {
        let formatter = JsonFormatter;
        let entry = make_entry("src/main.rs");
        let file_block = formatter.format_file(&entry, "fn main() {}");
        let tree = formatter.format_tree("src/\n  main.rs");
        let output = formatter.wrap(Some(&tree), &file_block);

        // Parse as JSON to verify validity
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON");
        assert!(parsed["tree"].is_string());
        assert!(parsed["files"].is_array());
        assert_eq!(parsed["files"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_json_formatter_wrap_without_tree() {
        let formatter = JsonFormatter;
        let entry = make_entry("src/main.rs");
        let file_block = formatter.format_file(&entry, "fn main() {}");
        let output = formatter.wrap(None, &file_block);

        // Parse as JSON to verify validity
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON");
        assert!(parsed.get("tree").is_none());
        assert!(parsed["files"].is_array());
    }

    #[test]
    fn test_json_formatter_multiple_files() {
        let formatter = JsonFormatter;
        let entry1 = make_entry("src/main.rs");
        let entry2 = make_entry("src/lib.rs");
        let file1 = formatter.format_file(&entry1, "fn main() {}");
        let file2 = formatter.format_file(&entry2, "pub mod test;");
        // For wrap() (non-streaming), use comma separator
        let files_block = format!("{},{}", file1, file2);
        let output = formatter.wrap(None, &files_block);

        // Parse as JSON to verify validity
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("Invalid JSON");
        assert_eq!(parsed["files"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["files"][0]["name"], "main.rs");
        assert_eq!(parsed["files"][1]["name"], "lib.rs");
    }

    #[test]
    fn test_json_formatter_streaming() {
        let formatter = JsonFormatter;
        let entry1 = make_entry("src/main.rs");
        let entry2 = make_entry("src/lib.rs");
        let file1 = formatter.format_file(&entry1, "fn main() {}");
        let file2 = formatter.format_file(&entry2, "pub mod test;");

        // Streaming uses newline separator (NDJSON format)
        let separator = formatter.separator();
        assert_eq!(separator, "\n");

        // Each line should be valid JSON
        let parsed1: serde_json::Value = serde_json::from_str(&file1).expect("Invalid JSON");
        let parsed2: serde_json::Value = serde_json::from_str(&file2).expect("Invalid JSON");
        assert_eq!(parsed1["name"], "main.rs");
        assert_eq!(parsed2["name"], "lib.rs");
    }
}
