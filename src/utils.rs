//! Shared utility functions for ctx.
//!
//! This module contains helper functions used across multiple command modules.

use std::collections::BTreeSet;

/// Stopwords excluded from lexical token matching: articles, prepositions, and
/// generic task verbs that carry no file-identifying signal (so "add a new X"
/// matches on "X", not on "add"/"new").
const LEXICAL_STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "to", "for", "with", "and", "or", "by", "as", "at", "is",
    "are", "be", "it", "this", "that", "from", "into", "via", "add", "new", "make", "use", "using",
];

/// Split a string into normalized lexical tokens for path / identifier matching.
///
/// Lowercases, splits on any non-alphanumeric character **and** on camelCase
/// boundaries (so `parseSolidity` → `parse`, `solidity`), drops tokens of length
/// ≤ 1, and removes a small stopword set. Used to score how strongly a task
/// description lexically overlaps a file path or symbol name — a high-precision
/// relevance signal that embedding similarity can miss.
pub fn lexical_tokens(s: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut current = String::new();
    // Was the previous char a lowercase letter or digit? Used to detect the
    // lower→upper camelCase boundary that starts a new word.
    let mut prev_lower_or_digit = false;

    for ch in s.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower_or_digit && !current.is_empty() {
                push_token(&mut tokens, &current);
                current.clear();
            }
            current.extend(ch.to_lowercase());
            prev_lower_or_digit = ch.is_lowercase() || ch.is_numeric();
        } else {
            if !current.is_empty() {
                push_token(&mut tokens, &current);
                current.clear();
            }
            prev_lower_or_digit = false;
        }
    }
    if !current.is_empty() {
        push_token(&mut tokens, &current);
    }
    tokens
}

/// Insert a normalized token, dropping length-≤-1 tokens and stopwords.
fn push_token(tokens: &mut BTreeSet<String>, tok: &str) {
    if tok.len() > 1 && !LEXICAL_STOPWORDS.contains(&tok) {
        tokens.insert(tok.to_string());
    }
}

/// Truncate a string with ellipsis, respecting UTF-8 char boundaries.
///
/// If the string is longer than `max` characters, it will be truncated
/// to `max - 3` characters followed by "...".
///
/// # Examples
///
/// ```ignore
/// assert_eq!(truncate_str("hello world", 8), "hello...");
/// assert_eq!(truncate_str("short", 10), "short");
/// ```
pub fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let target = max.saturating_sub(3);
        let mut end = target;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Truncate a path from the beginning, respecting UTF-8 char boundaries.
///
/// Unlike `truncate_str`, this keeps the end of the string (which typically
/// contains the filename) and truncates from the beginning.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(truncate_path("/very/long/path/to/file.rs", 15), "...to/file.rs");
/// assert_eq!(truncate_path("short.rs", 20), "short.rs");
/// ```
pub fn truncate_path(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let target = s.len() - max + 3;
        let mut start = target;
        while start < s.len() && !s.is_char_boundary(start) {
            start += 1;
        }
        format!("...{}", &s[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        lexical_tokens(s).into_iter().collect()
    }

    #[test]
    fn test_lexical_tokens_path_splits_on_separators() {
        // Path splits on '/' and '.'; tokens are lowercased, deduped, sorted.
        assert_eq!(
            toks("src/embeddings/openai.rs"),
            vec!["embeddings", "openai", "rs", "src"]
        );
    }

    #[test]
    fn test_lexical_tokens_underscore_splits_each_word() {
        // snake_case splits into individual words (no combined "run_sql" token).
        assert_eq!(toks("run_sql_duckdb"), vec!["duckdb", "run", "sql"]);
    }

    #[test]
    fn test_lexical_tokens_camelcase() {
        // camelCase and PascalCase split into words, lowercased.
        assert_eq!(toks("parseSolidity"), vec!["parse", "solidity"]);
        assert_eq!(toks("SmartConfig"), vec!["config", "smart"]);
        // digits stay attached to their word
        assert_eq!(toks("v1Symbols"), vec!["symbols", "v1"]);
    }

    #[test]
    fn test_lexical_tokens_stopwords_and_length() {
        // Stopwords and single chars are dropped; result is deduped + sorted.
        assert_eq!(
            toks("add a new output format to ctx sql"),
            vec!["ctx", "format", "output", "sql"]
        );
        assert_eq!(
            toks("generate embeddings with openai"),
            vec!["embeddings", "generate", "openai"]
        );
    }

    #[test]
    fn test_truncate_str_ascii() {
        // No truncation needed
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");

        // Truncation needed
        assert_eq!(truncate_str("hello world", 8), "hello...");
        assert_eq!(truncate_str("abcdefghij", 7), "abcd...");
    }

    #[test]
    fn test_truncate_str_unicode() {
        // Box drawing (─ is 3 bytes)
        let box_line = "┌────────────────────┐";
        let result = truncate_str(box_line, 10);
        assert!(result.ends_with("..."));
        // Should not panic

        // Emoji (🎉 is 4 bytes)
        let emoji = "Hello 🎉🎊🎁 World";
        let result = truncate_str(emoji, 10);
        assert!(result.ends_with("..."));

        // Chinese (each char is 3 bytes)
        let chinese = "你好世界测试";
        let result = truncate_str(chinese, 8);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_path_ascii() {
        // No truncation needed
        assert_eq!(truncate_path("src/main.rs", 20), "src/main.rs");

        // Truncation needed - keeps end of path
        let result = truncate_path("/very/long/path/to/file.rs", 15);
        assert!(result.starts_with("..."));
        assert!(result.contains("file.rs"));
    }

    #[test]
    fn test_truncate_path_unicode() {
        // Path with Unicode
        let path = "/home/用户/项目/文件.rs";
        let result = truncate_path(path, 15);
        assert!(result.starts_with("..."));
        // Should not panic

        // Path with emoji folder names
        let path = "/home/📁/🎉/file.rs";
        let result = truncate_path(path, 12);
        assert!(result.starts_with("..."));
    }

    #[test]
    fn test_truncate_edge_cases() {
        // Very short max
        assert_eq!(truncate_str("hello", 3), "...");
        assert_eq!(truncate_str("hi", 3), "hi");

        // Empty string
        assert_eq!(truncate_str("", 10), "");
        assert_eq!(truncate_path("", 10), "");
    }
}
