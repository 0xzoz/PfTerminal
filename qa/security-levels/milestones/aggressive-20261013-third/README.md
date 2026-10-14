# Milestone "Aggressive ships": third run (2026-10-13)

Third code-blind functional run of the P1 security milestone, after the defects of the second run
([../aggressive-20261011-rerun](../aggressive-20261011-rerun), #628: 41 passed, 10 failed, 6 blocked of 57) were fixed
and the owner decisions of 2026-10-13 were made. **Not passed: the milestone needs Travis's named sign-off.**

## Result

64 cases: **51 passed, 6 failed, 7 blocked** ([dispositions.md](dispositions.md), [results.json](results.json)).

- Failed:
  - AGG-10: `security find-generic-password` was offered for approval, then ran with no Aggressive denial (#736).
  - AGG-T03: `git config --global` (~/.gitconfig) was offered Grant once and lacked the #620 explanation (#737).
  - AGG-T01 and AGG-T02: under Permissive with the security features on, the approved outside write was refused by
    "Protected launch", so the file never changed (#739). The #626 behaviour itself held: the remembered approval was
    suspended under Aggressive with the note and applied again at Permissive.
  - AGG-01 and AGG-25 fail against frozen expectations that a later decision or fix made stale, with no owner decision
    amending them (as in run 2):
    - AGG-01 now differs only in the Vault row's wording, from decision #440;
    - AGG-25's E1 flag no longer blocks the preflight (#438).
- Blocked because the product's model declined a step, so the product's own denial was never shown:
  - AGG-18: the grant worked; the `cat .env` step was declined.
  - AGG-R07: steps 1-5 were product denials with the right wording, but step 6 was declined.
  - AGG-R10: three reads were denied correctly, but the model refused `grep -r`.
  - AGG-20: the child's model declined to start a role.
- Blocked for other reasons:
  - AGG-R03: the prompt appeared before pane A could be left.
  - AGG-R06: there is no Claude Code in the guest.
  - AGG-R14: the stop could not be credited to the kill switch.
- Compared with run 2, these now pass:
  - AGG-33: Grant once expiry on the in-folder hooks file `.husky/pre-commit`.
  - AGG-24, AGG-38 and AGG-R09.
  - AGG-27 (#664/#625) and AGG-32 (owner answer (b)).
  - AGG-R13 (#623) and AGG-R17 (#620).
- Other changes from run 2:
  - AGG-18 and AGG-R07 moved from failed to blocked. Their earlier defects (#620, #621) are fixed, and the remaining
    step was declined by the model.
  - AGG-R10 moved from passed to blocked, under the stricter rule above.
  - AGG-10 moved from passed to failed: in run 2 the model declined its third step, and this time the step ran.
  - Every other run-2 disposition is unchanged.
- Advisory finding: Grant once is offered where it cannot apply (#740).
- Some passes rest partly on the product's model declining a step whose expected wording the model's text still met.
  Each is disclosed in its summary: AGG-14, AGG-21, AGG-24 (setup steps), AGG-26 (step 4), AGG-30 and AGG-R08 (step 3).
- Executor verdicts the integrator changed, with the reasons in the summaries:
  - to passed: AGG-06, AGG-30, AGG-R16, and AGG-R09 (its first attempt was incomplete);
  - to blocked: AGG-18, AGG-20, AGG-R07, AGG-R10, AGG-R14.
- `qa/code-blind-functional/check.py` stops at the first unresolved case, as expected while there are failures and
  blocks ([checker-output.txt](checker-output.txt)).

## Candidate

`origin/main` `4b6dd55c85291dcf4458c285f0fc675a0121fd75`, packaged macOS arm64 0.1.48, `bin/corbanu` SHA-256
`ad288be9…0675043` ([candidate/candidate-manifest.md](candidate/candidate-manifest.md)).

## Design

The cases as they stood after run 2 are reused: the first run's frozen design plus run 2's amendments and AGG-R cases
([design/design-r2-20261012.json](design/design-r2-20261012.json)).

The 2026-10-13 owner decisions (#620 option B, #626, #623, #653/#664) are applied as a third amendment table. AGG-T
cases cover the new behaviours. One fresh read-only agent (`corbanu exec`, Opus 5.5 High) did both jobs:
- it reviewed the table against the decision comments only;
- it designed the AGG-T cases from a user-intent packet with 14 screens of this candidate, captured on the host
  ([design/designer-session.md](design/designer-session.md)).

Two answers were pending: (a) #505 timing and (b) AGG-32. Travis answered B and B at 17:10Z
([design/qual-aggr3.go](design/qual-aggr3.go), comments on #505 and #447). The final table
([design/amendments3-final.md](design/amendments3-final.md)) and the executor packet were frozen with SHA-256 at
commit `090bd128a6` before the VM was started ([design/FROZEN.sha256](design/FROZEN.sha256)). `design.json` combines
everything ([tools/make_design3.py](tools/make_design3.py)).

The grant cases use `.husky/pre-commit` with `git config core.hooksPath .husky`. The user docs say the project's own
`.git/config` stays read-only even with Grant once, so it could not show a grant. A pre-check in each group showed
the hooks file is protected without a grant.

## Execution

- **Where:** isolated execution in the UTM guest "macOS 3", per `qa/code-blind-functional/isolated-execution.md`, with
  the harness in [harness/](harness/).
- **Guest reset:** run 2 had ended without its restore. The guest was first restored to baseline, then provisioned
  fresh ([common/effective-policy.txt](common/effective-policy.txt)).
- **Isolation gate:** re-measured first ([common/gate/](common/gate/): 85 probes ok, source hosts unresolvable,
  inference reachable, smoke turn ok). Every group re-ran the probes before its executor.
- **Native-prompt guard:** stayed clear, with 0 `SECURITYAGENT-SEEN` in 29,039 checks
  ([common/guard-final.txt](common/guard-final.txt)).
- **Groups:** 32 groups (G01–G32, 2026-10-13 17:18Z to 2026-10-14 00:27Z), then 5 replays (G22b, G24b, G28b, G28c,
  G28d). The replays' reasons and neutral notes are in [tools/replays.json](tools/replays.json); earlier attempts are
  kept.
- **Executors:** one fresh executor per group (`corbanu exec` of the candidate, GLM 5.3 Flash), run one at a time on
  rotating accounts that were wiped before and after.
- **Product under test:** GLM 5.3 Flash on `zai`. Exceptions: AGG-A01 (GLM 5.2 on `zai-anthropic`, as the case
  requires), and AGG-27 and AGG-T07/T08, which also used the Claude-style connection.
- **Key:** the ZAI key reached the guest only through the owner-approved mediator path of the first run.
- **Shutdown:** at the end, the guest's campaign data was erased (`common/restore-end.out`). "macOS 3" was stopped
  with `utmctl stop`, and "macOS 2" stayed stopped throughout.

## Evidence check

Independent fresh-agent checks (never an executor), within the review budget of 5 ([review-ledger.md](review-ledger.md),
[review/](review/)):
- **r1** failed. It led to these corrections:
  - AGG-R09 moved from failed to passed, and #738 was closed;
  - AGG-18 moved from passed to blocked;
  - the AGG-14, 24, R14, R07, 26, R04 and 10 summaries were corrected;
  - the README lists and limits were completed.
- **r2** failed. It led to these corrections:
  - AGG-R07 and AGG-R10 moved to blocked, by the same rule as AGG-18;
  - the AGG-38 deviation is now disclosed;
  - AGG-28's relay result is recorded;
  - the README run-2 comparison and the token and window limits were corrected.
- Both checks verified the relay claims, every reference hash, the freeze and the isolation receipts themselves.

## Known limits

- macOS only. No code-blind milestone execution exists for Windows or Linux; that is a limit, not a pass.
- #505: the restart-only protections named on screen 07 still need a restart; owner answer (a) kept that list for
  this run (#662 is finished afterwards).
- AGG-R06 (kill switch interrupting a Claude pane) was not executed: there is no Claude Code in the guest.
- Isolation of the designer and checker is instruction-only (read-only sandbox, no OS read restriction).
- The packet screens were captured on the host Mac, not in the guest, because the VM could not start before the go
  file. Disposable profile, no native keyring, paths shortened.
- Mediator rows are not labelled by lane. Relay claims (AGG-R01, R04, AGG-27, T07, T08) rest on request time, path and
  size in the run's `mediator-window.jsonl`.
- The executor model is set by the harness default (`harness/run_group.sh`); the executor is the candidate binary
  itself.
- Executors run the candidate's `corbanu exec` with `--dangerously-bypass-approvals-and-sandbox`. Their confinement is
  the OS account, per-uid pf and file modes, verified by the probes, not the product sandbox
  (common/effective-policy.txt).
- Product and executor requests both carry 14 tools, so tool count cannot tell them apart in the relay. At the start
  of a run their sizes can overlap (G22b: executor 39,644 bytes, product 38,216 bytes); after that the executor's
  context grows past the product's.
- After the lane wipe, 7 to 22 processes owned by the lane account are still running (`runs/*/wipe-*.txt`, a count only),
  and the record does not identify them. Home files and temp files are wiped.
- The fixture's synthetic `.env` token reached the executors, never the product.
  - The harness's own `agms-tui fixture` prints test-data contents, so the token appears in 27 of the 37 executor
    transcripts, in 26 `fixture.jsonl` files and in the G08 and G19 reports. The G13 and G30 executors also disclosed
    it.
  - Every PTY log and screen file was checked: the product never displayed the token.
- Each run's `mediator-window.jsonl` starts about 2 minutes before its executor, so it can include the previous group's
  last requests. For example, G22's first three rows belong to G21's executor.
- Some attempts include in-attempt retries that are disclosed only in the executors' reports: G06 discarded its first
  AGG-09 try; G28d discarded an invalid preliminary AGG-R17 setup; G31 re-sent steps (d) and (h); G28 resubmitted the
  second AGG-R17 command.
- Correction to frozen files, which are not edited: `design/designer-session.md` and `design/packet/new-screens/README.txt`
  give the packet-screen capture window as 14:38–14:53Z. The capture files are dated 14:39:56Z–14:48:22Z, before the
  designer started at 14:49:18Z.
- Product logs (`collected/product-logs/*.log`) are kept out of git by the repository's `*.log` rule, as in run 2.
- Each run's test project is under `runs/<label>/collected/fixture/workspace/`. Its git directory is stored as
  `dot-git/`, and its synthetic `.env` is ignored by the project's own `.gitignore`. Raw PTY logs and SQLite files are
  gzipped.
