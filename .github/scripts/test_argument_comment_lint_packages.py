#!/usr/bin/env python3

import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from argument_comment_lint_packages import CANARY_PACKAGE
from argument_comment_lint_packages import changed_files
from argument_comment_lint_packages import lint_packages
from argument_comment_lint_packages import workspace_packages


ROOT = Path(__file__).resolve().parents[2]
PACKAGES = {
    "codex-rs/core": "codex-core",
    "codex-rs/core/tests/common": "core_test_support",
    "codex-rs/tui": "codex-tui",
}


class ArgumentCommentLintPackagesTest(unittest.TestCase):
    def test_changed_files_preserves_git_quoted_paths(self) -> None:
        paths = [
            f"codex-rs/core/tests/{name}.rs"
            for name in (
                "ordinary",
                "with space",
                'with"quote',
                "with\\slash",
                "with\ttab",
                "with\nnewline",
                "with\rcarriage",
                "café",
            )
        ]
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            self.run_git(root, "init")
            self.run_git(root, "config", "user.name", "Test User")
            self.run_git(root, "config", "user.email", "test@example.com")
            tree = self.run_git(root, "write-tree")
            base = self.run_git(root, "commit-tree", tree, "-m", "base")
            blob = self.run_git(root, "hash-object", "-w", "--stdin", input="fixture\n")
            # Build the index directly: these Git paths need not be valid host filenames.
            for path in paths:
                self.run_git(
                    root, "update-index", "--add", "--cacheinfo", "100644", blob, path
                )
            tree = self.run_git(root, "write-tree")
            head = self.run_git(root, "commit-tree", tree, "-p", base, "-m", "head")
            with mock.patch("argument_comment_lint_packages.ROOT", root):
                actual = changed_files(base, head)
                self.assertEqual(changed_files(head, head), [])
            self.assertCountEqual(actual, paths)
            for path in actual:
                with self.subTest(path=path):
                    self.assertEqual(lint_packages([path], PACKAGES), ["codex-core"])

    def run_git(self, root: Path, *args: str, input: str | None = None) -> str:
        return subprocess.run(
            ["git", *args],
            input=input,
            cwd=root,
            capture_output=True,
            check=True,
            text=True,
        ).stdout.strip()

    def test_changed_rust_sources_select_their_crates(self) -> None:
        self.assertEqual(
            lint_packages(
                [
                    "codex-rs/tui/src/app.rs",
                    "codex-rs/core/tests/suite/exec.rs",
                    "codex-rs/core/src/lib.rs",
                ],
                PACKAGES,
            ),
            ["codex-core", "codex-tui"],
        )

    def test_nested_crate_wins_over_its_parent(self) -> None:
        self.assertEqual(
            lint_packages(["codex-rs/core/tests/common/lib.rs"], PACKAGES),
            ["core_test_support"],
        )

    def test_non_rust_and_out_of_workspace_files_select_nothing(self) -> None:
        self.assertEqual(
            lint_packages(
                [
                    "codex-rs/core/Cargo.toml",
                    "codex-rs/core/README.md",
                    "codex-rs/Cargo.lock",
                    "docs/ci.md",
                    "scripts/tool.rs",
                ],
                PACKAGES,
            ),
            [],
        )

    def test_lint_job_changes_add_the_canary(self) -> None:
        for path in (
            ".github/workflows/rust-ci.yml",
            ".github/scripts/argument_comment_lint_packages.py",
            "tools/argument-comment-lint/src/lib.rs",
        ):
            with self.subTest(path=path):
                self.assertEqual(lint_packages([path], PACKAGES), [CANARY_PACKAGE])

    def test_real_workspace_maps_members_and_has_the_canary(self) -> None:
        packages = workspace_packages(ROOT)
        self.assertEqual(packages["codex-rs/core"], "codex-core")
        self.assertIn(CANARY_PACKAGE, packages.values())


if __name__ == "__main__":
    unittest.main()
