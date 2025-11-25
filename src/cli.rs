use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
pub enum OutputFormat {
    #[default]
    Xml,
    Markdown,
    #[value(alias = "md")]
    Md,
    Plain,
}

#[derive(Parser, Debug)]
#[command(name = "context")]
#[command(about = "Generate formatted context for AI assistants")]
#[command(version)]
#[command(after_help = r#"EXAMPLES:
    context                           # All files in current directory
    context "src/**/*.rs"             # Rust files matching glob
    context src/ Cargo.toml           # Specific paths
    context --format md               # Markdown output
    context -i "tests/" src/          # Ignore tests directory
    context --no-gitignore            # Include gitignored files
"#)]
pub struct Args {
    /// File patterns or paths to include (glob syntax supported)
    /// Examples: "src/**/*.rs", "*.ts", "src/"
    #[arg(default_value = ".")]
    pub patterns: Vec<String>,

    /// Output format
    #[arg(short = 'f', long, default_value = "xml", value_enum)]
    pub format: OutputFormat,

    /// Disable .gitignore pattern matching
    #[arg(long)]
    pub no_gitignore: bool,

    /// Additional ignore patterns (can be repeated)
    #[arg(short = 'i', long = "ignore")]
    pub ignore_patterns: Vec<String>,

    /// Disable built-in ignore patterns
    #[arg(long)]
    pub no_default_ignores: bool,

    /// Show file sizes in project tree
    #[arg(long)]
    pub show_sizes: bool,

    /// Disable project tree in output
    #[arg(long)]
    pub no_tree: bool,

    /// Buffer all output before printing (instead of streaming)
    #[arg(long)]
    pub no_stream: bool,

    /// Print stats (file count, total size, time taken)
    #[arg(long)]
    pub stats: bool,
}
