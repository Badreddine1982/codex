"""Unit tests for scripts/generate_changelog.py (uses real temp git repos).

Run with:  python3 -m unittest discover -s scripts -p 'test_generate_changelog.py'
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "generate_changelog", Path(__file__).with_name("generate_changelog.py")
)
generate_changelog = importlib.util.module_from_spec(_SPEC)
sys.modules["generate_changelog"] = generate_changelog
_SPEC.loader.exec_module(generate_changelog)  # type: ignore[union-attr]


class GitRepo:
    """Minimal throwaway git repository factory."""

    def __init__(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="changelog-test-"))
        self.git("init", "-q")
        self.git("config", "user.email", "t@example.com")
        self.git("config", "user.name", "T")

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(self.root), *args],
            capture_output=True,
            text=True,
            check=True,
        ).stdout

    def commit(self, message: str) -> None:
        (self.root / "f.txt").write_text(message, encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)

    def tag(self, name: str) -> None:
        self.git("tag", name)


class TestCollectAndRender(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = GitRepo()

    def test_parsing_and_sections(self) -> None:
        self.repo.commit("feat(tui): add status bar")
        self.repo.commit("fix: correct crash on resume")
        self.repo.commit("perf(core): faster startup")
        self.repo.commit("chore: bump deps")          # conventional -> Other Changes
        self.repo.commit("no conventional subject")   # kept -> Other Changes

        entries = generate_changelog.collect_entries(self.repo.root, None)
        self.assertEqual(len(entries), 5)
        md = generate_changelog.render(entries, "Unreleased")
        self.assertIn("### Features", md)
        self.assertIn("add status bar", md)
        self.assertIn("**tui**: add status bar", md)
        self.assertIn("### Bug Fixes", md)
        self.assertIn("### Performance", md)
        self.assertIn("**core**: faster startup", md)
        self.assertIn("### Other Changes", md)
        self.assertIn("bump deps", md)
        self.assertIn("no conventional subject", md)

    def test_github_squash_subjects_kept_with_pr_number(self) -> None:
        self.repo.commit("Identify Mac mini hosts in remote control handshakes (#38840)")
        entries = generate_changelog.collect_entries(self.repo.root, None)
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0].pr_number, 38840)
        self.assertEqual(entries[0].type, "other")
        self.assertEqual(
            entries[0].description,
            "Identify Mac mini hosts in remote control handshakes",
        )
        md = generate_changelog.render(entries, "Unreleased")
        self.assertIn("(#38840)", md)

    def test_pr_link_rendered_when_base_url_given(self) -> None:
        self.repo.commit("feat: shiny thing (#42)")
        entries = generate_changelog.collect_entries(self.repo.root, None)
        md = generate_changelog.render(entries, "Unreleased", "https://github.com/acme/app")
        self.assertIn(
            "[#42](https://github.com/acme/app/pull/42)", md
        )

    def test_remote_url_parsing(self) -> None:
        for remote, expected in (
            ("git@github.com:acme/app.git", "https://github.com/acme/app"),
            ("https://github.com/acme/app.git", "https://github.com/acme/app"),
            ("https://github.com/acme/app", "https://github.com/acme/app"),
            ("ssh://git@github.com/acme/app.git", "https://github.com/acme/app"),
        ):
            self.repo.git("remote", "add", "origin", remote)
            self.assertEqual(
                generate_changelog.repo_http_url(self.repo.root), expected
            )
            self.repo.git("remote", "remove", "origin")

    def test_no_origin_remote_returns_none(self) -> None:
        self.assertIsNone(generate_changelog.repo_http_url(self.repo.root))

    def test_breaking_change_flagged(self) -> None:
        self.repo.commit("feat(api)!: drop v1 endpoints")
        entries = generate_changelog.collect_entries(self.repo.root, None)
        self.assertTrue(entries[0].breaking)
        md = generate_changelog.render(entries, "Unreleased")
        self.assertIn("### ⚠️ Breaking Changes", md)
        self.assertIn("**(breaking)**", md)

    def test_since_excludes_tagged_history(self) -> None:
        self.repo.commit("feat: old feature")
        self.repo.tag("v1.0.0")
        self.repo.commit("fix: new fix")
        entries = generate_changelog.collect_entries(self.repo.root, "v1.0.0")
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0].description, "new fix")

    def test_latest_tag_detection(self) -> None:
        self.assertIsNone(generate_changelog.latest_tag(self.repo.root))
        self.repo.commit("feat: x")
        self.repo.tag("v2.0.0")
        self.assertEqual(generate_changelog.latest_tag(self.repo.root), "v2.0.0")


class TestMain(unittest.TestCase):
    def test_writes_changelog_file(self) -> None:
        repo = GitRepo()
        repo.commit("feat: wonderful thing")
        repo.commit("fix(tui): repaint glitch")
        rc = generate_changelog.main(["--repo", str(repo.root), "--title", "Unreleased"])
        self.assertEqual(rc, 0)
        content = (repo.root / "CHANGELOG.md").read_text(encoding="utf-8")
        self.assertIn("## Unreleased", content)
        self.assertIn("wonderful thing", content)
        self.assertIn("repaint glitch", content)

    def test_dry_run_writes_nothing(self) -> None:
        repo = GitRepo()
        repo.commit("feat: x")
        rc = generate_changelog.main(["--repo", str(repo.root), "--dry-run"])
        self.assertEqual(rc, 0)
        self.assertFalse((repo.root / "CHANGELOG.md").exists())


if __name__ == "__main__":
    unittest.main()
