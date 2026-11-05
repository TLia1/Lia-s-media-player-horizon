"""Regression tests for release layout, reproducibility, and atomic validation."""
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import package


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.mod = self.root / "mixins/testmod"
        self.mod.mkdir(parents=True)
        (self.root / "horizon-native.json").write_text(json.dumps({
            "id": "testmod", "version": "1.0.0", "horizon": "revision", "hooks": 359}))
        (self.root / "compatibility.json").write_text('{"hooks":359}')
        (self.mod / "horizon-mixin.toml").write_text(
            '[mod]\nid="testmod"\nversion="1.0.0"\n[compatibility]\nhorizon_revision="revision"\n')
        (self.mod / "lib.rs").write_text("pub fn hello() {}")

    @patch("package.subprocess.run")
    def test_reproducible_source_only_bundle(self, run):
        fixtures = self.mod / "tests/fixtures"
        fixtures.mkdir(parents=True)
        (fixtures / "test.png").write_bytes(b"png")
        out = package.verify_and_package(self.root)
        first = out.read_bytes()
        package.verify_and_package(self.root)
        self.assertEqual(first, out.read_bytes())
        self.assertEqual(run.call_count, 2)
        self.assertTrue(run.call_args.kwargs["check"])
        with zipfile.ZipFile(out) as archive:
            self.assertEqual(len(archive.namelist()), 3)
            self.assertFalse(any("tests/" in n for n in archive.namelist()))
        self.assertTrue(pathlib.Path(str(out) + ".sha256").is_file())

    @patch("package.subprocess.run")
    def test_failed_validation_preserves_previous_release(self, run):
        out = package.verify_and_package(self.root)
        previous = out.read_bytes()
        (self.mod / "lib.rs").write_text("bad source")
        run.side_effect = subprocess.CalledProcessError(1, ["validator"])
        with self.assertRaises(subprocess.CalledProcessError):
            package.verify_and_package(self.root)
        self.assertEqual(out.read_bytes(), previous)
        self.assertFalse(list(self.root.glob(".hnmod-*")))

    @patch("package.subprocess.run")
    def test_changed_bundle_removes_stale_signature(self, run):
        out = package.verify_and_package(self.root)
        signature = pathlib.Path(str(out) + ".minisig")
        signature.write_text("old signature")
        package.verify_and_package(self.root)
        self.assertTrue(signature.exists())
        (self.mod / "lib.rs").write_text("pub fn changed() {}")
        package.verify_and_package(self.root)
        self.assertFalse(signature.exists())

    def test_rejects_symlinks_and_binary_sources(self):
        link = self.mod / "secret.rs"
        link.symlink_to(self.root / "horizon-native.json")
        with self.assertRaisesRegex(ValueError, "symlinks"):
            package.bundle_entries(self.root)
        link.unlink()
        (self.mod / "secret.key").write_text("not a source")
        with self.assertRaisesRegex(ValueError, "unexpected native source"):
            package.bundle_entries(self.root)

    def test_rejects_manifest_mismatch(self):
        (self.root / "compatibility.json").write_text('{"hooks":409}')
        with self.assertRaisesRegex(ValueError, "hooks version mismatch"):
            package.bundle_entries(self.root)


if __name__ == "__main__":
    unittest.main()
