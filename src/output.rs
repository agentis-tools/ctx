//! Context generation from discovered files.
//!
//! Turns a set of [`FileEntry`] values (from [`crate::walker`]) into final
//! formatted output: [`generate_context`] builds the full result in memory
//! and returns a [`ContextResult`] with size/count stats, while
//! [`stream_context`] writes files to a writer as they are read, for
//! pipe-friendly streaming.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::formatter::{get_formatter, OutputFormat};
use crate::limits::MAX_FILE_BYTES;
use crate::tree::generate_tree;
use crate::walker::{secure_file_path, FileEntry};

/// Result of context generation.
pub struct ContextResult {
    pub content: String,
    pub file_count: usize,
    pub total_size: u64,
    /// Byte length of the generated output (used for token estimation).
    pub output_bytes: usize,
}

/// Generate context output from file entries.
pub fn generate_context(
    root: &Path,
    entries: &[FileEntry],
    format: OutputFormat,
    include_tree: bool,
    show_sizes: bool,
) -> io::Result<ContextResult> {
    let formatter = get_formatter(format);

    // Determine root name for tree
    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    // Generate tree if requested
    let tree_block = if include_tree {
        let tree = generate_tree(&root_name, entries, show_sizes);
        Some(formatter.format_tree(&tree))
    } else {
        None
    };

    // Generate file blocks
    let mut file_blocks = Vec::new();
    let mut total_size = 0u64;
    let mut processed_count = 0usize;

    for entry in entries {
        match secure_file_path(root, &entry.relative_path).and_then(|path| read_file_content(&path))
        {
            Ok(content) => {
                let block = formatter.format_file(entry, &content);
                file_blocks.push(block);
                total_size += entry.size;
                processed_count += 1;
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not read {}: {}",
                    entry.relative_path.display(),
                    e
                );
            }
        }
    }

    // Join file blocks
    let separator = get_separator(format);
    let files_block = file_blocks.join(&separator);

    // Wrap everything
    let content = formatter.wrap(tree_block.as_deref(), &files_block);

    Ok(ContextResult {
        output_bytes: content.len(),
        content,
        file_count: processed_count,
        total_size,
    })
}

/// Stream context output, printing each file as it's processed.
pub fn stream_context(
    root: &Path,
    entries: &[FileEntry],
    format: OutputFormat,
    include_tree: bool,
    show_sizes: bool,
) -> io::Result<ContextResult> {
    let formatter = get_formatter(format);
    let mut stdout = io::stdout().lock();

    // Determine root name for tree
    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    // Generate tree if requested
    let tree_block = if include_tree {
        let tree = generate_tree(&root_name, entries, show_sizes);
        Some(formatter.format_tree(&tree))
    } else {
        None
    };

    // Print opening (skip if empty to avoid blank lines in NDJSON)
    let start = formatter.stream_start(tree_block.as_deref());
    let mut output_bytes = 0usize;
    if !start.is_empty() {
        writeln!(stdout, "{}", start)?;
        output_bytes += start.len() + 1;
    }

    // Stream file blocks
    let mut total_size = 0u64;
    let mut processed_count = 0usize;
    let separator = formatter.separator();

    for (i, entry) in entries.iter().enumerate() {
        match secure_file_path(root, &entry.relative_path).and_then(|path| read_file_content(&path))
        {
            Ok(content) => {
                let block = formatter.format_file(entry, &content);
                if i > 0 {
                    write!(stdout, "{}", separator)?;
                    output_bytes += separator.len();
                }
                write!(stdout, "{}", block)?;
                output_bytes += block.len();
                stdout.flush()?;
                total_size += entry.size;
                processed_count += 1;
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not read {}: {}",
                    entry.relative_path.display(),
                    e
                );
            }
        }
    }

    // Print closing
    let end = formatter.stream_end();
    if !end.is_empty() {
        writeln!(stdout, "\n{}", end)?;
        output_bytes += end.len() + 2;
    } else {
        writeln!(stdout)?;
        output_bytes += 1;
    }

    Ok(ContextResult {
        content: String::new(), // Not used in streaming mode
        file_count: processed_count,
        total_size,
        output_bytes,
    })
}

/// Read file content, handling encoding gracefully.
pub fn read_file_content(path: &Path) -> io::Result<String> {
    read_file_content_with_limit(path, MAX_FILE_BYTES)
}

/// Read at most `max_bytes` from a file while retaining the global per-file cap.
///
/// Callers that expose a smaller aggregate response budget can use this helper
/// to avoid reading a full file only to truncate it after assembling a response.
pub fn read_file_content_with_limit(path: &Path, max_bytes: u64) -> io::Result<String> {
    let file = fs::File::open(path)?;
    if let Ok(metadata) = file.metadata() {
        if metadata.len() > MAX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                format!("file exceeds the {} byte per-file limit", MAX_FILE_BYTES),
            ));
        }
    }

    let read_limit = max_bytes.min(MAX_FILE_BYTES);
    let mut bytes = Vec::new();
    file.take(read_limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > read_limit {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            format!("file exceeds the {} byte read limit", read_limit),
        ));
    }

    // Try UTF-8 first
    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(s),
        Err(_) => {
            // Fall back to lossy conversion
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

/// Get the separator between file blocks based on format.
fn get_separator(format: OutputFormat) -> String {
    match format {
        OutputFormat::Xml => "\n".to_string(),
        OutputFormat::Markdown => "\n\n".to_string(),
        OutputFormat::Plain => "\n".to_string(),
        OutputFormat::Json => ",".to_string(), // Comma for non-streaming JSON array
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_separator_xml() {
        assert_eq!(get_separator(OutputFormat::Xml), "\n");
    }

    #[test]
    fn test_separator_markdown() {
        assert_eq!(get_separator(OutputFormat::Markdown), "\n\n");
    }
}
