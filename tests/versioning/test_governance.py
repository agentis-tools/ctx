#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def load_script(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


governance = load_script("ctx_governance", "check-governance.py")
contracts = load_script("ctx_contracts", "check-contracts.py")


class GovernancePolicyTests(unittest.TestCase):
    def test_cookbook_recipes_require_fast_path_evidence_and_agent_handoff(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cookbook = root / "docs/website/docs/cookbook"
            cookbook.mkdir(parents=True)
            (cookbook / "index.md").write_text(
                "# Cookbook\n\n[Broken route](recipe.md)\n", encoding="utf-8"
            )
            (cookbook / "recipe.md").write_text(
                "## Quickest version\n\n## Give the workflow to an agent\n",
                encoding="utf-8",
            )

            errors = governance.cookbook_structure_errors(root)

        self.assertEqual(len(errors), 2)
        self.assertTrue(any("extensionless" in error for error in errors))
        self.assertTrue(
            any("What worked, and what did not" in error for error in errors)
        )

    def test_unreleased_entries_do_not_reuse_released_history(self):
        text = """## [Unreleased]

### Fixed
- New fix

## [0.3.4] - 2026-07-12
- Historical fix
"""
        self.assertEqual(governance.unreleased_entries(text), {"- New fix"})

    def test_manifest_version_reads_only_package_version(self):
        text = """[package]
name = "agentis-ctx"
version = "0.3.4"

[dependencies]
example = "9.9.9"
"""
        self.assertEqual(governance.manifest_version(text), "0.3.4")


class ContractPolicyTests(unittest.TestCase):
    def test_removed_commands_and_options_are_breaking(self):
        base = {
            "commands": {
                "ctx": {"options": {"--json": "old"}, "subcommands": ["map"]},
                "ctx map": {"options": {"--budget": "old"}, "subcommands": []},
            }
        }
        current = {
            "commands": {
                "ctx": {"options": {"--json": "new"}, "subcommands": []},
            }
        }
        removed, changed = contracts.compare_contracts(base, current)
        self.assertIn("command ctx map", removed)
        self.assertIn("option contract ctx --json", changed)


if __name__ == "__main__":
    unittest.main()
