use clap::{Parser, Subcommand, ValueEnum};

#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
pub enum OutputFormat {
    #[default]
    Xml,
    Markdown,
    #[value(alias = "md")]
    Md,
    Plain,
    Json,
}

#[derive(Parser, Debug)]
#[command(name = "ctx")]
#[command(about = "Generate formatted context for AI assistants")]
#[command(version)]
#[command(after_help = r#"EXAMPLES:
    ctx                           # All files in current directory
    ctx "src/**/*.rs"             # Rust files matching glob
    ctx src/ Cargo.toml           # Specific paths
    ctx --format md               # Markdown output
    ctx -i "tests/" src/          # Ignore tests directory
    ctx --no-gitignore            # Include gitignored files
    
    ctx index                     # Build code intelligence index
    ctx query find main           # Find symbols named 'main'
    ctx query stats               # Show codebase statistics
"#)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// File patterns or paths to include (glob syntax supported)
    /// Examples: "src/**/*.rs", "*.ts", "src/"
    #[arg(default_value = ".", global = true)]
    pub patterns: Vec<String>,

    /// Output format
    #[arg(short = 'f', long, default_value = "xml", value_enum, global = true)]
    pub format: OutputFormat,

    /// Disable .gitignore pattern matching
    #[arg(long, global = true)]
    pub no_gitignore: bool,

    /// Additional ignore patterns (can be repeated)
    #[arg(short = 'i', long = "ignore", global = true)]
    pub ignore_patterns: Vec<String>,

    /// Disable built-in ignore patterns
    #[arg(long, global = true)]
    pub no_default_ignores: bool,

    /// Show file sizes in project tree
    #[arg(long, global = true)]
    pub show_sizes: bool,

    /// Disable project tree in output
    #[arg(long, global = true)]
    pub no_tree: bool,

    /// Buffer all output before printing (instead of streaming)
    #[arg(long, global = true)]
    pub no_stream: bool,

    /// Print stats (file count, total size, time taken)
    #[arg(long, global = true)]
    pub stats: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Build or update the code intelligence index
    Index {
        /// Watch for changes and reindex automatically
        #[arg(long, short)]
        watch: bool,

        /// Show verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Query the code intelligence database
    Query {
        #[command(subcommand)]
        query: QueryCommand,
    },

    /// Search for symbols using semantic or text search
    Search {
        /// Search query (symbol name or natural language)
        query: String,

        /// Maximum number of results
        #[arg(long, short, default_value = "20")]
        limit: i32,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        output: String,
    },

    /// Get the source code for a symbol
    Source {
        /// Symbol ID or name
        symbol: String,
    },

    /// Explain a symbol with its relationships
    Explain {
        /// Symbol name to explain
        symbol: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum QueryCommand {
    /// Find symbols by name pattern
    Find {
        /// Name pattern to search for
        pattern: String,

        /// Maximum number of results
        #[arg(long, short, default_value = "20")]
        limit: i32,

        /// Filter by symbol kind (function, struct, etc.)
        #[arg(long, short)]
        kind: Option<String>,
    },

    /// Show functions that call a given function
    Callers {
        /// Function name
        function: String,

        /// Maximum depth to traverse
        #[arg(long, short, default_value = "3")]
        depth: i32,
    },

    /// Show what a function depends on
    Deps {
        /// Symbol name
        symbol: String,

        /// Maximum depth to traverse
        #[arg(long, short, default_value = "3")]
        depth: i32,
    },

    /// Show the call graph from a starting point
    Graph {
        /// Starting symbol name
        start: String,

        /// Maximum depth
        #[arg(long, short, default_value = "5")]
        depth: i32,

        /// Output format (text, json, dot)
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Analyze impact of changing a symbol
    Impact {
        /// Symbol to analyze
        symbol: String,

        /// Maximum depth
        #[arg(long, short, default_value = "5")]
        depth: i32,
    },

    /// Show codebase statistics
    Stats,

    /// List all indexed files
    Files,
}
