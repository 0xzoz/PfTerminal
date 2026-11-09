#!/usr/bin/env python3
"""#1856: codex-core's and codex-app-server's integration tests run on native
Windows in a non-blocking PR job, minus the known failures listed in
.github/windows-integration-known-failures.txt."""

import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import windows_integration_known_failures as known  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
REUSABLE = "rust-ci-windows-integration-tests.yml"
PACKAGE_DIRS = {"codex-core": "core", "codex-app-server": "app-server"}


def job(text: str, name: str) -> str:
    match = re.search(rf"(?ms)^  {re.escape(name)}:\n.*?(?=^  \S|\Z)", text)
    assert match is not None, name
    return match.group(0)


class KnownFailureListTest(unittest.TestCase):
    def test_every_entry_names_an_existing_test_and_an_issue(self) -> None:
        entries = known.load()
        self.assertTrue(entries)
        self.assertEqual(len(entries), len(set(entries)))
        for entry in entries:
            with self.subTest(test=entry.test):
                self.assertIn(entry.package, PACKAGE_DIRS)
                *modules, function = entry.test.split("::")
                source = (
                    REPO_ROOT / "codex-rs" / PACKAGE_DIRS[entry.package] / "tests"
                ).joinpath(*modules[:-1], modules[-1] + ".rs")
                self.assertTrue(source.is_file(), source)
                self.assertRegex(
                    source.read_text(encoding="utf-8"),
                    rf"(?m)^\s*(pub )?(async )?fn {re.escape(function)}\(",
                )

    def test_malformed_lines_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "list.txt")
            for line in (
                "codex-core suite::a::b",
                "codex-core suite::a::b 1967",
                "codex-core suite::a::b) | all( #1967",
            ):
                with self.subTest(line=line):
                    path.write_text(f"# comment\n\n{line}\n", encoding="utf-8")
                    with self.assertRaises(ValueError):
                        known.load(path)

    def test_filter_excludes_only_the_packages_entries(self) -> None:
        entries = [
            known.KnownFailure("codex-core", "suite::a::one", "1"),
            known.KnownFailure("codex-core", "suite::b::two", "2"),
            known.KnownFailure("codex-app-server", "suite::v2::c::three", "3"),
        ]
        self.assertEqual(
            known.nextest_filter(entries, "codex-core"),
            "not (test(=suite::a::one) | test(=suite::b::two))",
        )
        self.assertEqual(
            known.nextest_filter(entries, "codex-app-server"),
            "not (test(=suite::v2::c::three))",
        )
        self.assertEqual(known.nextest_filter(entries, "codex-cli"), "all()")

    def test_summary_names_every_excluded_test_and_its_issue(self) -> None:
        entries = known.load()
        for package in PACKAGE_DIRS:
            text = known.summary(entries, package)
            self.assertIn("Non-blocking", text)
            for entry in entries:
                line = f"- `{entry.test}` (#{entry.issue})"
                if entry.package == package:
                    self.assertIn(line, text)
                else:
                    self.assertNotIn(line, text)

    def test_cli(self) -> None:
        script = Path(known.__file__)
        run = lambda *args: subprocess.run(
            [sys.executable, str(script), *args], capture_output=True, text=True
        )
        result = run("filter", "codex-core")
        self.assertEqual(result.returncode, 0)
        self.assertTrue(result.stdout.startswith("not (test(=suite::"))
        self.assertEqual(run("bogus", "codex-core").returncode, 2)


class WorkflowTest(unittest.TestCase):
    def test_reusable_workflow_runs_integration_tests_with_the_filter(self) -> None:
        reusable = (WORKFLOWS / REUSABLE).read_text()
        triggers = reusable.split("\njobs:")[0]
        self.assertRegex(triggers, r"(?m)^  workflow_call:$")
        self.assertNotRegex(triggers, r"(?m)^  (pull_request|push|schedule):")
        self.assertIn("runs-on: windows-2022", reusable)
        self.assertIn("timeout-minutes: 60", reusable)
        self.assertIn('default: \'["codex-core", "codex-app-server"]\'', reusable)
        self.assertIn(
            "cargo nextest run --cargo-profile ci-test -p \"$PACKAGE\" --test '*'", reusable
        )
        self.assertIn('-E "$filter"', reusable)
        self.assertIn('filter="$(python3 "$known" filter "$PACKAGE")"', reusable)
        self.assertIn('python3 "$known" summary "$PACKAGE" >> "$GITHUB_STEP_SUMMARY"', reusable)
        # Restore-only.
        self.assertNotIn("actions/cache/save", reusable)
        self.assertNotRegex(reusable, r"uses: actions/cache@")

    def test_cargo_home_cache_paths_match_the_windows_clippy_leg(self) -> None:
        # actions/cache versions a cache by its path list.
        def paths(text: str) -> str:
            match = re.search(
                r"(?ms)- name: Restore cargo home cache\n.*?path: \|\n(.*?)\n\s+key:", text
            )
            self.assertIsNotNone(match)
            return "\n".join(line.strip() for line in match.group(1).splitlines())

        self.assertEqual(
            paths((WORKFLOWS / REUSABLE).read_text()),
            paths((WORKFLOWS / "rust-ci-lint-build.yml").read_text()),
        )

    def test_pr_job_calls_it_and_is_not_required(self) -> None:
        rust_ci = (WORKFLOWS / "rust-ci.yml").read_text()
        self.assertRegex(
            job(rust_ci, "windows_integration_tests"),
            r"(?m)^    name: Windows integration tests \(non-blocking\)\n"
            r"    needs: changed\n"
            r"    if: \$\{\{ needs\.changed\.outputs\.windows_integration_tests == 'true' \}\}\n"
            rf"    uses: \./\.github/workflows/{re.escape(REUSABLE)}\n"
            r"    with:\n"
            r"      packages: \$\{\{ needs\.changed\.outputs\.windows_integration_packages \}\}\n",
        )
        self.assertNotIn("windows_integration_tests", job(rust_ci, "results"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
