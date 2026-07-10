//! ASCII project tree rendering.
//!
//! Builds the `├──`/`└──` style directory tree shown at the top of
//! generated context. Entry point: [`generate_tree`].

use std::collections::BTreeMap;

use crate::walker::{format_size, FileEntry};

/// A node in the file tree (either a file or directory).
#[derive(Debug)]
enum TreeNode {
    File {
        size: u64,
    },
    Directory {
        children: BTreeMap<String, TreeNode>,
    },
}

impl TreeNode {
    fn new_directory() -> Self {
        TreeNode::Directory {
            children: BTreeMap::new(),
        }
    }

    fn new_file(size: u64) -> Self {
        TreeNode::File { size }
    }

    fn get_or_create_dir(&mut self, name: &str) -> &mut TreeNode {
        if let TreeNode::Directory { children } = self {
            children
                .entry(name.to_string())
                .or_insert_with(TreeNode::new_directory)
        } else {
            panic!("Expected directory node");
        }
    }

    fn insert_file(&mut self, name: &str, size: u64) {
        if let TreeNode::Directory { children } = self {
            children.insert(name.to_string(), TreeNode::new_file(size));
        }
    }
}

/// Build a tree structure from file entries.
fn build_tree(entries: &[FileEntry]) -> TreeNode {
    let mut root = TreeNode::new_directory();

    for entry in entries {
        let components: Vec<_> = entry.relative_path.components().collect();
        let mut current = &mut root;

        for (i, component) in components.iter().enumerate() {
            let name = component.as_os_str().to_string_lossy().to_string();
            let is_last = i == components.len() - 1;

            if is_last {
                current.insert_file(&name, entry.size);
            } else {
                current = current.get_or_create_dir(&name);
            }
        }
    }

    root
}

/// Render the tree to an ASCII string.
fn render_tree(node: &TreeNode, prefix: &str, is_root: bool, show_sizes: bool) -> String {
    let mut output = String::new();

    if let TreeNode::Directory { children } = node {
        let entries: Vec<_> = children.iter().collect();
        let total = entries.len();

        for (i, (name, child)) in entries.iter().enumerate() {
            let is_last_entry = i == total - 1;

            // Determine the connector
            let connector = if is_root {
                ""
            } else if is_last_entry {
                "└── "
            } else {
                "├── "
            };

            // Build the line
            match child {
                TreeNode::File { size } => {
                    if show_sizes {
                        output.push_str(&format!(
                            "{}{}{} ({})\n",
                            prefix,
                            connector,
                            name,
                            format_size(*size)
                        ));
                    } else {
                        output.push_str(&format!("{}{}{}\n", prefix, connector, name));
                    }
                }
                TreeNode::Directory { .. } => {
                    output.push_str(&format!("{}{}{}/\n", prefix, connector, name));

                    // Recurse into directory
                    let new_prefix = if is_root {
                        prefix.to_string()
                    } else if is_last_entry {
                        format!("{}    ", prefix)
                    } else {
                        format!("{}│   ", prefix)
                    };

                    output.push_str(&render_tree(child, &new_prefix, false, show_sizes));
                }
            }
        }
    }

    output
}

/// Generate an ASCII tree representation of the file entries.
pub fn generate_tree(root_name: &str, entries: &[FileEntry], show_sizes: bool) -> String {
    if entries.is_empty() {
        return format!("{}/\n(empty)\n", root_name);
    }

    let tree = build_tree(entries);
    let mut output = format!("{}/\n", root_name);
    output.push_str(&render_tree(&tree, "", true, show_sizes));
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_simple_tree() {
        let entries = vec![
            FileEntry {
                absolute_path: PathBuf::from("/project/src/main.rs"),
                relative_path: PathBuf::from("src/main.rs"),
                size: 100,
            },
            FileEntry {
                absolute_path: PathBuf::from("/project/Cargo.toml"),
                relative_path: PathBuf::from("Cargo.toml"),
                size: 200,
            },
        ];

        let tree = generate_tree("project", &entries, false);
        assert!(tree.contains("project/"));
        assert!(tree.contains("src/"));
        assert!(tree.contains("main.rs"));
        assert!(tree.contains("Cargo.toml"));
    }

    #[test]
    fn test_tree_with_sizes() {
        let entries = vec![FileEntry {
            absolute_path: PathBuf::from("/project/file.rs"),
            relative_path: PathBuf::from("file.rs"),
            size: 1024,
        }];

        let tree = generate_tree("project", &entries, true);
        assert!(tree.contains("1.0 KB"));
    }
}
