# Contributing to ctx

Thank you for your interest in contributing to ctx! This document outlines how to get started and what we expect from contributions.

## Getting Started

1. **Clone the repository**
   ```bash
   git clone https://github.com/agentis-tools/ctx.git
   cd ctx
   ```

2. **Build the project**
   ```bash
   cargo build
   ```

3. **Run the CLI**
   ```bash
   cargo run -- --help
   cargo run -- src/
   ```

4. **Build with optional MCP support**
   ```bash
   cargo build --features mcp
   ```

## Making Changes

1. Create a feature branch (`git checkout -b feature/my-feature`)
2. Make your changes
3. Run the checks below
4. Push to your fork and open a Pull Request

## Before Submitting

Please ensure all of the following pass:

```bash
# Format code
cargo fmt

# Lint (zero warnings allowed)
cargo clippy -- -D warnings

# Run tests
cargo test

# Verify it still publishes cleanly
cargo publish --dry-run
```

### Performance gates

CI runs an **advisory** `perf` job on every PR: it spawns a prebuilt `ctx` binary against deterministic synthetic fixtures and checks the hook-path commands (incremental index, score, check, map, sql) against latency budgets, an RSS ceiling, and a committed baseline. Advisory means it does not block merges — but treat a red `perf` job as a real finding, not noise.

When it fails:

1. **Reproduce locally** following [`perf/README.md`](perf/README.md) (`cargo build --profile perf --all-features`, then run the harness with `CTX_PERF_BIN` pointing at the binary).
2. **Mind the budget scale.** Budgets are calibrated for local bare-metal runs (`CTX_PERF_BUDGET_SCALE=1.0`); CI uses `1.5` because hosted runners are slower and noisier. A scenario that passes locally but fails in CI by a hair is usually runner noise — re-run before digging. The 1.20x baseline-regression factor is deliberately *not* scaled.
3. **Baseline updates are deliberate, never automatic.** CI never writes baselines. If a slowdown is intentional (e.g. a feature genuinely does more work), capture a new baseline on the CI runner class per `perf/baselines/README.md` and commit it in its own commit with a justification.

## IDE Configuration

The repository includes `.vscode` and `.idea` in `.gitignore`. Feel free to add local IDE configs but do not commit them.

## Reporting Issues

When reporting issues, please include:
- Your OS and Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs actual behavior
- Relevant error messages or stack traces

## Security Issues

Please see [SECURITY.md](SECURITY.md) for how to report vulnerabilities.

## Questions?

Open an issue with the `question` label or start a discussion in the repository.
