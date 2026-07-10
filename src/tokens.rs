//! Token counting for LLM context management.
//!
//! This module provides token counting functionality using tiktoken-rs,
//! compatible with OpenAI models (GPT-4, GPT-3.5-turbo, etc.).

use std::path::Path;
use std::str::FromStr;
use tiktoken_rs::{cl100k_base, o200k_base, p50k_base, CoreBPE};

/// Result of token counting for a text or file.
#[derive(Debug, Clone)]
pub struct TokenCount {
    /// Number of tokens
    pub count: usize,
    /// Encoding used (e.g., "cl100k_base")
    #[allow(dead_code)] // Part of public API
    pub encoding: String,
    /// Original text length in characters (if available)
    #[allow(dead_code)] // Part of public API
    pub char_count: Option<usize>,
}

impl TokenCount {
    /// Create a new TokenCount.
    #[allow(dead_code)] // Part of public API
    pub fn new(count: usize, encoding: &str) -> Self {
        Self {
            count,
            encoding: encoding.to_string(),
            char_count: None,
        }
    }

    /// Create a new TokenCount with character count.
    pub fn with_char_count(count: usize, encoding: &str, char_count: usize) -> Self {
        Self {
            count,
            encoding: encoding.to_string(),
            char_count: Some(char_count),
        }
    }
}

/// Supported tokenizer encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// cl100k_base - GPT-4, GPT-3.5-turbo, text-embedding-ada-002
    #[default]
    Cl100kBase,
    /// o200k_base - GPT-4o, GPT-4.1, o1, o3
    O200kBase,
    /// p50k_base - Codex, text-davinci-002/003
    P50kBase,
}

impl Encoding {
    /// Get the encoding name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
            Self::P50kBase => "p50k_base",
        }
    }
}

impl FromStr for Encoding {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cl100k_base" | "cl100k" => Ok(Encoding::Cl100kBase),
            "o200k_base" | "o200k" => Ok(Encoding::O200kBase),
            "p50k_base" | "p50k" => Ok(Encoding::P50kBase),
            _ => Err(()),
        }
    }
}

/// Get the BPE tokenizer for the specified encoding.
pub fn get_bpe(encoding: Encoding) -> Result<CoreBPE, String> {
    match encoding {
        Encoding::Cl100kBase => cl100k_base().map_err(|e| e.to_string()),
        Encoding::O200kBase => o200k_base().map_err(|e| e.to_string()),
        Encoding::P50kBase => p50k_base().map_err(|e| e.to_string()),
    }
}

/// Count tokens in a text string using the default encoding (cl100k_base).
#[allow(dead_code)] // Part of public API
pub fn count_tokens(text: &str) -> Result<usize, String> {
    count_tokens_with_encoding(text, Encoding::default())
}

/// Count tokens in a text string using the specified encoding.
pub fn count_tokens_with_encoding(text: &str, encoding: Encoding) -> Result<usize, String> {
    let bpe = get_bpe(encoding)?;
    Ok(bpe.encode_with_special_tokens(text).len())
}

/// Count tokens with full details.
pub fn count_tokens_detailed(text: &str, encoding: Encoding) -> Result<TokenCount, String> {
    let bpe = get_bpe(encoding)?;
    let count = bpe.encode_with_special_tokens(text).len();
    Ok(TokenCount::with_char_count(
        count,
        encoding.as_str(),
        text.len(),
    ))
}

/// Count tokens in a file.
pub fn count_file_tokens(path: &Path, encoding: Encoding) -> Result<TokenCount, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;
    count_tokens_detailed(&content, encoding)
}

/// File with its token count.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Part of public API
pub struct FileTokens {
    /// File path
    pub path: String,
    /// Token count
    pub tokens: usize,
    /// File size in bytes
    pub size_bytes: usize,
}

/// Trait for types that have a token count field.
///
/// Used by `select_by_token_budget` to generically select items
/// that fit within a token budget.
pub trait HasTokenCount {
    /// Get the token count for this item.
    fn token_count(&self) -> usize;
}

impl HasTokenCount for FileTokens {
    fn token_count(&self) -> usize {
        self.tokens
    }
}

/// Select items that fit within a token budget (generic version).
///
/// Returns items in order of selection (highest priority first) that fit
/// within the specified token limit. Items are selected greedily - if an item
/// doesn't fit, it's skipped and the next item is tried.
///
/// # Arguments
/// * `items` - List of items with token counts, in priority order
/// * `max_tokens` - Maximum total tokens to include
///
/// # Returns
/// * Tuple of (selected items, total tokens used, number of items omitted)
pub fn select_by_token_budget<T: HasTokenCount>(
    items: Vec<T>,
    max_tokens: usize,
) -> (Vec<T>, usize, usize) {
    let mut selected = Vec::new();
    let mut total_tokens = 0;
    let mut omitted = 0;

    for item in items {
        if total_tokens + item.token_count() <= max_tokens {
            total_tokens += item.token_count();
            selected.push(item);
        } else {
            omitted += 1;
        }
    }

    (selected, total_tokens, omitted)
}

