#!/usr/bin/env python3
"""Generate a searchable CHANGELOG.md from conventional-commit git history.

Replaces the one-line pointer in CHANGELOG.md with a real, offline-browsable
changelog (suggestion #9 in IMPROVEMENT_SUGGESTIONS.md). No external tools
or network access required — plain git + Python.

Commit message formats recognized:

    <type>[optional scope][!]: <description>     (Conventional Commits 1.0.0)
    <description> (#1234)                        (GitHub squash-merge subject)

Conventional types map to changelog sections:

    feat -> Features           fix -> Bug Fixes
    perf -> Performance        refactor -> Refactoring
    docs -> Documentation      everything else -> Other Changes

GitHub-style subjects land in "Other Changes" with a PR link when the
origin remote is parseable. Merge commits are skipped entirely.

Usage:
    python3 scripts/generate_changelog.py                    # write CHANGELOG.md
    python3 scripts/generate_changelog.py --stdout           # print only
    python3 scripts/generate_changelog.py --since v1.2.0     # history range
    python3 scripts/generate_changelog.py --dry-run          # no file writes
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

SECTION_ORDER = [
    ("feat", "Features"),
    ("fix", "Bug Fixes"),
    ("perf", "Performance"),
    ("refactor", "Refactoring"),
    ("docs", "Documentation"),
]
SECTION_BY_TYPE = dict(SECTION_ORDER)
FALLBACK_SECTION = "Other Changes"

_HEADER = """# Changelog

All notable changes to this project are documented here, generated from
conventional-commit history by `python3 scripts/generate_changelog.py`.
Release notes with binaries live on the
[GitHub releases page](https://github.com/openai/codex/releases).

"""

_COMMIT_RE = re.compile(
    r"^(?P<type>[a-zA-Z]+)(?:\((?P<scope>[^)]+)\))?(?P<breaking>!)?:\s+(?P<desc>.+)$"
)


@dataclass(frozen=True)
class Entry:
    type: str
    scope: str | None
    breaking: bool
    description: str
    short_hash: str
    pr_number: int | None = None


def run_git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout


_PR_SUFFIX_RE = re.compile(r"^(?P<desc>.+?)\s+\(#(?P<pr>\d+)\)$")


def repo_http_url(repo: Path) -> str | None:
    """Return an https:// URL for the origin remote, if parseable."""
    result = subprocess.run(
        ["git", "-C", str(repo), "remote", "get-url", "origin"],
        capture_output=True,
        text=True,
    )
    url = result.stdout.strip() if result.returncode == 0 else ""
    for pattern, repl in (
        (r"^git@([^:]+):(.+?)(?:\.git)?$", r"https://\1/\2"),
        (r"^https://(.+?)(?:\.git)?$", r"https://\1"),
        (r"^ssh://git@([^/]+)/(.+?)(?:\.git)?$", r"https://\1/\2"),
    ):
        matched = re.match(pattern, url)
        if matched:
            return matched.expand(repl)
    return None


def latest_tag(repo: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(repo), "describe", "--abbrev=0", "--tags"],
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def collect_entries(repo: Path, since: str | None) -> list[Entry]:
    """Return changelog-worthy commits, newest first."""
    log_format = "%h%x00%s"
    args = ["log", "--no-merges", f"--format={log_format}"]
    if since:
        args.append(f"{since}..HEAD")
    entries: list[Entry] = []
    for line in run_git(repo, *args).splitlines():
        if "\x00" not in line:
            continue
        short_hash, subject = line.split("\x00", 1)
        subject = subject.strip()
        if not subject:
            continue
        pr_number = None
        pr_match = _PR_SUFFIX_RE.match(subject)
        if pr_match:
            subject = pr_match.group("desc")
            pr_number = int(pr_match.group("pr"))
        match = _COMMIT_RE.match(subject)
        if match:
            entries.append(
                Entry(
                    type=match.group("type").lower(),
                    scope=match.group("scope"),
                    breaking=bool(match.group("breaking")),
                    description=match.group("desc").strip(),
                    short_hash=short_hash,
                    pr_number=pr_number,
                )
            )
            continue
        # GitHub squash-merge style (or any other subject): keep it.
        entries.append(
            Entry(
                type="other",
                scope=None,
                breaking=False,
                description=subject,
                short_hash=short_hash,
                pr_number=pr_number,
            )
        )
    return entries


def render(entries: list[Entry], title: str, base_url: str | None = None) -> str:
    def ref(e: Entry) -> str:
        if e.pr_number and base_url:
            return f"([#{e.pr_number}]({base_url}/pull/{e.pr_number}))"
        if e.pr_number:
            return f"(#{e.pr_number})"
        return f"({e.short_hash})"

    lines = [_HEADER, f"## {title}", ""]
    if not entries:
        lines.append("_No commits in this range._")
        lines.append("")
        return "\n".join(lines)

    breaking = [e for e in entries if e.breaking]
    if breaking:
        lines.append("### ⚠️ Breaking Changes")
        lines.append("")
        for e in breaking:
            lines.append(f"- {e.description} {ref(e)}")
        lines.append("")

    by_section: dict[str, list[Entry]] = {}
    for e in entries:
        section = SECTION_BY_TYPE.get(e.type, FALLBACK_SECTION)
        by_section.setdefault(section, []).append(e)

    ordered = [s for _, s in SECTION_ORDER if s in by_section]
    if FALLBACK_SECTION in by_section:
        ordered.append(FALLBACK_SECTION)
    for section in ordered:
        lines.append(f"### {section}")
        lines.append("")
        for e in by_section[section]:
            scope = f"**{e.scope}**: " if e.scope else ""
            marker = " **(breaking)**" if e.breaking else ""
            lines.append(f"- {scope}{e.description}{marker} {ref(e)}")
        lines.append("")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--output", type=Path, default=None, help="default: <repo>/CHANGELOG.md")
    parser.add_argument("--since", default=None, help="git rev/tag to start from (default: latest tag)")
    parser.add_argument("--title", default=None, help="section heading (default: tag or 'Unreleased')")
    parser.add_argument("--stdout", action="store_true", help="print instead of writing the file")
    parser.add_argument("--dry-run", action="store_true", help="print what would be written, change nothing")
    args = parser.parse_args(argv)

    repo = args.repo.resolve()
    since = args.since if args.since is not None else latest_tag(repo)
    title = args.title or (since or "Unreleased")

    entries = collect_entries(repo, since)
    conventional = sum(1 for e in entries if e.type != "other")
    base_url = repo_http_url(repo)
    markdown = render(entries, title, base_url)

    if args.stdout or args.dry_run:
        print(markdown, end="")
        print(
            f"[generate_changelog] {len(entries)} commits since '{since}' "
            f"({conventional} conventional, {len(entries) - conventional} other).",
            file=sys.stderr,
        )
        return 0

    output = args.output or repo / "CHANGELOG.md"
    output.write_text(markdown, encoding="utf-8")
    print(f"[generate_changelog] wrote {len(entries)} entries to {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
