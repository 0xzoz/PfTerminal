#!/usr/bin/env python3

"""Keep one labelled issue open while postmerge-ci is red on main (#1697).

postmerge-ci runs overlap on main (each gets its own v8-canary group, #1696),
so a run's report can arrive after a newer commit's. Each red report records
its run and failed jobs in a hidden marker on the issue. A failure counts as
fixed once a later main run (by creation time, which follows push order) ran
that same job and it passed. A job that a later run skipped, such as the V8
matrix on a commit that doesn't touch V8, proves nothing. So:

- a red run whose failed jobs all passed in a later run is stale: no comment,
  no new issue;
- a green run closes the issue only when every recorded failure has been
  fixed that way and no newer main run is still in progress (that run's
  report decides instead).

The report job has no concurrency group (a single-slot group cancelled
pending reports); if two red reports open an issue at once, the newer
duplicate is closed.
"""

import json
import os
import re
import subprocess
from dataclasses import dataclass
from typing import Callable

LABEL = "postmerge-red"
TITLE = "postmerge-ci is failing on main"
WORKFLOW = "postmerge-ci.yml"
REPORT_JOB = "Report main status"
RESULTS_JOB = "Postmerge CI results"
FAILED = ("failure", "timed_out")
NOTE = (
    "PR checks do not run postmerge-ci (full Linux tests, musl and Windows "
    "clippy). This issue closes itself once every failed job has passed again "
    "in a later main run."
)
MARKER = re.compile(r"<!-- postmerge-report (\{[^\n]*\}) -->")

Gh = Callable[[list[str]], str]


def run_gh(args: list[str]) -> str:
    return subprocess.run(
        ["gh", *args], check=True, capture_output=True, text=True
    ).stdout


@dataclass(frozen=True)
class Run:
    id: int
    created_at: str
    status: str

    def key(self) -> tuple[str, int]:
        return (self.created_at, self.id)


@dataclass(frozen=True)
class Record:
    run_id: int
    created_at: str
    failed_jobs: tuple[str, ...]


def json_lines(text: str) -> list[dict]:
    return [json.loads(line) for line in text.splitlines() if line.strip()]


class Reporter:
    def __init__(self, gh: Gh, repo: str) -> None:
        self.gh = gh
        self.repo = repo
        self.jobs_cache: dict[int, dict[str, str]] = {}

    def run(self, run_id: int) -> Run:
        data = json.loads(self.gh(["api", f"repos/{self.repo}/actions/runs/{run_id}"]))
        return Run(data["id"], data["created_at"], data["status"])

    def runs_since(self, created_at: str) -> list[Run]:
        query = f"branch=main&event=push&per_page=100&created=%3E%3D{created_at}"
        out = self.gh(
            [
                "api",
                "--paginate",
                f"repos/{self.repo}/actions/workflows/{WORKFLOW}/runs?{query}",
                "--jq",
                ".workflow_runs[] | {id, created_at, status}",
            ]
        )
        return [Run(r["id"], r["created_at"], r["status"]) for r in json_lines(out)]

    def jobs(self, run_id: int) -> dict[str, str]:
        if run_id not in self.jobs_cache:
            out = self.gh(
                [
                    "api",
                    "--paginate",
                    f"repos/{self.repo}/actions/runs/{run_id}/jobs?per_page=100",
                    "--jq",
                    ".jobs[] | {name, conclusion}",
                ]
            )
            self.jobs_cache[run_id] = {
                job["name"]: job["conclusion"] or "" for job in json_lines(out)
            }
        return self.jobs_cache[run_id]

    def open_issues(self) -> list[int]:
        out = self.gh(
            [
                "issue",
                "list",
                "--state",
                "open",
                "--label",
                LABEL,
                "--json",
                "number",
                "--jq",
                ".[].number",
            ]
        )
        return sorted(int(line) for line in out.split())

    def records(self, number: int) -> list[Record]:
        data = json.loads(self.gh(["issue", "view", str(number), "--json", "body,comments"]))
        texts = [data.get("body") or ""] + [c.get("body") or "" for c in data.get("comments", [])]
        records = []
        for text in texts:
            for match in MARKER.finditer(text):
                raw = json.loads(match.group(1))
                records.append(
                    Record(raw["run_id"], raw["created_at"], tuple(raw["failed_jobs"]))
                )
        return records

    def fixed_later(self, created_at: str, run_id: int, job: str, runs: list[Run]) -> bool:
        after = (created_at, run_id)
        return any(
            run.key() > after and self.jobs(run.id).get(job) == "success" for run in runs
        )

    def dedupe(self, issues: list[int]) -> int | None:
        if not issues:
            return None
        keep, *duplicates = issues
        for number in duplicates:
            self.gh(["issue", "close", str(number), "--comment", f"Duplicate of #{keep}."])
        return keep

    def report(self, run_id: int, sha: str, result: str, run_url: str) -> str:
        try:
            self.gh(["label", "create", LABEL, "--color", "B60205", "--description", TITLE])
        except subprocess.CalledProcessError:
            pass  # The label already exists.
        me = self.run(run_id)
        issue = self.dedupe(self.open_issues())

        if result == "failure":
            failed = sorted(
                name
                for name, conclusion in self.jobs(run_id).items()
                if conclusion in FAILED and name != REPORT_JOB
            ) or [RESULTS_JOB]
            later = [run for run in self.runs_since(me.created_at) if run.id != run_id]
            still_red = [
                job for job in failed if not self.fixed_later(me.created_at, run_id, job, later)
            ]
            if not still_red:
                return f"stale: every failed job of {sha[:10]} passed in a later main run"
            marker = json.dumps(
                {"run_id": run_id, "created_at": me.created_at, "failed_jobs": failed},
                sort_keys=True,
            )
            jobs = "\n".join(f"- {job}" for job in failed)
            body = (
                f"postmerge-ci failed on {sha[:10]}: {run_url}\n\nFailed jobs:\n{jobs}\n\n"
                f"<!-- postmerge-report {marker} -->"
            )
            if issue is not None:
                self.gh(["issue", "comment", str(issue), "--body", body])
                return f"commented on #{issue}"
            create = ["issue", "create", "--title", TITLE, "--label", LABEL]
            self.gh([*create, "--body", f"{body}\n\n{NOTE}"])
            self.dedupe(self.open_issues())
            return "opened an issue"

        if issue is None:
            return "green, no open issue"
        records = self.records(issue)
        if not records:
            # An issue opened before these markers existed: keep the old rule.
            head = self.gh(["api", f"repos/{self.repo}/commits/main", "--jq", ".sha"]).strip()
            if head != sha:
                return "green on an older commit; left open"
        else:
            runs = self.runs_since(min(record.created_at for record in records))
            if any(run.key() > me.key() and run.status != "completed" for run in runs):
                return "a newer main run is still in progress; it decides"
            unfixed = sorted(
                {
                    job
                    for record in records
                    for job in record.failed_jobs
                    if not self.fixed_later(record.created_at, record.run_id, job, runs)
                }
            )
            if unfixed:
                return "left open; not passed again yet: " + ", ".join(unfixed)
        comment = f"postmerge-ci is green again on {sha[:10]}: {run_url}"
        self.gh(["issue", "close", str(issue), "--comment", comment])
        return f"closed #{issue}"


def main() -> None:
    reporter = Reporter(run_gh, os.environ["GH_REPO"])
    print(
        reporter.report(
            int(os.environ["RUN_ID"]),
            os.environ["SHA"],
            os.environ["RESULT"],
            os.environ["RUN_URL"],
        )
    )


if __name__ == "__main__":
    main()
