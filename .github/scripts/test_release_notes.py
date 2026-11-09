"""Tests for release_notes.py. Run: python3 -m unittest discover -s .github/scripts"""
import tempfile
import unittest
from pathlib import Path

import release_notes as rn


class ReleaseNotes(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp()) / "release-notes"

    def test_the_app_version_is_read_from_the_manifest(self):
        # semver: major.minor.patch, optionally a pre-release (0.25.0-beta.1).
        self.assertRegex(rn.current_version(), r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$")

    def test_scaffold_makes_the_versioned_file_once(self):
        self.assertTrue(rn.scaffold("1.2.3", self.dir))
        path = rn.notes_path("1.2.3", self.dir)
        self.assertEqual(path.name, "1.2.3.md")
        self.assertIn("Release notes for 1.2.3", path.read_text())
        # A second run keeps what is there, edits included.
        path.write_text("Mine.\n")
        self.assertFalse(rn.scaffold("1.2.3", self.dir))
        self.assertEqual(path.read_text(), "Mine.\n")

    def test_the_template_is_not_notes(self):
        rn.scaffold("1.2.3", self.dir)
        self.assertIn("still the empty template", rn.problem("1.2.3", self.dir))

    def test_a_missing_file_is_a_problem(self):
        self.assertIn("is missing", rn.problem("9.9.9", self.dir))

    def test_text_beside_the_comment_is_notes(self):
        rn.scaffold("1.2.3", self.dir)
        path = rn.notes_path("1.2.3", self.dir)
        path.write_text(path.read_text() + "\nA desktop app.\n")
        self.assertIsNone(rn.problem("1.2.3", self.dir))

    def test_written_ignores_comments_and_whitespace_only(self):
        self.assertFalse(rn.written(""))
        self.assertFalse(rn.written("  \n"))
        self.assertFalse(rn.written("<!-- a\nmultiline\ncomment -->\n\n"))
        self.assertTrue(rn.written("<!-- c -->\nWhat is new\n"))
        self.assertTrue(rn.written("Plain."))

    def test_a_version_with_a_release_tag_is_released(self):
        tags = ["kimun-notes-v0.24.6", "kimun-notes-v0.24.7"]
        self.assertTrue(rn.is_released("0.24.7", tags))
        self.assertFalse(rn.is_released("0.25.0", tags))
        # The server's tags are another series.
        self.assertFalse(rn.is_released("0.5.0", ["kimun_server-v0.5.0"]))


if __name__ == "__main__":
    unittest.main()
