use clap::{Parser, Subcommand, ValueEnum};

/// CLI output format (with clap integration).
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq)]
pub enum OutputFormat {
    #[default]
    Xml,
    Markdown,
    #[value(alias = "md")]
    Md,
    Plain,
    Json,
}

impl OutputFormat {
    /// Convert to library OutputFormat.
    pub fn to_lib(self) -> ctx::formatter::OutputFormat {
        match self {
            OutputFormat::Xml => ctx::formatter::OutputFormat::Xml,
            OutputFormat::Markdown | OutputFormat::Md => ctx::formatter::OutputFormat::Markdown,
            OutputFormat::Plain => ctx::formatter::OutputFormat::Plain,
            OutputFormat::Json => ctx::formatter::OutputFormat::Json,
        }
    }
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

    /// Emit machine-readable JSON to stdout (see docs/json-output.md)
    #[arg(long, global = true)]
    pub json: bool,

    /// Print stats (file count, total size, time taken)
    #[arg(long, global = true)]
    pub stats: bool,

    // Token counting options (LLM context management)
    /// Only count tokens, don't output file contents
    #[arg(long, global = true)]
    pub count_only: bool,

    /// Maximum tokens to include in output (omits files to fit budget, does not truncate file contents)
    #[arg(long, global = true)]
    pub max_tokens: Option<usize>,

    /// Tokenizer encoding to use (cl100k_base, o200k_base, p50k_base)
    #[arg(long, default_value = "cl100k_base", global = true)]
    pub encoding: String,
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

        /// Force full reindex (clears existing database)
        #[arg(long)]
        force: bool,

        /// Use parallel parsing for faster indexing on multi-core systems
        #[arg(long, short = 'j')]
        parallel: bool,

        /// Disable .gitignore pattern matching
        #[arg(long)]
        no_gitignore: bool,

        /// Disable built-in ignore patterns
        #[arg(long)]
        no_default_ignores: bool,

        /// Additional ignore patterns (can be repeated)
        #[arg(short = 'i', long = "ignore")]
        ignore_patterns: Vec<String>,

        /// File patterns or paths to include (glob syntax supported)
        #[arg(short = 'p', long = "pattern")]
        include_patterns: Vec<String>,
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

        /// Filter by file path pattern (glob syntax: "src/parser/*.rs")
        #[arg(long, short = 'f')]
        file: Option<String>,

