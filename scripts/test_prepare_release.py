"""Release metadata regression tests; no Rust toolchain is invoked."""

import os
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare-release.py")
MANIFEST = b'''# Keep the source version for local/manual builds.
[workspace]
members = ["crates/*"]

[workspace.package]
# All workspace members inherit this version.
version = "0.1.0" # workspace version
edition = "2024"

[workspace.dependencies]
example = { version = "0.1.0", features = ["std"] }
'''


class PrepareReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest = self.root / "Cargo.toml"
        self.manifest.write_bytes(MANIFEST)
        self.output = self.root / "output"

    def run_script(self, event, tag):
        return subprocess.run(
            [sys.executable, str(SCRIPT)],
            cwd=self.root,
            env={
                **os.environ,
                "GITHUB_EVENT_NAME": event,
                "RELEASE_TAG": tag,
                "GITHUB_OUTPUT": str(self.output),
            },
            text=True,
            capture_output=True,
            check=False,
        )

    def test_release_uses_tag_version_without_changing_dependencies(self):
        result = self.run_script("release", "v0.1.1")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.manifest.read_bytes(),
            MANIFEST.replace(b'version = "0.1.0" #', b'version = "0.1.1" #'),
        )
        self.assertEqual(self.output.read_text(), "version=0.1.1\n")

    def test_manual_build_preserves_manifest_even_with_a_tag(self):
        result = self.run_script("workflow_dispatch", "v0.1.1")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.manifest.read_bytes(), MANIFEST)
        self.assertEqual(self.output.read_text(), "version=0.1.0\n")

    def test_valid_semver_tags(self):
        for tag in ("v0.0.0", "v2.10.3", "v1.2.3-rc.1", "v1.2.3+build.4"):
            with self.subTest(tag=tag):
                self.manifest.write_bytes(MANIFEST)
                result = self.run_script("release", tag)

                self.assertEqual(result.returncode, 0, result.stderr)
                data = tomllib.loads(self.manifest.read_text())
                self.assertEqual(data["workspace"]["package"]["version"], tag[1:])

    def test_invalid_tags_fail_before_changing_manifest(self):
        for tag in ("", "0.1.1", "v01.2.3", "v1.2", "v1.2.3-01", "v1.2.3\n"):
            with self.subTest(tag=tag):
                result = self.run_script("release", tag)

                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Invalid release tag", result.stderr)
                self.assertEqual(self.manifest.read_bytes(), MANIFEST)
                self.assertFalse(self.output.exists())

    def test_build_and_publish_resolve_the_same_version(self):
        build = self.run_script("release", "v0.1.1")
        self.assertEqual(build.returncode, 0, build.stderr)
        build_output = self.output.read_text()
        # The publish job checks out the original tagged sources again.
        self.manifest.write_bytes(MANIFEST)
        self.output.write_text("")

        publish = self.run_script("release", "v0.1.1")

        self.assertEqual(publish.returncode, 0, publish.stderr)
        self.assertEqual(self.output.read_text(), build_output)


if __name__ == "__main__":
    unittest.main()
