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

    def _run_pr_policy(self, labels, changelog_diff):
        """Exercise pr_policy against a simulated release diff.

        No contract change and CHANGELOG.md is the only touched file (not a
        SENSITIVE_PATH), so the only gate that can fire is the breaking-change
        acknowledgement. Module I/O is stubbed so no real git repo is needed.
        """
        contract = {"schema": 1, "commands": {"ctx": {"options": {}, "subcommands": []}}}

        def fake_git_output(*args):
            if "--name-only" in args:
                return "CHANGELOG.md\n"
            if args and args[-1] == "CHANGELOG.md":
                return changelog_diff
            return ""

        saved = (contracts.load_contract, contracts.contract_from_ref, contracts.git_output)
        contracts.load_contract = lambda *a, **k: contract
        contracts.contract_from_ref = lambda ref: contract
        contracts.git_output = fake_git_output
        try:
            contracts.pr_policy("main", set(labels))
        finally:
            (contracts.load_contract, contracts.contract_from_ref, contracts.git_output) = saved

    def test_release_preparation_exempts_breaking_change_relocation(self):
        # A release PR relocates already-acknowledged BREAKING entries into the
        # dated section: the entry line is unchanged context, only the version
        # header is added -- no new "+- BREAKING:" line. version.py enforces the
        # release side, so release-preparation is exempt here.
        relocation_diff = "\n".join(
            [
                " ## [Unreleased]",
                "+## [0.4.0] - 2026-07-18",
                " ### Fixed",
                " - BREAKING: caller lookup narrowed (#61)",
            ]
        )
        # release PR: exempt -> passes
        self._run_pr_policy(
            {"breaking-change", "release-preparation", "contract-review"}, relocation_diff
        )
        # same diff on a feature PR (no release-preparation): still enforced
        with self.assertRaises(contracts.ContractError):
            self._run_pr_policy({"breaking-change", "contract-review"}, relocation_diff)

    def test_feature_pr_adding_breaking_entry_passes(self):
        # A feature PR that genuinely adds a "- BREAKING:" entry satisfies the gate.
        added_diff = "\n".join(
            [
                " ### Fixed",
                "+- BREAKING: new incompatible behavior (#99)",
            ]
        )
        self._run_pr_policy({"breaking-change", "contract-review"}, added_diff)


if __name__ == "__main__":
    unittest.main()
