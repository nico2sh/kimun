import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_manifest as rm  # noqa: E402

H = "a" * 64
H2 = "b" * 64


def asset(name):
    return {"name": name, "browser_download_url": f"https://dl/{name}"}


def release(tag, assets=(), draft=False, published="2026-11-09T00:00:00Z"):
    return {
        "tag_name": tag,
        "draft": draft,
        "published_at": published,
        "html_url": f"https://page/{tag}",
        "assets": [asset(n) for n in assets],
    }


def fetcher(sums_by_url):
    def fetch(url):
        if url not in sums_by_url:
            raise OSError("404")
        return sums_by_url[url]
    return fetch


class Parse(unittest.TestCase):
    def test_two_column_format_and_star(self):
        text = f"{H}  kimun-1.0.0-linux-x64.tar.gz\n{H2} *kimun-1.0.0-windows-x64.exe\n"
        self.assertEqual(
            rm.parse_checksums(text),
            {"kimun-1.0.0-linux-x64.tar.gz": H, "kimun-1.0.0-windows-x64.exe": H2},
        )

    def test_junk_lines_and_bad_digests_are_ignored(self):
        text = f"\nnot a line\nzz  name\n{H}  ok\n"
        self.assertEqual(rm.parse_checksums(text), {"ok": H})

    def test_digest_is_lowercased(self):
        self.assertEqual(rm.parse_checksums(f"{'A' * 64}  n\n"), {"n": "a" * 64})


class Pick(unittest.TestCase):
    def test_skips_prerelease_draft_and_other_series(self):
        rels = [
            release("kimun-notes-v0.26.0", draft=True),
            release("kimun-notes-v0.25.0-beta.1"),
            release("kimun_core-v0.8.0"),
            release("kimun-notes-v0.24.8"),
            release("kimun-notes-v0.24.7"),
        ]
        version, r = rm.newest_stable(rels, "kimun-notes-v")
        self.assertEqual(version, "0.24.8")
        self.assertEqual(r["tag_name"], "kimun-notes-v0.24.8")

    def test_highest_version_wins_over_list_order(self):
        rels = [
            release("kimun-notes-v0.24.9"),  # a hotfix cut from an older commit sorts first
            release("kimun-notes-v0.24.10"),
            release("kimun-notes-v0.24.8"),
        ]
        self.assertEqual(rm.newest_stable(rels, "kimun-notes-v")[0], "0.24.10")

    def test_non_plain_versions_are_skipped(self):
        rels = [release("kimun-notes-v1"), release("kimun-notes-v0.9.0")]
        self.assertEqual(rm.newest_stable(rels, "kimun-notes-v")[0], "0.9.0")

    def test_none_when_the_series_has_nothing_stable(self):
        self.assertIsNone(rm.newest_stable([release("kimun-notes-v1.0.0-rc.1")], "kimun-notes-v"))

    def test_server_prefix(self):
        rels = [release("kimun-notes-v0.24.8"), release("kimun_server-v0.3.1")]
        self.assertEqual(rm.newest_stable(rels, "kimun_server-v")[0], "0.3.1")


