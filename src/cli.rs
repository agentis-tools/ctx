use clap::{Parser, Subcommand, ValueEnum};

use crate::commands::hotspots::HotspotBy;
use ctx::embeddings::Provider;

/// CLI output format (with clap integration).
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq)]
pub enum OutputFormat {
    #[default]
    Xml,
    Markdown,
    #[value(alias = "md")]
    Md,
    Plain,
    /// Plain text (used by `ctx map`; alias for plain elsewhere)
    Text,
    Json,
}

impl OutputFormat {
    /// Convert to library OutputFormat.
    pub fn to_lib(self) -> ctx::formatter::OutputFormat {
        match self {
            OutputFormat::Xml => ctx::formatter::OutputFormat::Xml,
            OutputFormat::Markdown | OutputFormat::Md => ctx::formatter::OutputFormat::Markdown,
            OutputFormat::Plain | OutputFormat::Text => ctx::formatter::OutputFormat::Plain,
            OutputFormat::Json => ctx::formatter::OutputFormat::Json,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "ctx")]
#[command(about = "Generate formatted context for AI assistants")]
// clap's auto --version exits during parsing, before our code runs; we
// define our own flag below so `--version --check` can compare against the
// latest GitHub release (`ctx --version` alone prints the same "ctx <ver>"
// line clap used to).
#[command(disable_version_flag = true)]
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

    /// Print version
    #[arg(short = 'V', long = "version")]
    pub version: bool,

    /// With --version: also query GitHub for a newer release
    #[arg(long, requires = "version")]
    pub check: bool,

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