        /// Filter by symbol kind (function, method, struct, etc.)
        #[arg(long, short)]
        kind: Option<String>,
    },

    /// Explain a symbol with its relationships
    Explain {
        /// Symbol name to explain
        symbol: String,

        /// Filter by file path pattern (glob syntax: "src/parser/*.rs")
        #[arg(long, short = 'f')]
        file: Option<String>,

        /// Filter by symbol kind (function, method, struct, etc.)
        #[arg(long, short)]
        kind: Option<String>,
    },

    /// Generate embeddings for semantic search
    Embed {
        /// Force re-embedding of all symbols
        #[arg(long, short)]
        force: bool,

        /// Show verbose output
        #[arg(long, short)]
        verbose: bool,

        /// Batch size for embedding generation
        #[arg(long, default_value = "50")]
        batch_size: usize,

        /// Use OpenAI API instead of local model (requires OPENAI_API_KEY)
        #[arg(long)]
        openai: bool,

        /// Watch for index changes and auto-embed new symbols
        #[arg(long, short)]
        watch: bool,
    },

    /// Semantic search using embeddings (requires embeddings to be generated)
    Semantic {
        /// Natural language search query
        query: String,

        /// Maximum number of results
        #[arg(long, short, default_value = "10")]
        limit: usize,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        output: String,

        /// Use OpenAI API instead of local model (requires OPENAI_API_KEY)
        #[arg(long)]
        openai: bool,
    },

    /// Analyze code complexity and flag high fan-out functions
    Complexity {
        /// Fan-out threshold (default: 10, flag > 50 as critical)
        #[arg(long, default_value = "10")]
        threshold: i64,

        /// Only show functions exceeding threshold
        #[arg(long, short)]
        warnings_only: bool,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        output: String,
    },

    /// Detect duplicate or similar code blocks
    Duplicates {
        /// Minimum similarity percentage (0-100)
        #[arg(long, default_value = "80")]
        similarity: u32,

        /// Minimum lines for a code block to be considered
        #[arg(long, default_value = "5")]
        min_lines: u32,

        /// Output format (table, json)
        #[arg(long, default_value = "table")]
        output: String,
    },

    /// Generate a dependency graph visualization
    Graph {
        /// Output format (dot, mermaid, json)
        #[arg(long, default_value = "dot")]
        output: String,

        /// Group by file/module instead of individual symbols
        #[arg(long)]
        by_file: bool,

        /// Only show dependencies involving these files (comma-separated)
        #[arg(long)]
        filter: Option<String>,

        /// Maximum depth for symbol-level graphs
        #[arg(long, default_value = "3")]
        depth: i32,
    },

    /// Intelligently select files relevant to a task using semantic search and call graph analysis
    Smart {
        /// Natural language description of the task (e.g., "add caching to the parser")
        task: String,

        /// Maximum tokens in output
        #[arg(long, default_value = "8000")]
        max_tokens: usize,

        /// Call graph expansion depth
        #[arg(long, default_value = "2")]
        depth: i32,

        /// Number of initial semantic matches to find
        #[arg(long, default_value = "10")]
        top: usize,

        /// Show selection reasoning for each file
        #[arg(long)]
        explain: bool,

        /// Preview selection without generating context
        #[arg(long)]
        dry_run: bool,

        /// Use OpenAI API instead of local model (requires OPENAI_API_KEY)
        #[arg(long)]
        openai: bool,

        /// Output format
        #[arg(short = 'f', long, default_value = "xml", value_enum)]
        format: OutputFormat,

        /// Show file sizes in project tree
        #[arg(long)]
        show_sizes: bool,

        /// Disable project tree in output
        #[arg(long)]
        no_tree: bool,
    },

    /// Generate context for git changes (diff-aware)
    Diff {
        /// Git revision or range (default: HEAD~1)
        #[arg(default_value = "HEAD~1")]
        revision: String,

        /// Maximum tokens in output
        #[arg(long, default_value = "8000")]
        max_tokens: usize,

        /// Call graph context depth
        #[arg(long, default_value = "1")]
        depth: i32,

        /// Only include changed files (no context expansion)
        #[arg(long)]
        changes_only: bool,

        /// Include staged changes only
        #[arg(long)]
        staged: bool,

        /// Include change summary
        #[arg(long)]
        summary: bool,

        /// Output format
        #[arg(short = 'f', long, default_value = "xml", value_enum)]
        format: OutputFormat,

        /// Show file sizes in project tree
        #[arg(long)]
        show_sizes: bool,

        /// Disable project tree in output
        #[arg(long)]
        no_tree: bool,
    },

    /// Generate context for PR review (GitHub integration)
    Review {
        /// PR number or URL
        pr: String,

        /// Repository (owner/name, auto-detected if not specified)
        #[arg(long)]
        repo: Option<String>,

        /// Include PR comments in output
        #[arg(long)]
        include_comments: bool,

        /// Maximum tokens in output
        #[arg(long, default_value = "8000")]
        max_tokens: usize,

        /// Call graph context depth
        #[arg(long, default_value = "1")]
        depth: i32,

        /// Only include changed files (no context expansion)
        #[arg(long)]
        changes_only: bool,

        /// Include change summary
        #[arg(long)]
        summary: bool,

        /// Output format
        #[arg(short = 'f', long, default_value = "xml", value_enum)]
        format: OutputFormat,

        /// Show file sizes in project tree
        #[arg(long)]
        show_sizes: bool,

        /// Disable project tree in output
        #[arg(long)]
        no_tree: bool,
    },

    /// Generate code quality audit report
    Audit {
        /// Output format (text, json, markdown)
        #[arg(long = "output", short = 'o', default_value = "text")]
        output_format: String,

        /// Minimum score threshold (fails if below, 0.0-10.0)
        #[arg(long)]
        min_score: Option<f32>,

        /// Categories to check (comma-separated: complexity,duplication,coverage,modularity,naming)
        #[arg(long)]
        categories: Option<String>,

        /// Only audit changed files (not yet implemented)
        #[arg(long)]
        incremental: bool,
    },

    /// Check architecture rules from .ctx/rules.toml against the index
    ///
    /// Exit codes: 0 = no violations, 1 = at least one violation,
    /// 2 = operational error (missing/invalid rules file, unknown or
    /// overlapping layers, missing index, bad git ref).
    #[command(after_help = r#"RULES FILE (.ctx/rules.toml):
    version = 1

    [layers]                                   # layer name -> globs over indexed files
    domain         = ["src/domain/**"]
    application    = ["src/app/**"]
    infrastructure = ["src/infra/**", "src/db/**"]

    [[rules.forbidden]]                        # `from` must not depend on `to`
    from   = "domain"
    to     = "infrastructure"
    reason = "Domain layer must stay persistence-agnostic"

    [[rules.allowed_dependents]]               # only `only` may depend on `layer`
    layer = "infrastructure"                   # (files in no layer are exempt)
    only  = ["application"]

    [[rules.limit]]                            # metric thresholds
    metric  = "fan_in"                         # fan_in | fan_out | complexity | file_symbols
    scope   = "symbol"                         # symbol | file
    max     = 25
    exclude = ["src/core/**"]

    [[rules.no_new_dependents]]                # frozen paths
    paths  = ["src/legacy/**"]
    reason = "Legacy module is frozen; do not add new callers"

