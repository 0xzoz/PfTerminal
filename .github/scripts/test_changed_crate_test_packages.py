#!/usr/bin/env python3

import re
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


class ChangedCrateTestsWorkflowTest(unittest.TestCase):
    """#1972: PRs run their changed crates' tests on Linux, as a required job."""

    def workflow(self, name: str) -> str:
        return (ROOT / ".github" / "workflows" / name).read_text()

    def test_rust_ci_runs_and_requires_the_job(self) -> None:
        rust_ci = self.workflow("rust-ci.yml")
        self.assertIn(
            'python3 .github/scripts/changed_crate_test_packages.py "$BASE_SHA" "$HEAD_SHA"',
            rust_ci,
        )
        self.assertRegex(
            rust_ci,
            r"(?ms)^  changed_crate_tests:\n.*?"
            r"if: \$\{\{ needs\.changed\.outputs\.changed_crate_test_packages != '' \}\}\n"
            r"    uses: \./\.github/workflows/rust-ci-changed-crate-tests\.yml\n",
        )
        self.assertRegex(
            rust_ci, r"(?ms)^  results:\n.*?^        changed_crate_tests,$"
        )
        self.assertIn(
            "[[ '${{ needs.changed_crate_tests.result }}' == 'success' ]]", rust_ci
        )
        self.assertIn(
            "NEEDS_CHANGED_OUTPUTS_CHANGED_CRATE_TEST_PACKAGES: "
            "${{ needs.changed.outputs.changed_crate_test_packages }}",
            rust_ci,
        )

    def test_job_runs_nextest_within_budget_and_saves_no_cache(self) -> None:
        job = self.workflow("rust-ci-changed-crate-tests.yml")
        self.assertIn("timeout-minutes: 75", job)
        self.assertRegex(
            job, r"cargo nextest run --locked .*--no-fail-fast --no-tests=pass"
        )
        self.assertIn("actions/cache/restore@", job)
        self.assertNotIn("actions/cache/save@", job)
        self.assertNotRegex(job, r"uses: actions/cache@")

    def test_job_restores_the_core_security_linux_sccache(self) -> None:
        def keys(text: str) -> list[str]:
            return re.findall(r"(?m)^ +key: (sccache-core-security-.+)$", text)

        saved = keys(self.workflow("rust-ci-core-security.yml"))[0]
        restored = keys(self.workflow("rust-ci-changed-crate-tests.yml"))[0]
        self.assertEqual(
            restored,
            saved.replace("${{ inputs.runner }}", "ubuntu-24.04").replace(
                "${{ inputs.target }}", "${{ env.TARGET }}"
            ),
        )


if __name__ == "__main__":
    unittest.main()
