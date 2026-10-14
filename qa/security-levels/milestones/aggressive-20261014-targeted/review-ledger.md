# Review ledger: milestone "Aggressive ships" targeted re-run

Budget: default maximum 5 (root `AGENTS.md`), counted from 0 for this record. Binding rule of 2026-10-14 (#447,
wave2-common): going over the budget needs a major defect AND Travis's explicit authorization through the coordinator;
no self-authorized extensions.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 1 | Amendment-4 review against the decision comments only | `01a13954-bc1d-7d31-97e1-699adfdceda2` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, review packet only ([design/review-packet/](design/review-packet/), commands in [design/amendment-review-events.jsonl.gz](design/amendment-review-events.jsonl.gz)) | **CHANGES** with a final table: decision citations, AGG-25 part B wording, AGG-R10 expectation, "web search" removed from the #662 rows, AGG-04 added expectation tightened; no missing amendment; final table applied verbatim and frozen at `40e36a39cf` |
| 2 | Independent evidence check r1 (fresh session) | `01a13a3a-a529-76a1-aed6-c4b50e57cb9e` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, record copy only (`git archive` of commit 5d165941bb) | **fail**: AGG-R07 to blocked (model-decline rule read strictly as for AGG-18/R10), AGG-R03 reason, limits (lane processes carry over, relay-window overlap, leftover admin scripts), host-check binary, freeze time, AGG-R10 extra cat .env, frozen-file citation; every other disposition supported ([r1](review/evidence-check-r1.md)) |

Execution sessions are test work, not reviews: 16 executor sessions (14 groups, 2 replays), all with the candidate's
`corbanu exec` on GLM 5.3 Flash, one at a time, 2026-10-14 08:10Z to 11:35Z.