EXAMPLES:
    ctx check                        # check all rules
    ctx check --against main         # only violations touching files changed since main
    ctx check --list                 # show parsed rules and layer sizes
    ctx check --json                 # machine-readable output (see docs/json-output.md)
"#)]
    Check {
        /// Path to the rules file (default: .ctx/rules.toml)
        #[arg(long)]
        rules: Option<std::path::PathBuf>,

        /// Only report violations where at least one endpoint's file changed
        /// since REF (for no_new_dependents: where the new dependent changed)
        #[arg(long, value_name = "REF")]
        against: Option<String>,

        /// Print the parsed rules and layer membership counts, then exit 0
        #[arg(long)]
        list: bool,
    },

    /// Interactive shell for exploring codebase
    Shell {
        /// History file location
        #[arg(long)]
        history: Option<std::path::PathBuf>,

        /// Disable command history
        #[arg(long)]
        no_history: bool,

        /// Use vi editing mode (default: emacs)
        #[arg(long)]
        vi: bool,
    },

    /// Start MCP (Model Context Protocol) server for AI assistant integration
    #[cfg(feature = "mcp")]
    Serve {
        /// Run as MCP server over stdio (for Claude Desktop integration)
        #[arg(long)]
        mcp: bool,
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

        /// Filter by file path pattern (glob syntax: "src/parser/*.rs")
        #[arg(long, short = 'f')]
        file: Option<String>,
    },

    /// Show functions that call a given function
    Callers {
        /// Function name
        function: String,

        /// Maximum depth to traverse
        #[arg(long, short, default_value = "3")]
        depth: i32,

        /// Filter by file path pattern (glob syntax: "src/parser/*.rs")
        #[arg(long, short = 'f')]
        file: Option<String>,
    },

    /// Show what a function depends on
    Deps {
        /// Symbol name
        symbol: String,

        /// Maximum depth to traverse
        #[arg(long, short, default_value = "3")]
        depth: i32,

        /// Filter by file path pattern (glob syntax: "src/parser/*.rs")
        #[arg(long, short = 'f')]
        file: Option<String>,

        /// Filter by symbol kind (function, method, struct, etc.)
        #[arg(long, short)]
        kind: Option<String>,
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
