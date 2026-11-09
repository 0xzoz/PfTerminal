#!/usr/bin/env python3
"""#1856: the nextest filter and job summary for one package's known Windows
integration-test failures (.github/windows-integration-known-failures.txt).

Usage: windows_integration_known_failures.py filter|summary <cargo package>
"""

import re
import sys
from pathlib import Path
from typing import NamedTuple

LIST = Path(__file__).resolve().parents[1] / "windows-integration-known-failures.txt"
LINE = re.compile(r"^(?P<package>[a-z0-9-]+) (?P<test>[A-Za-z0-9_:]+) #(?P<issue>[0-9]+)$")


class KnownFailure(NamedTuple):
    package: str
    test: str
    issue: str


def load(path: Path = LIST) -> list[KnownFailure]:
    entries = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        match = LINE.match(line)
        if match is None:
            raise ValueError(f"{path.name}:{number}: expected '<package> <test> #<issue>': {raw!r}")
        entries.append(KnownFailure(**match.groupdict()))
    return entries


def nextest_filter(entries: list[KnownFailure], package: str) -> str:
    """A nextest filterset that runs every test except the package's known failures."""
    tests = [entry.test for entry in entries if entry.package == package]
    if not tests:
        return "all()"
    return "not (" + " | ".join(f"test(={test})" for test in tests) + ")"


def summary(entries: list[KnownFailure], package: str) -> str:
    excluded = [entry for entry in entries if entry.package == package]
    lines = [
        f"### {package}: integration tests (`tests/*.rs`) on native Windows (#1856)",
        "",
        "Non-blocking: this job is not part of `CI results (required)`.",
    ]
    if excluded:
        lines += [
            "",
            f"{len(excluded)} known Windows failures were excluded "
            "(`.github/windows-integration-known-failures.txt`):",
            "",
        ]
        lines += [f"- `{entry.test}` (#{entry.issue})" for entry in excluded]
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 3 or argv[1] not in ("filter", "summary"):
        print(__doc__, file=sys.stderr)
        return 2
    entries = load()
    if argv[1] == "filter":
        print(nextest_filter(entries, argv[2]))
    else:
        sys.stdout.write(summary(entries, argv[2]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
