"""Tests for check-agent-guidance.py. Run: python3 -m unittest discover -s .github/scripts"""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "check-agent-guidance.py"
GUIDANCE = "<!-- short:start -->\nShort text.\n<!-- short:end -->\n\n# Full\n\nMore.\n"
FRONTMATTER = "---\nname: kimun-ask\ndescription: x\n---\n\n"
SERVER = 'fn x() { include_str!("guidance.md"); }\n'


class CheckAgentGuidance(unittest.TestCase):
    def run_check(self, guidance=GUIDANCE, skill=None, server=SERVER):
        d = Path(tempfile.mkdtemp())
        files = {
            "guidance.md": guidance,
            "SKILL.md": FRONTMATTER + GUIDANCE if skill is None else skill,
            "mod.rs": server,
        }
        for name, text in files.items():
            (d / name).write_text(text)
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT)] + [str(d / n) for n in files],
            capture_output=True,
            text=True,
        )

    def test_matching_files_pass(self):
        self.assertEqual(self.run_check().returncode, 0)

    def test_a_skill_that_differs_fails(self):
        r = self.run_check(skill=FRONTMATTER + GUIDANCE + "Extra line.\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("SKILL.md", r.stdout)

    def test_an_instructions_source_that_differs_fails(self):
        r = self.run_check(server='const S: &str = "Old short text.";\n')
        self.assertEqual(r.returncode, 1)
        self.assertIn("server instructions", r.stdout)

    def test_a_literal_copy_of_the_short_block_passes(self):
        self.assertEqual(
            self.run_check(server='const S: &str = "Short text.";\n').returncode, 0
        )

    def test_the_real_files_match(self):
        root = Path(__file__).resolve().parents[2]
        r = subprocess.run(
            [sys.executable, "-I", str(SCRIPT)], cwd=root, capture_output=True, text=True
        )
        self.assertEqual(r.returncode, 0, r.stdout)


if __name__ == "__main__":
    unittest.main()
