#!/usr/bin/env python3
"""Repo health gate for the Codex workspace.

Enforces the AGENTS.md module-size guidance with a ratchet so existing
debt cannot grow and no new debt can be introduced:

1. ``max-lines`` gate — any non-test Rust source file above the limit
   (default 800 lines, per AGENTS.md "if a file exceeds roughly 800 LoC,
   add new functionality in a new module") must already appear in the
   baseline snapshot with at least as many lines as it has today.
   - A NEW oversized file that is not in the baseline fails.
   - An existing oversized file that GREW past its baseline count fails.
   - Shrinking or deleting a baseline file always passes (and the
     baseline can be tightened afterwards with ``--update-baseline``).

2. ``--metrics`` — report-only dashboard (production ``unwrap()`` count,
   TODO/FIXME count, duplicate Cargo.lock dependency versions) so the
   trends tracked in IMPROVEMENT_SUGGESTIONS.md stay measurable.

Excluded from the gate: vendored code, generated OpenAPI models, test
files (``*_tests.rs``, ``tests.rs``, ``**/tests/**``) — matching the
AGENTS.md wording "target Rust modules under 500 LoC, excluding tests".

Usage:
    python3 scripts/check_repo_health.py                # gate check
    python3 scripts/check_repo_health.py --update-baseline
    python3 scripts/check_repo_health.py --metrics      # dashboard only
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

DEFAULT_MAX_LINES = 800

# Directories that are never counted: vendored or generated code.
EXCLUDED_DIR_PARTS = frozenset(
    {
        "vendor",  # vendored third-party Rust sources
        "target",  # build output
        "codex-backend-openapi-models",  # generated OpenAPI models
    }
)

# A path is treated as a test file if the file name matches or it lives
# under a `tests` directory (mirrors the exclusions AGENTS.md grants for
# the module-size guidance: "excluding tests").
_TEST_FILE_RE = re.compile(r"^(tests|(.*)_tests)\.rs$")

_TODO_RE = re.compile(r"\b(TODO|FIXME|HACK)\b")
_UNWRAP_RE = re.compile(r"\.unwrap\(\)")


@dataclass(frozen=True)
class Violation:
    """One gate failure: either a new or a grown oversized file."""

    path: str
    lines: int
    baseline_lines: int | None  # None => not in baseline (new violation)

    @property
    def reason(self) -> str:
        if self.baseline_lines is None:
            return (
                f"NEW oversized file ({self.lines} lines > "
                f"{DEFAULT_MAX_LINES} max) — not in baseline; split it per "
                "AGENTS.md or add it deliberately via --update-baseline"
            )
        return (
            f"oversized file GREW ({self.baseline_lines} -> {self.lines} "
            "lines, baseline exceeded); move new functionality to a new "
            "module per AGENTS.md"
        )


def is_test_file(path: Path) -> bool:
    """True for dedicated test files, which AGENTS.md exempts."""
    if _TEST_FILE_RE.match(path.name):
        return True
    return "tests" in path.parts


def is_excluded(path: Path) -> bool:
    """True for vendored/generated/hidden paths the gate ignores."""
    parts = path.parts
    return bool(EXCLUDED_DIR_PARTS.intersection(parts)) or any(
        part.startswith(".") for part in parts
    )


def iter_rust_files(root: Path):
    """Yield non-excluded, non-test ``*.rs`` files under ``root``."""
    for path in root.rglob("*.rs"):
        if path.is_file() and not is_excluded(path) and not is_test_file(path):
            yield path


def count_lines(path: Path) -> int:
    """Count physical lines the same way ``wc -l`` does (newline chars)."""
    with path.open("rb") as handle:
        return sum(chunk.count(b"\n") for chunk in iter(lambda: handle.read(1 << 20), b""))


def collect_oversized(root: Path, max_lines: int) -> dict[str, int]:
    """Return ``{relative_path: line_count}`` for files over the limit."""
    oversized: dict[str, int] = {}
    for path in iter_rust_files(root):
        lines = count_lines(path)
        if lines > max_lines:
            oversized[path.relative_to(root).as_posix()] = lines
    return dict(sorted(oversized.items(), key=lambda kv: (-kv[1], kv[0])))


def load_baseline(path: Path) -> dict[str, int]:
    if not path.exists():
        return {}
    data = json.loads(path.read_text(encoding="utf-8"))
    return {str(k): int(v) for k, v in data.get("files", {}).items()}


def load_todo_baseline(path: Path) -> int | None:
    """Read the ratcheted TODO count from the baseline, if recorded."""
    if not path.exists():
        return None
    data = json.loads(path.read_text(encoding="utf-8"))
    value = data.get("todo_count")
    return int(value) if value is not None else None


def write_baseline(path: Path, oversized: dict[str, int], todo_count: int) -> None:
    payload = {
        "comment": (
            "Ratchet snapshot of oversized Rust files and TODO/FIXME/HACK "
            "count. Regenerate with `python3 scripts/check_repo_health.py "
            "--update-baseline` ONLY to record deliberate additions or after "
            "shrinking/deleting files — never to make a failing check pass "
            "without review."
        ),
        "todo_count": todo_count,
        "files": oversized,
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def evaluate(current: dict[str, int], baseline: dict[str, int]) -> list[Violation]:
    """Fail on new oversized files and on files that grew past baseline."""
    violations: list[Violation] = []
    for path, lines in current.items():
        if path not in baseline:
            violations.append(Violation(path, lines, None))
        elif lines > baseline[path]:
            violations.append(Violation(path, lines, baseline[path]))
    return violations


_TODO_TAG_RE = re.compile(r"\bTODO\((\w+)\)")


def count_todos(root: Path) -> int:
    """Count TODO/FIXME/HACK mentions in production (non-test) sources."""
    total = 0
    for path in iter_rust_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        total += len(_TODO_RE.findall(text))
    return total


def todo_report(root: Path) -> dict[str, int]:
    """Group TODO mentions by owner tag (e.g. ``TODO(anp)`` -> ``anp``)."""
    groups: dict[str, int] = {}
    for path in iter_rust_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in re.finditer(r"\bTODO\b", text):
            tagged = _TODO_TAG_RE.search(text, match.start(), match.end() + 32)
            tag = tagged.group(1) if tagged else "(untagged)"
            groups[tag] = groups.get(tag, 0) + 1
    return dict(sorted(groups.items(), key=lambda kv: (-kv[1], kv[0])))


def print_todos(root: Path) -> None:
    groups = todo_report(root)
    total = sum(groups.values())
    print(f"Production TODO mentions: {total} (FIXME/HACK counted by the gate, not here)")
    for tag, count in groups.items():
        print(f"  {tag:14} {count}")


def print_metrics(root: Path) -> None:
    """Report-only health dashboard for trend tracking."""
    unwrap_count = 0
    file_count = 0
    for path in iter_rust_files(root):
        file_count += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        unwrap_count += len(_UNWRAP_RE.findall(text))

    duplicates: dict[str, int] = {}
    lock = root / "Cargo.lock"
    if lock.exists():
        names = re.findall(r'^name = "(.+)"$', lock.read_text(encoding="utf-8"), re.M)
        duplicates = {n: c for n, c in ((n, names.count(n)) for n in set(names)) if c > 1}

    print("Repo health metrics (report-only):")
    print(f"  non-test Rust files scanned : {file_count}")
    print(f"  production .unwrap()        : {unwrap_count}")
    print(f"  TODO/FIXME/HACK mentions    : {count_todos(root)}")
    print(f"  duplicated Cargo.lock deps  : {len(duplicates)}")
    for name, count in sorted(duplicates.items(), key=lambda kv: (-kv[1], kv[0]))[:10]:
        print(f"    {name} x{count}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "codex-rs",
        help="workspace root to scan (default: codex-rs next to this script)",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=Path(__file__).resolve().parent / "repo-health-baseline.json",
        help="ratchet baseline snapshot path",
    )
    parser.add_argument(
        "--max-lines",
        type=int,
        default=DEFAULT_MAX_LINES,
        help=f"line limit before a file counts as oversized (default {DEFAULT_MAX_LINES})",
    )
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="regenerate the baseline from the current tree instead of gating",
    )
    parser.add_argument(
        "--metrics",
        action="store_true",
        help="print the report-only health dashboard instead of gating",
    )
    parser.add_argument(
        "--todos",
        action="store_true",
        help="print TODO mentions grouped by owner tag instead of gating",
    )
    args = parser.parse_args(argv)

    root = args.root.resolve()
    if not root.is_dir():
        print(f"error: workspace root not found: {root}", file=sys.stderr)
        return 2

    if args.metrics:
        print_metrics(root)
        return 0

    if args.todos:
        print_todos(root)
        return 0

    current = collect_oversized(root, args.max_lines)
    todos_now = count_todos(root)

    if args.update_baseline:
        write_baseline(args.baseline, current, todos_now)
        print(
            f"Baseline updated: {len(current)} oversized file(s) and "
            f"{todos_now} TODO(s) recorded in {args.baseline}"
        )
        return 0

    baseline = load_baseline(args.baseline)
    violations = evaluate(current, baseline)
    todo_baseline = load_todo_baseline(args.baseline)
    todo_regression = todo_baseline is not None and todos_now > todo_baseline

    stale = sorted(set(baseline) - set(current))
    improved = sorted(p for p in set(baseline) & set(current) if current[p] < baseline[p])

    failed = bool(violations) or todo_regression
    if violations:
        print(
            f"FAIL: {len(violations)} new-or-worse oversized file(s) "
            f"(limit {args.max_lines} lines, tests/vendor/generated excluded):"
        )
        for v in violations:
            print(f"  {v.path}: {v.reason}")
    if todo_regression:
        print(
            f"FAIL: TODO/FIXME/HACK count regressed ({todo_baseline} -> "
            f"{todos_now}); resolve or split existing TODOs instead of adding "
            "new ones, then ratchet the baseline down via --update-baseline"
        )
    if failed:
        return 1

    print(
        f"OK: gate passed. {len(current)} known oversized file(s) within "
        f"baseline (limit {args.max_lines} lines); {todos_now} TODO(s) "
        f"(baseline {todo_baseline})."
    )
    if stale:
        print(f"  {len(stale)} baseline file(s) now fixed — consider `--update-baseline`:")
        for path in stale[:10]:
            print(f"    {path}")
        if len(stale) > 10:
            print(f"    ... and {len(stale) - 10} more")
    if improved:
        print(f"  {len(improved)} baseline file(s) shrank — consider `--update-baseline`.")
    if todo_baseline is not None and todos_now < todo_baseline:
        print(
            f"  TODO count improved ({todo_baseline} -> {todos_now}) — "
            "ratchet it down with `--update-baseline`."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
