//! Shared utility functions for ctx.
//!
//! This module contains helper functions used across multiple command modules.

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