    /// Run read-only SQL against the code index (via the stable `v1.*` views)
    #[command(after_help = r#"EXAMPLES:
    ctx sql "SELECT name, file, complexity FROM v1.symbols ORDER BY complexity DESC LIMIT 10"
    ctx sql --json "SELECT kind, COUNT(*) n FROM v1.symbols GROUP BY kind"
    ctx sql --fail-on-rows --file .ctx/gates/no-utils-imports.sql
    ctx sql --schema        # print the v1 schema reference and exit
    ctx sql --snapshots "SELECT commit_sha, count(*) FROM snap.dup_pairs GROUP BY commit_sha"

The query surface is the versioned `v1` schema; anything outside `v1.*` is
internal and unstable. Access is read-only and engine-hardened.
"#)]
    Sql {
        /// SQL text. If "-" or omitted while stdin is piped, read from stdin.
        query: Option<String>,

        /// Read the query from a file (mutually exclusive with the QUERY arg)
        #[arg(long)]
        file: Option<std::path::PathBuf>,

        /// Output format: table, csv, or json (the global -f/--format does not
        /// apply here; use this or --json)
        #[arg(long = "output", default_value = "table")]
        output: String,

        /// Alias for --output json
        #[arg(long)]
        json: bool,

        /// Cap returned rows (0 = unlimited)
        #[arg(long, default_value = "1000")]
        max_rows: usize,

        /// Abort the query after N seconds
        #[arg(long, default_value = "10")]
        timeout: u64,

        /// Exit 1 if the query returns >= 1 row (for gate usage)
        #[arg(long)]
        fail_on_rows: bool,

        /// Print the public schema reference and exit
        #[arg(long)]
        schema: bool,

        /// Load snapshot partitions from DIR (default .ctx/snapshots) as
        /// in-memory tables snap.files / snap.symbols / snap.dup_pairs /
        /// snap.meta for trend queries (see `ctx snapshot`). Pass a custom
        /// dir with `--snapshots=DIR`.
        #[arg(
            long,
            value_name = "DIR",
            num_args = 0..=1,
            require_equals = true,
            default_missing_value = ".ctx/snapshots"
        )]
        snapshots: Option<std::path::PathBuf>,
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
        // No `short`: `-f` is the global `--format` flag (other subcommands'
        // `--force` are also long-only). Use `--force`.
        #[arg(long)]
        force: bool,

        /// Show verbose output
        #[arg(long, short)]
        verbose: bool,

        /// Batch size for embedding generation
        #[arg(long, default_value = "50")]
        batch_size: usize,

        /// Embedding backend: local (default), openai, or ollama
        #[arg(long, value_enum)]
        provider: Option<Provider>,

        /// Deprecated alias for `--provider openai` (requires OPENAI_API_KEY)
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

        /// Embedding backend: local (default), openai, or ollama
        #[arg(long, value_enum)]
        provider: Option<Provider>,

        /// Deprecated alias for `--provider openai` (requires OPENAI_API_KEY)
        #[arg(long)]
        openai: bool,
    },

    /// Find existing functions similar to a description (reuse before you write)
    ///
    /// Searches function and method symbols by embedding similarity and
    /// reports a one-line doc, similarity score, and fan-in for each hit so
    /// you can judge whether an established utility already covers the need.
    ///
    /// Exit codes: 0 = success (even with no matches); 2 = no embeddings
    /// generated yet (run `ctx embed`, or use --keyword for FTS-based search
    /// that needs no embeddings) or any other operational error.
    Similar {
        /// Natural language or signature-like description of the intended function
        query: String,

        /// Maximum number of results
        #[arg(long, short, default_value = "10")]
        limit: usize,

        /// Use FTS5 keyword search instead of embeddings (works with zero
        /// embeddings and no API key)
        #[arg(long)]
        keyword: bool,

        /// Embedding backend: local (default), openai, or ollama
        #[arg(long, value_enum)]
        provider: Option<Provider>,

        /// Deprecated alias for `--provider openai` (requires OPENAI_API_KEY)
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

    /// Detect structurally similar functions (MinHash near-duplicate search)
    ///
    /// Functions are compared by the Jaccard similarity of their normalized
    /// token shingles (identifiers -> ID, literals -> LIT, comments dropped),
    /// so renamed variables and changed literals still match. Fingerprints
    /// are built during `ctx index`; reindex before running this command.
    /// Solidity functions are fingerprinted too (tokenized with the
    /// solang-parser lexer).
    ///
    /// Breaking change: this replaces the old line-based detector and its
    /// percent/line-count flags.
    Duplicates {
        /// Jaccard similarity threshold (0.0-1.0) over normalized token
        /// shingles. Breaking change from the old percent-based, line-oriented
        /// threshold: 0.85 means 85% of 5-token shingles are shared, not that
        /// 85% of lines match. Values below 0.5 are clamped to 0.5.
        #[arg(long, default_value_t = 0.85)]
        threshold: f64,

        /// Ignore functions with fewer than N normalized tokens
        #[arg(long, default_value_t = 50)]
        min_tokens: i64,

        /// Only report pairs where at least one function is in a file
        /// changed relative to this git reference (e.g. `main`)
        #[arg(long, value_name = "REF")]
        against: Option<String>,

        /// Exit with code 1 when any near-duplicate pair is reported
        /// (default: informational, exit 0)
        #[arg(long)]
        fail_on_found: bool,
    },

    /// Print a token-budgeted map of the repository's most important symbols
    ///
    /// Ranks symbols with PageRank over the resolved symbol graph (calls,
    /// imports, extends, implements) and emits them (grouped by file,
    /// preceded by a compact project tree) until the token budget is
    /// exhausted. Tokens are estimated as ceil(chars / 4). Output is
    /// deterministic for identical index state, which makes it well suited
    /// for SessionStart hooks that prime an AI assistant with a stable
    /// overview of the codebase.
    Map {
        /// Token budget for the map (tokens are estimated as ceil(chars / 4))
        #[arg(long, default_value = "2000")]
        budget: usize,

        /// Focus on a file path/glob or symbol name: boosts the matching
        /// symbols and their direct neighbors in the ranking
        #[arg(long)]
        focus: Option<String>,

        /// Output format (text, markdown, json)
        #[arg(short = 'f', long, default_value = "text", value_enum)]
        format: OutputFormat,
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

        /// Embedding backend: local (default), openai, or ollama
        #[arg(long, value_enum)]
        provider: Option<Provider>,

        /// Deprecated alias for `--provider openai` (requires OPENAI_API_KEY)
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

    /// Rank files by combined git churn and code complexity (hotspots)
    ///
    /// A hotspot is code that is both structurally complex and frequently
    /// changed -- usually the highest-leverage refactoring target. Requires a
    /// git repository and a built index (run `ctx index` first).
    #[command(after_help = r#"SCORING:
    score = normalized_churn x normalized_complexity, where both factors are
    min-max normalized to [0, 1] over the analyzed set (indexed files with at
    least --min-churn commits since --since). If all values are equal, they
    all normalize to 1.0. Raw commit and complexity counts are reported
    alongside the score.

APPROXIMATIONS (v1):
    - With --by symbol, a symbol's churn is approximated by its FILE's commit
      count; per-symbol git history is not tracked yet.
    - Churn is collected with `git log --no-renames`, so renaming a file
      resets its commit count.

EXIT CODES:
    0    success (informational command; hotspots never affect the exit code)
    2    operational error (not a git repository, missing index, bad ref)
"#)]
    Hotspots {
        /// How far back to count commits (git --since date spec)
        #[arg(long, default_value = "6 months ago")]
        since: String,

        /// Maximum number of entries to show
        #[arg(long, default_value = "20")]
        limit: usize,

        /// Rank by file or by symbol (symbol churn is approximated by its file's churn)
        #[arg(long, value_enum, default_value_t = HotspotBy::File)]
        by: HotspotBy,

        /// Minimum number of commits for a file to be analyzed
        #[arg(long, default_value = "2")]
        min_churn: u32,

        /// Only analyze files changed relative to this git ref (e.g. main)
        #[arg(long, value_name = "REF")]
        against: Option<String>,
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

    /// Score the quality delta of your changes against a git reference
    ///
    /// Compares the working tree (plus commits since the merge base with
    /// REF) against REF and prints a scorecard: complexity and fan-out
    /// deltas, newly introduced near-duplicate functions, architecture-rule
    /// violations, and symbols added/removed. The index is refreshed
    /// incrementally before scoring.
    ///
    /// Exit codes: 0 = clean (or no --fail-on given), 1 = at least one
    /// --fail-on condition was met, 2 = operational error (not a git repo,
    /// bad reference, malformed --fail-on, invalid rules file).
    #[command(after_help = r#"METRICS (for --fail-on and JSON output):
    complexity_delta    sum over changed files of per-function
                        2*fan_out + same-file fan_in, current - baseline
    fan_out_delta       calls sourced in changed files, current - baseline
    new_duplication     near-duplicate pairs (Jaccard >= 0.85, >= 50 tokens,
                        >= 1 endpoint in a changed file) absent at REF
    check_violations    `ctx check --against REF` violations
                        (0 with a note when .ctx/rules.toml is missing)
    symbols_added       symbols present now but not at REF
    symbols_removed     symbols present at REF but not now
    files_changed       changed source files that were scored

NOTES:
    Baseline metrics come from parsing each changed file's content at REF in
    memory with the same parser; fan-in is approximated as same-file callers
    on both sides so the deltas compare like with like.

EXAMPLES:
    ctx score                        # score uncommitted changes (vs HEAD)
    ctx score --against main         # score the whole branch / PR vs main
    ctx score --against main --fail-on "check_violations>0,new_duplication>0"
    ctx score --fail-on "complexity_delta>=20" --json
"#)]
    Score {
        /// Git reference to compare against. The default (HEAD) scores
        /// uncommitted changes; use your default branch (main/master) to
        /// score a whole branch or PR
        #[arg(long, value_name = "REF", default_value = "HEAD")]
        against: String,

        /// Fail (exit 1) when any comma-separated condition `metric OP value`
        /// holds; OP is one of >=, <=, >, < (e.g. "new_duplication>0")
        #[arg(long, value_name = "EXPR")]
        fail_on: Option<String>,
    },

    /// Capture per-commit Parquet metric snapshots (.ctx/snapshots/sha=<sha>/)
    ///
    /// Refreshes the index incrementally, then exports per-symbol and
    /// per-file metrics, near-duplicate pairs, and metadata for HEAD as one
    /// Parquet partition, for longitudinal quality analysis. Requires a git
    /// repository; snapshot capture requires the duckdb feature.
    ///
    /// Exit codes: 0 = success (including "partition already exists"),
    /// 2 = operational error (not a git repo, missing duckdb feature, IO).
    #[command(after_help = r#"PARTITION LAYOUT:
    .ctx/snapshots/sha=<sha>/symbols.parquet     per-symbol metrics
    .ctx/snapshots/sha=<sha>/files.parquet       per-file metrics + churn + violations
    .ctx/snapshots/sha=<sha>/dup_pairs.parquet   near-duplicate function pairs
    .ctx/snapshots/sha=<sha>/meta.parquet        capture metadata (1 row)

    Every row carries commit_sha and committed_at, so tables union cleanly
    across partitions with a read_parquet glob.

EXAMPLES:
    ctx snapshot                                # snapshot HEAD (skip if it exists)
    ctx snapshot --force                        # rewrite the HEAD partition
    ctx snapshot --json                         # machine-readable report
    ctx snapshot backfill --since v0.1.0        # snapshot v0.1.0..HEAD (first-parent)
    ctx snapshot backfill --since main~20 --every 5
"#)]
    Snapshot {
        #[command(subcommand)]
        cmd: Option<SnapshotCommand>,

        /// Overwrite an existing partition for HEAD
        #[arg(long)]
        force: bool,

        /// How far back to count per-file churn (git --since date spec)
        #[arg(long, value_name = "SPEC", default_value = "90 days ago")]
        churn_window: String,
    },

    /// Package ctx as an AI coding harness integration (Claude Code)
    ///
    /// `init` scaffolds hook scripts, settings, and (in plugin mode) a full
    /// Claude Code plugin from templates embedded in this binary. `compat`
    /// is the version guard those generated hooks call before doing any
    /// work. `doctor` diagnoses the integration end to end.
    #[command(after_help = r#"EXIT CODES:
    0    success / healthy
    1    doctor found problems (errors or warnings)
    2    operational error (unknown --target, bad arguments, IO failure)
    3    version requirement not met (reserved exclusively for
         'ctx harness compat --require <VERSION>')

EXAMPLES:
    ctx harness init                          # wire hooks into .claude/ (local mode)
    ctx harness init --mode plugin            # scaffold a Claude Code plugin
    ctx harness init --force                  # regenerate even user-modified files
    ctx harness compat --require 0.2          # exit 0 if this binary is new enough
    ctx harness doctor --json                 # machine-readable health report
"#)]
    Harness {
        #[command(subcommand)]
        cmd: HarnessCommand,
    },

    /// Update ctx to the latest GitHub release (or a pinned version)
    ///
    /// Downloads the release artifact for this platform, verifies its
    /// sha256 against the release's published SHA256SUMS file, and
    /// atomically replaces the running executable. ctx never updates
    /// itself automatically: the passive update notice only points here.
    #[command(after_help = r#"EXIT CODES (ctx-wide convention):
    0    updated successfully, or already up to date
    2    operational error (network failure, checksum mismatch, install
         location not writable, unsupported platform, unknown version)

SECURITY:
    The downloaded archive is verified against the SHA256SUMS file published
    with the GitHub release before anything is replaced. On mismatch the
    update aborts with exit code 2 and the installed binary is untouched.
    Harness permissions generated by 'ctx harness init' deny
    'Bash(ctx self-update*)', so AI agents cannot swap the binary out from
    under a running session.

NOTES:
    Set CTX_NO_UPDATE_CHECK=1 to silence the passive update notice.
    If ctx was installed via cargo, prefer 'cargo install agentis-ctx'.
    On Windows the previous binary is left beside the new one as
    'ctx.exe.old' (removed on the next self-update).

EXAMPLES:
    ctx self-update                    # update to the latest release
    ctx self-update --version 0.3.0    # install an exact release
    ctx self-update --json             # machine-readable result
"#)]
    SelfUpdate {
        /// Install this exact release (allows downgrades) instead of the latest
        #[arg(long, value_name = "X.Y.Z")]
        version: Option<String>,
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

/// Harness targets supported by `ctx harness init`.
///
/// Unknown targets are rejected by clap with a usage error (exit code 2)
/// that lists the supported values.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq)]
pub enum HarnessTarget {
    /// Claude Code (hooks, settings, skills, plugin manifest)
    #[default]
    Claude,
}

/// Scaffolding mode for `ctx harness init`.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq)]
pub enum HarnessMode {
    /// Hook scripts under `.claude/hooks/ctx/` plus a settings snippet
    /// printed for manual inclusion
    #[default]
    Local,
    /// A distributable Claude Code plugin (`.claude-plugin/`, hooks, skill,
    /// marketplace manifest)
    Plugin,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommand {
    /// Backfill snapshots for historical commits (first-parent walk)
    ///
    /// Walks `--since <REF>`..HEAD along the first parent, oldest first
    /// (including REF itself when it names a commit). Each missing commit is
    /// checked out into a temporary git worktree, snapshotted into this
    /// repository's .ctx/snapshots/, and the worktree is removed again; the
    /// working tree is never touched. Existing partitions are skipped.
    /// Per-commit failures are logged to stderr and do not abort the run.
    Backfill {
        /// Starting commit/ref (inclusive when it resolves to a commit)
        #[arg(long, value_name = "REF")]
        since: String,

        /// Sample every Nth commit (the newest commit is always included)
        #[arg(long, value_name = "N", default_value = "1")]
        every: usize,

        /// How far back to count per-file churn (git --since date spec);
        /// the upper bound is anchored to each commit's date
        #[arg(long, value_name = "SPEC", default_value = "90 days ago")]
        churn_window: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum HarnessCommand {
    /// Generate integration files from templates embedded in this binary
    ///
    /// Every generated file carries a `generated by ctx` header and a
    /// checksum. Re-running `init` regenerates unmodified files in place
    /// but never overwrites files you have edited (warn + skip) unless
    /// `--force` is given. `.ctx/rules.toml` is never overwritten, even
    /// with `--force`.
    Init {
        /// Harness to target (unknown values exit 2 listing supported targets)
        #[arg(long, value_enum, default_value = "claude")]
        target: HarnessTarget,

        /// What to scaffold: local hooks or a distributable plugin
        #[arg(long, value_enum, default_value = "local")]
        mode: HarnessMode,

        /// Overwrite user-modified generated files (except .ctx/rules.toml)
        #[arg(long)]
        force: bool,
    },

    /// Check that this binary satisfies a version requirement
    ///
    /// Exits 0 when the running binary satisfies REQUIRE, otherwise prints
    /// one line to stderr and exits 3. Exit code 3 is reserved exclusively
    /// for this subcommand; generated hook scripts call it as a guard so
    /// they can fail open (loudly) when the binary is older than the
    /// templates that generated them.
    Compat {
        /// Required version: a bare version ("0.2" means "at least 0.2.0")
        /// or a semver requirement expression ("^0.2", ">=0.2, <0.4")
        #[arg(long, value_name = "SEMVER")]
        require: String,
    },

    /// Diagnose the harness integration (binary, templates, index, rules, hooks, MCP)
    ///
    /// Exit codes: 0 = healthy (info-level notes only), 1 = problems found
    /// (errors or warnings), 2 = operational error.
    Doctor,
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
