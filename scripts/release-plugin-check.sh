#!/usr/bin/env bash
# Plugin/binary version lockstep check + plugin packaging for releases.
#
# The Claude Code plugin manifest is *generated* from the crate version by
# `ctx harness init --mode plugin`, so plugin and binary versions agree by
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

(cd "$scratch" && "$ctx_bin" harness init --mode plugin >/dev/null)

plugin_json="$scratch/.claude-plugin/plugin.json"
if [ ! -f "$plugin_json" ]; then
  echo "ERROR: harness init did not generate .claude-plugin/plugin.json" >&2
  exit 1
fi

actual="$(jq -r '.version' "$plugin_json")"
if [ "$actual" != "$expected" ]; then
  echo "ERROR: plugin.json version '$actual' does not match release version '$expected'" >&2
  echo "       (plugin.json is generated from the crate version; tag, Cargo.toml," >&2
  echo "       and the generated manifest must agree)" >&2
  exit 1
fi

zip_name="ctx-claude-plugin-$expected.zip"
rm -f "$repo_root/$zip_name"
(cd "$scratch" && zip -qr "$repo_root/$zip_name" .)

echo "OK: plugin.json version $actual matches release version $expected"
echo "Packaged $zip_name"
