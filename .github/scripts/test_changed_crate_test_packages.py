#!/usr/bin/env python3

import unittest

from changed_crate_test_packages import CANARY_PACKAGE
from changed_crate_test_packages import EXCLUDED_PACKAGES
from changed_crate_test_packages import HELPER_BINARIES
from changed_crate_test_packages import ROOT
from changed_crate_test_packages import helper_build_args
from changed_crate_test_packages import test_packages
from changed_crate_test_packages import workspace_packages

PACKAGES = {
    "codex-rs/core": "codex-core",
    "codex-rs/core/tests/common": "core_test_support",
    "codex-rs/rmcp-client": "codex-rmcp-client",
    "codex-rs/utils/string": "codex-utils-string",
}


class ChangedCrateTestPackagesTest(unittest.TestCase):
    def test_any_file_under_a_crate_selects_it(self) -> None:
        self.assertEqual(
            test_packages(
                [
                    "codex-rs/rmcp-client/tests/fixtures/response.json",
                    "codex-rs/utils/string/Cargo.toml",
                    "codex-rs/Cargo.lock",
                    "docs/ci.md",
                ],
                PACKAGES,
            ),
            ["codex-rmcp-client", "codex-utils-string"],
        )

    def test_nested_package_wins_and_excluded_packages_are_dropped(self) -> None:
        self.assertEqual(
            test_packages(
                ["codex-rs/core/src/lib.rs", "codex-rs/core/tests/common/lib.rs"],
                PACKAGES,
            ),
            ["core_test_support"],
        )

    def test_job_changes_select_the_canary(self) -> None:
        self.assertEqual(
            test_packages(
                [".github/workflows/rust-ci-changed-crate-tests.yml"], PACKAGES
            ),
            [CANARY_PACKAGE],
        )

    def test_helpers(self) -> None:
        self.assertEqual(
            helper_build_args(["codex-utils-string", "codex-rmcp-client"]),
            ["-p", "codex-cli", "--bin", "codex"],
        )
        self.assertEqual(helper_build_args(["codex-utils-string"]), [])

    def test_named_packages_exist(self) -> None:
        names = set(workspace_packages(ROOT).values())
        named = {CANARY_PACKAGE} | EXCLUDED_PACKAGES | set(HELPER_BINARIES)
        named |= {
            package for helpers in HELPER_BINARIES.values() for package, _ in helpers
        }
        self.assertEqual(named - names, set())


if __name__ == "__main__":
    unittest.main()
