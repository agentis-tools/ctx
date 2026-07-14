#!/usr/bin/env bash
# Verify that the crate version, generated plugin scaffolds, committed canonical
# plugin trees, and release ZIP contents stay in lockstep.
#
# Usage: scripts/release-plugin-check.sh [--check] [EXPECTED_VERSION]
#   --check validates only; without it, matching canonical trees are packaged.
#   EXPECTED_VERSION defaults to the version in Cargo.toml.
#
# Requires: cargo, diff, jq and (when packaging) zip.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

check_only=0
if [ "${1:-}" = "--check" ]; then
  check_only=1
  shift
fi
if [ "$#" -gt 1 ]; then
  echo "usage: $0 [--check] [EXPECTED_VERSION]" >&2
  exit 2
fi

expected="${1:-$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')}"
target_dir="$(cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"

if ! diff -u src/harness/templates/SKILL.md skills/ctx/SKILL.md; then
  echo "ERROR: standalone skills/ctx/SKILL.md has drifted from the harness template" >&2
  exit 1
fi

echo "Building ctx plugin generator..."
# Plugin templates do not depend on DuckDB; avoiding default features keeps
# this packaging check fast and portable while preserving the default no-MCP
# release plugin shape.
cargo build --quiet --no-default-features
ctx_bin="$target_dir/debug/ctx"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

for target in claude codex; do
  generated="$scratch/$target"
  committed="$repo_root/plugins/$target/ctx"
  mkdir -p "$generated"
  (cd "$generated" && "$ctx_bin" harness init --target "$target" --mode plugin >/dev/null)

  plugin_json="$generated/.$target-plugin/plugin.json"
  if [ ! -f "$plugin_json" ]; then
    echo "ERROR: harness init did not generate .$target-plugin/plugin.json" >&2
    exit 1
  fi
  actual="$(jq -r '.version' "$plugin_json")"
  if [ "$actual" != "$expected" ]; then
    echo "ERROR: $target plugin version '$actual' does not match '$expected'" >&2
    exit 1
  fi
  if [ ! -d "$committed" ]; then
    echo "ERROR: canonical $target plugin is missing: plugins/$target/ctx" >&2
    exit 1
  fi
  if ! diff -ru "$committed" "$generated"; then
    echo "ERROR: plugins/$target/ctx has drifted from ctx harness output" >&2
    echo "Regenerate it with ctx harness init --target $target --mode plugin." >&2
    exit 1
  fi
  for hook in session-start post-tool-use stop; do
    if [ ! -x "$committed/hooks/$hook.sh" ]; then
      echo "ERROR: plugins/$target/ctx/hooks/$hook.sh is not executable" >&2
      exit 1
    fi
  done

  if [ "$check_only" -eq 0 ]; then
    zip_name="ctx-$target-plugin-$expected.zip"
    rm -f "$repo_root/$zip_name"
    (cd "$committed" && zip -qr "$repo_root/$zip_name" .)
    echo "Packaged $zip_name from plugins/$target/ctx"
  fi
done

echo "OK: templates, canonical plugins, and crate version match $expected"
