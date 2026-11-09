#!/usr/bin/env python3

import json
import re
import unittest
from pathlib import Path

from postmerge_report import LABEL
from postmerge_report import Reporter

REPO = "CorbanuCore/CorbanuTerminal"
REPO_ROOT = Path(__file__).resolve().parents[2]
V8 = "v8-canary / Build release x86_64-unknown-linux-gnu"
V8_SKIPPED = "v8-canary / Build ${{ matrix.variant }} ${{ matrix.target }}"
TESTS = "rust-ci-full / Tests — ubuntu-24.04 - x86_64-unknown-linux-gnu / Tests shard 1/4"
RESULTS = "Postmerge CI results"


def jobs(*, v8: str | None, tests: str = "success") -> dict[str, str]:
    """A postmerge run's jobs; v8=None means the V8 matrix was skipped."""
    out = {"v8-canary / metadata": "success", TESTS: tests}
    if v8 is None:
        out[V8_SKIPPED] = "skipped"
    else:
        out[V8] = v8
    red = tests != "success" or v8 not in (None, "success")
    out[RESULTS] = "failure" if red else "success"
    out["Report main status"] = ""
    return out


def marker(run_id: int, created_at: str, failed: list[str]) -> str:
    data = {"run_id": run_id, "created_at": created_at, "failed_jobs": sorted(failed)}
    return f"<!-- postmerge-report {json.dumps(data, sort_keys=True)} -->"


class FakeGitHub:
    """Answers the gh calls the report makes, from an in-memory repository."""

    def __init__(self, head: str) -> None:
        self.head = head
        self.runs: dict[int, dict] = {}
        self.issues: list[dict] = []

    def add_run(self, run_id: int, sha: str, created_at: str, job_results: dict[str, str],
                status: str = "completed") -> None:
        self.runs[run_id] = {
            "id": run_id, "sha": sha, "created_at": created_at, "status": status,
            "jobs": job_results,
        }

    def add_issue(self, body: str) -> int:
        number = len(self.issues) + 1
        self.issues.append({"number": number, "open": True, "body": body, "comments": []})
        return number

    def open_numbers(self) -> list[int]:
        return [issue["number"] for issue in self.issues if issue["open"]]

    def issue(self, number: str) -> dict:
        return self.issues[int(number) - 1]

    def __call__(self, args: list[str]) -> str:
        flags = {args[i]: args[i + 1] for i in range(len(args) - 1) if args[i].startswith("--")}
        if args[:2] == ["label", "create"]:
            return ""
        if args[:2] == ["issue", "list"]:
            numbers = self.open_numbers()
            if flags.get("--jq") == ".[].number":
                return "".join(f"{n}\n" for n in numbers)
            return f"{numbers[0]}\n" if numbers else ""
        if args[:2] == ["issue", "view"]:
            issue = self.issue(args[2])
            comments = [{"body": body} for body in issue["comments"]]
            return json.dumps({"body": issue["body"], "comments": comments})
        if args[:2] == ["issue", "comment"]:
            self.issue(args[2])["comments"].append(flags["--body"])
            return ""
        if args[:2] == ["issue", "create"]:
            self.add_issue(flags["--body"])
            return ""
        if args[:2] == ["issue", "close"]:
            issue = self.issue(args[2])
            issue["open"] = False
            issue["comments"].append(flags["--comment"])
            return ""
        path = next(arg for arg in args[1:] if arg.startswith("repos/"))
        if path == f"repos/{REPO}/commits/main":
            return self.head + "\n"
        if match := re.fullmatch(rf"repos/{REPO}/actions/runs/(\d+)", path):
            run = self.runs[int(match.group(1))]
            return json.dumps({k: run[k] for k in ("id", "created_at", "status")})
        if match := re.fullmatch(rf"repos/{REPO}/actions/runs/(\d+)/jobs\?per_page=100", path):
            run = self.runs[int(match.group(1))]
            return "".join(
                json.dumps({"name": name, "conclusion": conclusion or None}) + "\n"
                for name, conclusion in run["jobs"].items()
            )
        if match := re.fullmatch(
            rf"repos/{REPO}/actions/workflows/postmerge-ci\.yml/runs\?branch=main&event=push"
            r"&per_page=100&created=%3E%3D(.+)",
            path,
        ):
            return "".join(
                json.dumps({k: run[k] for k in ("id", "created_at", "status")}) + "\n"
                for run in self.runs.values()
                if run["created_at"] >= match.group(1)
            )
        raise AssertionError(f"unexpected gh call: {args}")


