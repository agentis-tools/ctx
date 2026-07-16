#!/usr/bin/env python3

import importlib.util
from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("ctx_version", ROOT / "scripts/version.py")
version = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = version
SPEC.loader.exec_module(version)


class SemVerTests(unittest.TestCase):
    def test_accepts_full_semver(self):
        parsed = version.SemVer.parse("1.2.3-rc.1+build.9")
        self.assertEqual(str(parsed), "1.2.3-rc.1+build.9")

    def test_rejects_incomplete_and_leading_zero_versions(self):
        for value in ("1.2", "v1.2.3", "01.2.3", "1.2.3-01"):
            with self.subTest(value=value), self.assertRaises(version.PolicyError):
                version.SemVer.parse(value)

    def test_semver_precedence(self):
        ordered = ["0.4.0-alpha.1", "0.4.0-alpha.2", "0.4.0-rc.1", "0.4.0"]
        parsed = [version.SemVer.parse(item) for item in ordered]
        for left, right in zip(parsed, parsed[1:]):
            self.assertLess(left.compare(right), 0)
        self.assertEqual(
            version.SemVer.parse("1.0.0+one").compare(
                version.SemVer.parse("1.0.0+two")
            ),
            0,
        )


class RewriteTests(unittest.TestCase):
    def test_manifest_rewrite_touches_package_version_only(self):
        source = '[package]\nname = "agentis-ctx"\nversion = "0.3.4"\n\n[dependencies]\nx = "0.3.4"\n'
        updated = version.replace_manifest_version(source, "0.3.4", "0.4.0")
        self.assertIn('version = "0.4.0"', updated)
        self.assertIn('x = "0.3.4"', updated)

    def test_lock_rewrite_touches_named_local_package_only(self):
        source = '[[package]]\nname = "agentis-ctx"\nversion = "0.3.4"\n\n[[package]]\nname = "other"\nversion = "0.3.4"\n'
        updated = version.replace_lock_version(source, "0.3.4", "0.4.0")
        self.assertIn('name = "agentis-ctx"\nversion = "0.4.0"', updated)
        self.assertIn('name = "other"\nversion = "0.3.4"', updated)

    def test_finalize_changelog_moves_unreleased_notes_and_links(self):
        source = """# Changelog

## [Unreleased]

### Added
- A reviewed feature (#1)

## [0.3.4] - 2026-07-12

### Fixed
- Old fix

[Unreleased]: https://github.com/agentis-tools/ctx/compare/v0.3.4...HEAD
[0.3.4]: https://github.com/agentis-tools/ctx/releases/tag/v0.3.4
"""
        updated = version.finalize_changelog(
            source, "0.3.4", "0.4.0", "2026-07-13", False
        )
        self.assertIn("## [Unreleased]\n\n## [0.4.0] - 2026-07-13", updated)
        self.assertIn("compare/v0.4.0...HEAD", updated)
        self.assertIn("compare/v0.3.4...v0.4.0", updated)
        self.assertEqual(version.changelog_section(updated, "0.4.0").count("# Added"), 1)

    def test_empty_release_requires_explicit_override(self):
        source = "## [Unreleased]\n\n## [0.3.4] - 2026-07-12\n\n- prior\n\n[Unreleased]: https://github.com/agentis-tools/ctx/compare/v0.3.4...HEAD\n"
        with self.assertRaises(version.PolicyError):
            version.finalize_changelog(
                source, "0.3.4", "0.3.5", "2026-07-13", False
            )


class GitHubEnvironmentTests(unittest.TestCase):
    def test_branch_ref_is_not_implicitly_a_release_tag(self):
        with mock.patch.dict(
            version.os.environ,
            {"GITHUB_REF_NAME": "123/merge", "GITHUB_REF_TYPE": "branch"},
            clear=True,
        ):
            github_tag = version.tag_from_environment(version.os.environ)
        self.assertIsNone(github_tag)

    def test_tag_ref_is_used_for_release_validation(self):
        environment = {"GITHUB_REF_NAME": "v0.3.4", "GITHUB_REF_TYPE": "tag"}
        self.assertEqual(version.tag_from_environment(environment), "v0.3.4")


class BreakingBumpTests(unittest.TestCase):
    """Breaks land under Unreleased; the version they owe is due at release."""

    BREAKING = (
        "# Changelog\n\n## [Unreleased]\n\n### Fixed\n"
        "- BREAKING: `ctx index` now exits 2 when a scope matches nothing.\n\n"
        "## [0.3.5] - 2026-07-01\n"
    )
    COMPATIBLE = (
        "# Changelog\n\n## [Unreleased]\n\n### Fixed\n"
        "- Diff selection is now deterministic.\n\n"
        "## [0.3.5] - 2026-07-01\n"
    )

    def check(self, text, old, new):
        version.require_breaking_bump(
            text, version.SemVer.parse(old), version.SemVer.parse(new)
        )

    def test_pre_1_0_break_requires_a_minor_bump(self):
        with self.assertRaises(version.PolicyError) as caught:
            self.check(self.BREAKING, "0.3.5", "0.3.6")
        self.assertIn("BREAKING:", str(caught.exception))
        self.assertIn("minor", str(caught.exception))

    def test_pre_1_0_break_accepts_a_minor_bump(self):
        self.check(self.BREAKING, "0.3.5", "0.4.0")

    def test_post_1_0_break_requires_a_major_bump(self):
        with self.assertRaises(version.PolicyError):
            self.check(self.BREAKING, "1.2.3", "1.3.0")
        self.check(self.BREAKING, "1.2.3", "2.0.0")

    def test_compatible_release_is_unaffected_by_the_gate(self):
        self.check(self.COMPATIBLE, "0.3.5", "0.3.6")

    def test_only_the_unreleased_section_is_considered(self):
        already_released = (
            "# Changelog\n\n## [Unreleased]\n\n### Fixed\n- A compatible fix.\n\n"
            "## [0.4.0] - 2026-07-01\n\n### Changed\n- BREAKING: an old break.\n"
        )
        self.check(already_released, "0.4.0", "0.4.1")


if __name__ == "__main__":
    unittest.main()
