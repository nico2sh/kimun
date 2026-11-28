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
CLI = "---\nname: kimun-cli\n---\n\n# CLI\n"
WIKI = "---\nname: kimun-wiki\n---\n\n# Wiki\n"


class CheckAgentGuidance(unittest.TestCase):
    def run_check(
        self,
        guidance=GUIDANCE,
        skill=None,
        server=SERVER,
        cli_skill=CLI,
        cli_embedded=CLI,
        ask_frontmatter=FRONTMATTER,
        wiki_skill=WIKI,
        wiki_embedded=WIKI,
    ):
        d = Path(tempfile.mkdtemp())
        files = {
            "guidance.md": guidance,
            "SKILL.md": FRONTMATTER + GUIDANCE if skill is None else skill,
            "mod.rs": server,
            "cli-SKILL.md": cli_skill,
            "kimun-cli.md": cli_embedded,
            "kimun-ask.frontmatter.md": ask_frontmatter,
            "wiki-SKILL.md": wiki_skill,
            "kimun-wiki.md": wiki_embedded,
        }
        for name, text in files.items():
            (d / name).write_bytes(text.encode())
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

    def test_a_matching_embedded_cli_skill_passes(self):
        self.assertEqual(self.run_check(cli_skill=CLI, cli_embedded=CLI).returncode, 0)

    def test_an_embedded_cli_skill_that_differs_fails(self):
        r = self.run_check(cli_embedded=CLI + "Old line.\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("kimun-cli", r.stdout)

    def test_a_matching_ask_frontmatter_passes(self):
        r = self.run_check(skill=FRONTMATTER + GUIDANCE, ask_frontmatter=FRONTMATTER)
        self.assertEqual(r.returncode, 0)

    def test_an_ask_frontmatter_that_differs_fails(self):
        # The body still matches, so only the new byte comparison catches it.
        r = self.run_check(ask_frontmatter="---\nname: kimun-ask\ndescription: old\n---\n\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("kimun-ask.frontmatter.md", r.stdout)

    def test_a_matching_embedded_wiki_skill_passes(self):
        self.assertEqual(
            self.run_check(wiki_skill=WIKI, wiki_embedded=WIKI).returncode, 0
        )

    def test_an_embedded_wiki_skill_that_differs_fails(self):
        r = self.run_check(wiki_embedded=WIKI + "Old line.\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("kimun-wiki", r.stdout)

    def test_the_real_files_match(self):
        root = Path(__file__).resolve().parents[2]
        r = subprocess.run(
            [sys.executable, "-I", str(SCRIPT)], cwd=root, capture_output=True, text=True
        )
        self.assertEqual(r.returncode, 0, r.stdout)


if __name__ == "__main__":
    unittest.main()
