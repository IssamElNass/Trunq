"""Checks release routing without building binaries or publishing releases."""

from datetime import datetime, timezone
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("release_metadata", Path(__file__).with_name("release-metadata.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
SHA = "a" * 40
NOW = datetime(2026, 10, 24, 2, tzinfo=timezone.utc)


class ReleaseMetadataTests(unittest.TestCase):
    def make(self, event="push", tag="v0.1.0", version="0.1.0", run="17"):
        return release.metadata(version, SHA, event, tag, run, NOW)

    def test_stable_tag_must_match_manifest(self):
        self.assertEqual(self.make()["tag"], "v0.1.0")
        self.assertEqual(self.make()["prerelease"], "false")
        with self.assertRaises(ValueError):
            self.make(tag="v0.2.0")

    def test_prerelease_detection_ignores_build_metadata(self):
        self.assertEqual(self.make(version="0.2.0-rc.1", tag="v0.2.0-rc.1")["prerelease"], "true")
        self.assertEqual(self.make(version="0.2.0+test-build", tag="v0.2.0+test-build")["prerelease"], "false")

    def test_schedule_preview_and_manual_nightly_have_snapshot_identity(self):
        for event, tag in [("schedule", "main"), ("pull_request", "feature"), ("workflow_dispatch", "nightly")]:
            with self.subTest(event=event):
                values = self.make(event=event, tag=tag)
                self.assertEqual(values["tag"], "nightly-20261024-17-gaaaaaaa")
                self.assertEqual(values["version"], values["tag"])
                self.assertEqual(values["sha"], SHA)
                self.assertEqual(values["nightly"], "true")
                self.assertEqual(values["prerelease"], "true")
                self.assertEqual(values["deb_version"], "0.1.0~nightly.20261024.17+gaaaaaaa")

    def test_repeated_runs_have_distinct_tags(self):
        self.assertNotEqual(self.make(event="schedule")["tag"], self.make(event="schedule", run="18")["tag"])

    def test_invalid_identity_cannot_become_an_output(self):
        for version in ["0.1.0\nnightly=true", "invalid", "0.1"]:
            with self.assertRaises(ValueError):
                self.make(version=version)
        with self.assertRaises(ValueError):
            self.make(event="schedule", run="0")


if __name__ == "__main__":
    unittest.main()