A, B, C, Z = "a" * 40, "b" * 40, "c" * 40, "9" * 40
T0, T1, T2, T3 = (f"2026-11-08T1{n}:00:00Z" for n in range(4))


class Scenario:
    """Main was red at commit Z (run 10, V8 failed); issue #1 records it."""

    def __init__(self, head: str = B) -> None:
        self.gh = FakeGitHub(head)
        self.gh.add_run(10, Z, T0, jobs(v8="failure"))
        self.gh.add_issue(
            f"postmerge-ci failed on {Z[:10]}\n\n" + marker(10, T0, [RESULTS, V8])
        )

    def report(self, run_id: int, result: str) -> str:
        sha = self.gh.runs[run_id]["sha"]
        return Reporter(self.gh, REPO).report(run_id, sha, result, f"run-{run_id}")


class PostmergeReportTest(unittest.TestCase):
    def test_red_older_commit_finishing_after_a_green_head_opens_nothing(self) -> None:
        # #1697 item 1: commit A's multi-hour V8 run is red, but newer head B
        # ran V8 green and reported first.
        s = Scenario(head=B)
        s.gh.add_run(11, A, T1, jobs(v8="failure"), status="in_progress")
        s.gh.add_run(12, B, T2, jobs(v8="success"))
        self.assertEqual(s.report(12, "success"), "closed #1")
        s.gh.runs[11]["status"] = "completed"
        self.assertIn("stale", s.report(11, "failure"))
        self.assertEqual(s.gh.open_numbers(), [])
        self.assertEqual(len(s.gh.issues), 1)

    def test_green_head_that_skipped_v8_leaves_a_v8_failure_open(self) -> None:
        # #1697 item 1: a green commit that skipped the V8 matrix says nothing
        # about V8 at head.
        s = Scenario(head=B)
        s.gh.add_run(12, B, T2, jobs(v8=None))
        self.assertIn(V8, s.report(12, "success"))
        self.assertEqual(s.gh.open_numbers(), [1])

    def test_green_run_that_reran_the_failed_jobs_closes_the_issue(self) -> None:
        s = Scenario(head=B)
        s.gh.add_run(12, B, T2, jobs(v8="success"))
        self.assertEqual(s.report(12, "success"), "closed #1")
        self.assertEqual(s.gh.open_numbers(), [])

    def test_v8_fixed_on_an_older_green_closes_once_the_head_is_done(self) -> None:
        # A fixes V8 (slow run); B, the head, skips V8 and finishes first.
        s = Scenario(head=B)
        s.gh.add_run(11, A, T1, jobs(v8=""), status="in_progress")
        s.gh.add_run(12, B, T2, jobs(v8=None))
        self.assertIn("not passed again yet", s.report(12, "success"))
        s.gh.runs[11].update(status="completed", jobs=jobs(v8="success"))
        self.assertEqual(s.report(11, "success"), "closed #1")

    def test_green_run_waits_for_a_newer_run_still_in_progress(self) -> None:
        s = Scenario(head=C)
        s.gh.add_run(12, B, T2, jobs(v8="success"))
        s.gh.add_run(13, C, T3, jobs(v8=None, tests="failure"), status="in_progress")
        self.assertIn("still in progress", s.report(12, "success"))
        self.assertEqual(s.gh.open_numbers(), [1])
        s.gh.runs[13]["status"] = "completed"
        self.assertEqual(s.report(13, "failure"), "commented on #1")
        self.assertIn(TESTS, s.gh.issues[0]["comments"][-1])

    def test_red_run_records_its_failed_jobs(self) -> None:
        s = Scenario(head=A)
        s.gh.add_run(11, A, T1, jobs(v8=None, tests="failure"))
        self.assertEqual(s.report(11, "failure"), "commented on #1")
        comment = s.gh.issues[0]["comments"][-1]
        self.assertIn(f"postmerge-ci failed on {A[:10]}: run-11", comment)
        self.assertIn(marker(11, T1, [RESULTS, TESTS]), comment)

    def test_red_run_with_some_jobs_fixed_later_still_reports(self) -> None:
        s = Scenario(head=B)
        s.gh.add_run(11, A, T1, jobs(v8="failure", tests="failure"))
        s.gh.add_run(12, B, T2, jobs(v8=None))
        self.assertEqual(s.report(11, "failure"), "commented on #1")

    def test_red_with_no_issue_opens_one_and_green_head_closes_it(self) -> None:
        gh = FakeGitHub(head=B)
        gh.add_run(11, A, T1, jobs(v8=None, tests="failure"))
        reporter = Reporter(gh, REPO)
        self.assertEqual(reporter.report(11, A, "failure", "run-11"), "opened an issue")
        self.assertEqual(gh.open_numbers(), [1])
        self.assertIn(marker(11, T1, [RESULTS, TESTS]), gh.issues[0]["body"])
        gh.add_run(12, B, T2, jobs(v8=None))
        self.assertEqual(Reporter(gh, REPO).report(12, B, "success", "run-12"), "closed #1")

    def test_duplicate_issues_from_concurrent_reports_are_merged(self) -> None:
        # #1697 item 2: without the single-slot group two red reports can both
        # open an issue; the next report keeps the oldest.
        s = Scenario(head=A)
        s.gh.add_issue("postmerge-ci failed on " + marker(10, T0, [RESULTS, V8]))
        s.gh.add_run(11, A, T1, jobs(v8="failure"))
        self.assertEqual(s.report(11, "failure"), "commented on #1")
        self.assertEqual(s.gh.open_numbers(), [1])
        self.assertEqual(s.gh.issues[1]["comments"], ["Duplicate of #1."])

    def test_issue_without_markers_keeps_the_head_rule(self) -> None:
        gh = FakeGitHub(head=B)
        gh.add_issue("postmerge-ci failed on 9999999999: run-0")
        gh.add_run(11, A, T1, jobs(v8=None))
        gh.add_run(12, B, T2, jobs(v8=None))
        self.assertIn("older commit", Reporter(gh, REPO).report(11, A, "success", "run-11"))
        self.assertEqual(Reporter(gh, REPO).report(12, B, "success", "run-12"), "closed #1")


class PostmergeWorkflowTest(unittest.TestCase):
    def report_job(self) -> str:
        workflow = (REPO_ROOT / ".github" / "workflows" / "postmerge-ci.yml").read_text()
        match = re.search(r"(?ms)^  report:\n(.*?)(?=^  \S|\Z)", workflow)
        assert match, "postmerge-ci.yml has no report job"
        return match.group(1)

    def test_report_job_has_no_single_slot_concurrency_group(self) -> None:
        # #1697 item 2: a shared group keeps one pending run and cancels the rest.
        self.assertNotRegex(self.report_job(), r"(?m)^    concurrency:")

    def test_report_job_runs_the_tested_helper_with_run_read_access(self) -> None:
        job = self.report_job()
        self.assertIn("run: python3 .github/scripts/postmerge_report.py", job)
        self.assertRegex(job, r"(?m)^      actions: read$")
        self.assertIn("RUN_ID: ${{ github.run_id }}", job)
        self.assertIn(LABEL, (REPO_ROOT / ".github/scripts/postmerge_report.py").read_text())


if __name__ == "__main__":
    unittest.main()
