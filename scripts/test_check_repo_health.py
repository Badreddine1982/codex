"""Unit tests for scripts/check_repo_health.py.

Run with:  python3 -m unittest discover -s scripts -p 'test_check_repo_health.py'
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_repo_health", Path(__file__).with_name("check_repo_health.py")
)
check_repo_health = importlib.util.module_from_spec(_SPEC)
# Register before exec so dataclasses can resolve the module namespace.
sys.modules["check_repo_health"] = check_repo_health
_SPEC.loader.exec_module(check_repo_health)  # type: ignore[union-attr]


class WorkspaceFixture:
    """Temporary workspace tree factory used by the tests below."""

    def __init__(self, name: str = "ws") -> None:
        self.root = Path(tempfile.mkdtemp(prefix=f"repo-health-{name}-"))

    def add(self, rel: str, lines: int) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        # `lines` newline characters == `wc -l` line count.
        path.write_text("\n" * lines, encoding="utf-8")
        return path

    def baseline(self, entries: dict[str, int]) -> Path:
        import json

        path = self.root / "baseline.json"
        path.write_text(json.dumps({"files": entries}), encoding="utf-8")
        return path


class TestExclusions(unittest.TestCase):
    def test_test_files_are_exempt(self) -> None:
        self.assertTrue(check_repo_health.is_test_file(Path("core/src/session/tests.rs")))
        self.assertTrue(check_repo_health.is_test_file(Path("core/src/client_tests.rs")))
        self.assertTrue(check_repo_health.is_test_file(Path("core/tests/all.rs")))
        self.assertFalse(check_repo_health.is_test_file(Path("core/src/client.rs")))

    def test_vendor_and_generated_are_exempt(self) -> None:
        self.assertTrue(check_repo_health.is_excluded(Path("vendor/foo/src/lib.rs")))
        self.assertTrue(
            check_repo_health.is_excluded(
                Path("codex-backend-openapi-models/src/models/mod.rs")
            )
        )
        self.assertFalse(check_repo_health.is_excluded(Path("tui/src/app.rs")))


class TestGate(unittest.TestCase):
    def setUp(self) -> None:
        self.ws = WorkspaceFixture("gate")

    def test_clean_tree_passes(self) -> None:
        self.ws.add("tui/src/app.rs", 100)
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.ws.root / "b.json")]
        )
        self.assertEqual(rc, 0)

    def test_new_oversized_file_fails(self) -> None:
        self.ws.add("tui/src/huge.rs", 900)
        baseline = self.ws.baseline({})
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 1)

    def test_oversized_file_at_baseline_passes(self) -> None:
        self.ws.add("tui/src/huge.rs", 900)
        baseline = self.ws.baseline({"tui/src/huge.rs": 900})
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 0)

    def test_oversized_file_growth_fails(self) -> None:
        self.ws.add("tui/src/huge.rs", 950)
        baseline = self.ws.baseline({"tui/src/huge.rs": 900})
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 1)

    def test_shrinking_file_passes(self) -> None:
        self.ws.add("tui/src/huge.rs", 400)
        baseline = self.ws.baseline({"tui/src/huge.rs": 900})
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 0)

    def test_test_files_do_not_trip_gate(self) -> None:
        self.ws.add("core/src/session/tests.rs", 11_000)
        self.ws.add("core/src/client_tests.rs", 5_000)
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.ws.root / "b.json")]
        )
        self.assertEqual(rc, 0)

    def test_baseline_update_roundtrip(self) -> None:
        self.ws.add("tui/src/huge.rs", 900)
        baseline = self.ws.root / "b.json"
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline), "--update-baseline"]
        )
        self.assertEqual(rc, 0)
        recorded = check_repo_health.load_baseline(baseline)
        self.assertEqual(recorded, {"tui/src/huge.rs": 900})
        # After snapshotting, the same tree must gate cleanly.
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 0)


class TestTodoRatchet(unittest.TestCase):
    def setUp(self) -> None:
        self.ws = WorkspaceFixture("todos")

    def baseline_with(self, count: int) -> Path:
        import json

        path = self.ws.root / "b.json"
        path.write_text(json.dumps({"todo_count": count, "files": {}}), encoding="utf-8")
        return path

    def test_todo_count_at_baseline_passes(self) -> None:
        path = self.ws.add("core/src/lib.rs", 10)
        path.write_text("// TODO(anp): one\n// FIXME: two\n", encoding="utf-8")
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.baseline_with(2))]
        )
        self.assertEqual(rc, 0)

    def test_new_todo_fails(self) -> None:
        path = self.ws.add("core/src/lib.rs", 10)
        path.write_text("// TODO(anp): one\n// TODO: two\n// HACK: three\n", encoding="utf-8")
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.baseline_with(2))]
        )
        self.assertEqual(rc, 1)

    def test_fewer_todos_passes(self) -> None:
        path = self.ws.add("core/src/lib.rs", 10)
        path.write_text("// TODO(anp): one\n", encoding="utf-8")
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.baseline_with(2))]
        )
        self.assertEqual(rc, 0)

    def test_todos_in_test_files_ignored(self) -> None:
        self.ws.add("core/src/client_tests.rs", 10).write_text(
            "// TODO: a\n// TODO: b\n// TODO: c\n", encoding="utf-8"
        )
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(self.baseline_with(0))]
        )
        self.assertEqual(rc, 0)

    def test_baseline_without_todo_count_skips_todo_gate(self) -> None:
        # Old-format baseline (files only) must not break or fail the gate.
        self.ws.add("tui/src/app.rs", 10).write_text("// TODO: x\n", encoding="utf-8")
        baseline = self.ws.baseline({})
        rc = check_repo_health.main(
            ["--root", str(self.ws.root), "--baseline", str(baseline)]
        )
        self.assertEqual(rc, 0)


class TestTodoReport(unittest.TestCase):
    def test_grouping_by_owner_tag(self) -> None:
        ws = WorkspaceFixture("report")
        ws.add("core/src/a.rs", 10).write_text(
            "// TODO(anp): 1\n// TODO(anp): 2\n// TODO(jif): 3\n// TODO: 4\n",
            encoding="utf-8",
        )
        groups = check_repo_health.todo_report(ws.root)
        self.assertEqual(groups["anp"], 2)
        self.assertEqual(groups["jif"], 1)
        self.assertEqual(groups["(untagged)"], 1)


if __name__ == "__main__":
    unittest.main()
