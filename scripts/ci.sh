#!/usr/bin/env bash
# Local equivalent of the deterministic pull-request quality gates.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
release_bin="$target_dir/release/ctx"

python3 scripts/check-governance.py check
python3 -m unittest discover -s tests/versioning -p 'test_*.py'
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo test --locked --no-default-features
cargo build --locked --release --no-default-features
python3 scripts/version.py check --binary "$release_bin"
python3 scripts/check-contracts.py check --binary "$release_bin"
cargo publish --locked --dry-run
cargo publish --locked --dry-run --no-default-features
