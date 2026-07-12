#!/usr/bin/env bash
# Plugin/binary version lockstep check + plugin packaging for releases.
#
# The Claude Code and Codex plugin manifests are generated from the crate version by
# `ctx harness init --mode plugin`, so plugins and binary versions agree by
# construction. This script asserts that invariant for a release: it builds
# the binary, scaffolds the plugin into a scratch directory, and fails when
# `plugin.json`'s version differs from the expected release version. On
# success it zips the scaffold as `ctx-claude-plugin-<version>.zip` in the
# repository root.
#
# Usage: scripts/release-plugin-check.sh [EXPECTED_VERSION]
#   EXPECTED_VERSION defaults to the version in Cargo.toml. The release
#   workflow passes the tag version (${GITHUB_REF_NAME#v}).
#
# Requires: cargo, jq, zip.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

expected="${1:-$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')}"

echo "Building ctx..."
cargo build --quiet
ctx_bin="$repo_root/target/debug/ctx"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

for target in claude codex; do
  target_scratch="$scratch/$target"
  mkdir -p "$target_scratch"
  (cd "$target_scratch" && "$ctx_bin" harness init --target "$target" --mode plugin >/dev/null)
  plugin_json="$target_scratch/.$target-plugin/plugin.json"
  if [ ! -f "$plugin_json" ]; then
    echo "ERROR: harness init did not generate .$target-plugin/plugin.json" >&2
    exit 1
  fi
  actual="$(jq -r '.version' "$plugin_json")"
  if [ "$actual" != "$expected" ]; then
    echo "ERROR: $target plugin version '$actual' does not match '$expected'" >&2
    exit 1
  fi
  zip_name="ctx-$target-plugin-$expected.zip"
  rm -f "$repo_root/$zip_name"
  (cd "$target_scratch" && zip -qr "$repo_root/$zip_name" .)
  echo "Packaged $zip_name"
done

echo "OK: Claude and Codex plugin versions match $expected"
