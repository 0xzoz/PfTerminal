# Review ledger: milestone "Aggressive ships" second targeted re-run

Budget: default maximum 5 (root `AGENTS.md`: the independent design pass and the evidence checks are charged to it),
counted from 0 for this record. Binding rule of 2026-10-14 (#447, wave2-common): going over the budget needs explicit
authorization from Travis or the coordinator, requested through a request file; no self-authorized extensions.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 1 | Keystroke-driver design (fresh designer) from the case text, tools and screens only | `01a13cc1-be21-7c62-9bf7-19bd1d23e6ab` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, designer packet only ([design/designer-packet/](design/designer-packet/), commands in [design/designer-events.jsonl.gz](design/designer-events.jsonl.gz)) | `DESIGN: COMPLETE`: drivers for AGG-R03 and AGG-R14 with a step-to-case mapping and ambiguities ([design/designer-output.md](design/designer-output.md)) |
| 2 | Amendment-5 and driver review against the decisions, the case text and the screens | `01a13ccf-596f-74f2-8980-26697b8e57bd` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, review packet only ([design/review-packet/](design/review-packet/)) | **CHANGES** with a final table and a final AGG-R03 driver (AGG-R14 unchanged): old text quoted verbatim, AGG-37 y-then-Enter and the product-home path, the #739 decision text, BLOCKED-at-action-2 rule, navigation the drivers add, AGG-R03 timing; applied verbatim and frozen at `aaf2f999f7` ([design/amendment-review-output.md](design/amendment-review-output.md)) |
| 3 | Independent evidence check r1 (fresh session) | `01a13d1e-2729-71a3-9b1c-1de68951e7c1` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, record copy only (`git archive` of commit be416ffd92) | **fail**: AGG-04 result 3 not shown as written (to blocked pending owner ruling #816), AGG-37 owner-rule wording and skipped resubmission, AGG-R03 "could not be reached" overstated; AGG-R14 correction confirmed with further executor-report errors; freeze, drivers, isolation and owner-accepted cases confirmed ([r1](review/evidence-check-r1.md)) |

Execution sessions are test work, not reviews: 4 executor sessions (groups V01-V04), all with the candidate's
`corbanu exec` on GLM 5.3 Flash, one at a time, 2026-10-15 00:11Z to 00:51Z.
