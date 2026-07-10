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