/// Select files that fit within a token budget.
///
/// Returns files in order of selection (highest priority first) that fit
/// within the specified token limit. Files are selected greedily - if a file
/// doesn't fit, it's skipped and the next file is tried.
///
/// # Arguments
/// * `files` - List of files with their token counts, in priority order
/// * `max_tokens` - Maximum total tokens to include
///
/// # Returns
/// * Tuple of (selected files, total tokens used, number of files omitted)
#[allow(dead_code)] // Part of public API
pub fn select_files_by_tokens(
    files: &[FileTokens],
    max_tokens: usize,
) -> (Vec<FileTokens>, usize, usize) {
    let mut selected = Vec::new();
    let mut total_tokens = 0;
    let mut omitted = 0;

    for file in files {
        if total_tokens + file.tokens <= max_tokens {
            total_tokens += file.tokens;
            selected.push(file.clone());
        } else {
            omitted += 1;
        }
    }

    (selected, total_tokens, omitted)
}

/// Estimate tokens without loading the tokenizer (fast approximation).
///
/// Uses a rough heuristic of ~4 characters per token for English text.
/// This is useful for quick estimates when exact counts aren't needed.
#[allow(dead_code)] // Part of public API
pub fn estimate_tokens(text: &str) -> usize {
    // Rough approximation: ~4 characters per token for English
    // This is a conservative estimate (actual tokens are usually fewer)
    text.len().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_basic() {
        let text = "Hello, world!";
        let count = count_tokens(text).unwrap();
        assert!(count > 0);
        assert!(count < text.len()); // Tokens should be fewer than characters
    }

    #[test]
    fn test_count_tokens_encoding() {
        let text = "fn main() { println!(\"Hello\"); }";

        let cl100k = count_tokens_with_encoding(text, Encoding::Cl100kBase).unwrap();
        let o200k = count_tokens_with_encoding(text, Encoding::O200kBase).unwrap();

        // Both should give reasonable counts
        assert!(cl100k > 0 && cl100k < 50);
        assert!(o200k > 0 && o200k < 50);
    }

    #[test]
    fn test_count_tokens_detailed() {
        let text = "Hello, world!";
        let result = count_tokens_detailed(text, Encoding::Cl100kBase).unwrap();

        assert!(result.count > 0);
        assert_eq!(result.encoding, "cl100k_base");
        assert_eq!(result.char_count, Some(text.len()));
    }

    #[test]
    fn test_select_files_by_tokens() {
        let files = vec![
            FileTokens {
                path: "a.rs".to_string(),
                tokens: 100,
                size_bytes: 400,
            },
            FileTokens {
                path: "b.rs".to_string(),
                tokens: 200,
                size_bytes: 800,
            },
            FileTokens {
                path: "c.rs".to_string(),
                tokens: 150,
                size_bytes: 600,
            },
        ];

        // All fit
        let (selected, total, omitted) = select_files_by_tokens(&files, 500);
        assert_eq!(selected.len(), 3);
        assert_eq!(total, 450);
        assert_eq!(omitted, 0);

        // Only first two fit
        let (selected, total, omitted) = select_files_by_tokens(&files, 300);
        assert_eq!(selected.len(), 2);
        assert_eq!(total, 300);
        assert_eq!(omitted, 1);

        // Only first fits
        let (selected, total, omitted) = select_files_by_tokens(&files, 150);
        assert_eq!(selected.len(), 1);
        assert_eq!(total, 100);
        assert_eq!(omitted, 2);
    }

    #[test]
    fn test_encoding_from_str() {
        use std::str::FromStr;
        assert_eq!(
            Encoding::from_str("cl100k_base").ok(),
            Some(Encoding::Cl100kBase)
        );
        assert_eq!(
            Encoding::from_str("o200k_base").ok(),
            Some(Encoding::O200kBase)
        );
        assert_eq!(
            Encoding::from_str("p50k_base").ok(),
            Some(Encoding::P50kBase)
        );
        assert_eq!(Encoding::from_str("unknown").ok(), None);
    }

    #[test]
    fn test_estimate_tokens() {
        let text = "Hello, world!"; // 13 chars
        let estimate = estimate_tokens(text);
        // Should be roughly 3-4 tokens based on ~4 chars per token
        assert!((3..=5).contains(&estimate));
    }
}