class Build(unittest.TestCase):
    def setUp(self):
        self.warnings = []

    def warn(self, m):
        self.warnings.append(m)

    def test_both_series_keyed_by_exact_name(self):
        rels = [
            release("kimun_server-v0.3.1", ["kimun-server-0.3.1-linux-x64.tar.gz", rm.CHECKSUMS]),
            release("kimun-notes-v0.24.8", ["kimun-0.24.8-linux-x64.tar.gz", "kimun-gui-0.24.8-linux-x64.tar.gz", rm.CHECKSUMS]),
        ]
        sums = {
            f"https://dl/{rm.CHECKSUMS}": f"{H}  kimun-0.24.8-linux-x64.tar.gz\n{H2}  kimun-gui-0.24.8-linux-x64.tar.gz\n"
            f"{H}  kimun-server-0.3.1-linux-x64.tar.gz\n",
        }
        m = rm.build(rels, fetcher(sums), self.warn)
        self.assertEqual(m["schema"], 1)
        self.assertEqual(m["app"]["version"], "0.24.8")
        self.assertEqual(m["app"]["page"], "https://page/kimun-notes-v0.24.8")
        self.assertEqual(
            m["app"]["assets"]["kimun-gui-0.24.8-linux-x64.tar.gz"],
            {"url": "https://dl/kimun-gui-0.24.8-linux-x64.tar.gz", "sha256": H2},
        )
        self.assertNotIn(rm.CHECKSUMS, m["app"]["assets"])
        self.assertEqual(m["server"]["version"], "0.3.1")
        self.assertEqual(self.warnings, [])

    def test_stable_downloads_are_listed_beside_the_versioned_files(self):
        # ADR-0057: the versionless copies are ordinary checksummed files. They get their
        # own entry, and the versioned names the installers look up stay as they were.
        versioned, stable = "kimun-gui-1.0.0-macos-arm64.dmg", "kimun-gui-macos-arm64.dmg"
        rels = [release("kimun-notes-v1.0.0", [versioned, stable, rm.CHECKSUMS])]
        sums = {f"https://dl/{rm.CHECKSUMS}": f"{H}  {versioned}\n{H}  {stable}\n"}
        m = rm.build(rels, fetcher(sums), self.warn)
        self.assertEqual(list(m["app"]["assets"]), [versioned, stable])
        self.assertEqual(m["app"]["assets"][stable]["sha256"], m["app"]["assets"][versioned]["sha256"])
        self.assertEqual(m["app"]["assets"][stable]["url"], f"https://dl/{stable}")
        self.assertEqual([w for w in self.warnings if w.startswith("app")], [])

    def test_signature_url_only_when_the_release_was_signed(self):
        sums = {f"https://dl/{rm.CHECKSUMS}": f"{H}  a.zip\n"}
        signed = [release("kimun-notes-v1.0.0", ["a.zip", rm.CHECKSUMS, rm.SIGNATURE])]
        app = rm.build(signed, fetcher(sums), self.warn)["app"]
        self.assertEqual(app["checksums_url"], f"https://dl/{rm.CHECKSUMS}")
        self.assertEqual(app["signature_url"], f"https://dl/{rm.SIGNATURE}")
        self.assertEqual(list(app["assets"]), ["a.zip"])  # the .minisig is not a listed file
        unsigned = [release("kimun-notes-v1.0.0", ["a.zip", rm.CHECKSUMS])]
        app = rm.build(unsigned, fetcher(sums), self.warn)["app"]
        self.assertNotIn("signature_url", app)
        self.assertEqual([w for w in self.warnings if w.startswith("app")], [])

    def test_only_files_both_attached_and_checksummed(self):
        rels = [release("kimun-notes-v1.0.0", ["a.zip", "b.zip", rm.CHECKSUMS])]
        sums = {f"https://dl/{rm.CHECKSUMS}": f"{H}  a.zip\n{H2}  not-attached.zip\n"}
        m = rm.build(rels, fetcher(sums), self.warn)
        self.assertEqual(list(m["app"]["assets"]), ["a.zip"])
        self.assertTrue(any("b.zip" in w and "no checksum" in w for w in self.warnings), self.warnings)

    def test_series_without_checksums_is_left_out_not_filled_from_an_older_release(self):
        rels = [
            release("kimun-notes-v1.1.0", ["x.zip"]),  # newest, but no checksums file
            release("kimun-notes-v1.0.0", ["y.zip", rm.CHECKSUMS]),
        ]
        sums = {f"https://dl/{rm.CHECKSUMS}": f"{H}  y.zip\n"}
        m = rm.build(rels, fetcher(sums), self.warn)
        self.assertNotIn("app", m)
        self.assertTrue(any("1.1.0" in w and "checksums" in w for w in self.warnings), self.warnings)

    def test_unreadable_checksums_leave_the_series_out(self):
        rels = [release("kimun-notes-v1.0.0", ["y.zip", rm.CHECKSUMS])]
        m = rm.build(rels, fetcher({}), self.warn)
        self.assertNotIn("app", m)
        self.assertTrue(any("cannot read" in w for w in self.warnings), self.warnings)

    def test_one_series_failing_keeps_the_other(self):
        rels = [
            release("kimun_server-v0.3.1", ["s.zip"]),  # no checksums
            release("kimun-notes-v1.0.0", ["a.zip", rm.CHECKSUMS]),
        ]
        sums = {f"https://dl/{rm.CHECKSUMS}": f"{H}  a.zip\n"}
        m = rm.build(rels, fetcher(sums), self.warn)
        self.assertIn("app", m)
        self.assertNotIn("server", m)


class Flatten(unittest.TestCase):
    def test_slurped_pages_and_a_plain_list(self):
        a, b = release("kimun-notes-v1"), release("kimun-notes-v2")
        self.assertEqual(rm.flatten([[a], [b]]), [a, b])
        self.assertEqual(rm.flatten([a, b]), [a, b])
        self.assertEqual(rm.flatten([]), [])


class Cli(unittest.TestCase):
    def run_cli(self, releases):
        with tempfile.TemporaryDirectory() as d:
            src, out = Path(d, "r.json"), Path(d, "latest.json")
            src.write_text(json.dumps(releases))
            p = subprocess.run(
                [sys.executable, "-I", str(Path(rm.__file__)), str(src), str(out)],
                capture_output=True, text=True,
            )
            return p, (json.loads(out.read_text()) if out.exists() else None)

    def test_the_layout_the_shell_installers_read_with_awk(self):
        rels = [[release("kimun-notes-v1.2.3", ["a.zip", "b.zip", rm.CHECKSUMS])]]
        with tempfile.TemporaryDirectory() as d:
            sums = Path(d, "sums.txt")
            sums.write_text(f"{H}  a.zip\n{H2}  b.zip\n")
            for page in rels:
                for r in page:
                    for a in r["assets"]:
                        a["browser_download_url"] = sums.as_uri() if a["name"] == rm.CHECKSUMS else f"https://dl/{a['name']}"
            src, out = Path(d, "r.json"), Path(d, "latest.json")
            src.write_text(json.dumps(rels))
            p = subprocess.run([sys.executable, "-I", str(Path(rm.__file__)), str(src), str(out)],
                               capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            lines = out.read_text().splitlines()
        for want in (
            '  "schema": 1,',
            '  "app": {',
            '    "version": "1.2.3",',
            '    "assets": {',
            '      "a.zip": {',
            '        "url": "https://dl/a.zip",',
            f'        "sha256": "{H2}"',
            '      "b.zip": {',
        ):
            self.assertIn(want, lines)

    def test_exit_1_and_no_file_when_nothing_can_be_described(self):
        p, out = self.run_cli([[release("kimun-notes-v1.0.0-beta.1")]])
        self.assertEqual(p.returncode, 1, p.stderr)
        self.assertIsNone(out)


if __name__ == "__main__":
    unittest.main()
