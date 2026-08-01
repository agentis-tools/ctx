import hashlib
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "ctx_validate_release_assets", ROOT / "scripts/validate-release-assets.py"
)
assets = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = assets
SPEC.loader.exec_module(assets)


class ReleaseChecksumTests(unittest.TestCase):
    def test_valid_manifest_covers_exact_artifact_set(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "ctx.tar.gz"
            artifact.write_bytes(b"payload")
            digest = hashlib.sha256(b"payload").hexdigest()
            sums = root / "SHA256SUMS"
            sums.write_text(f"{digest}  {artifact.name}\n", encoding="utf-8")

            assets.validate_checksums(root, sums)

    def test_self_references_are_rejected_after_path_normalization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sums = root / "SHA256SUMS"
            digest = "0" * 64
            for name in ("SHA256SUMS", "./SHA256SUMS", "nested/../SHA256SUMS"):
                sums.write_text(f"{digest}  {name}\n", encoding="utf-8")
                with self.subTest(name=name), self.assertRaises(SystemExit):
                    assets.read_checksums(sums)

    def test_missing_and_extra_entries_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "ctx.tar.gz"
            artifact.write_bytes(b"payload")
            digest = hashlib.sha256(b"payload").hexdigest()
            sums = root / "SHA256SUMS"

            sums.write_text(
                f"{digest}  {artifact.name}\n"
                f"{digest}  extra.zip\n",
                encoding="utf-8",
            )
            with self.assertRaises(SystemExit):
                assets.validate_checksums(root, sums)

            sums.write_text("", encoding="utf-8")
            with self.assertRaises(SystemExit):
                assets.validate_checksums(root, sums)

    def test_wrong_digest_is_rejected_with_optimization_enabled(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "ctx.tar.gz"
            artifact.write_bytes(b"payload")
            sums = root / "SHA256SUMS"
            sums.write_text(f"{'0' * 64}  {artifact.name}\n", encoding="utf-8")

            with self.assertRaises(SystemExit):
                assets.validate_checksums(root, sums)


if __name__ == "__main__":
    unittest.main()
