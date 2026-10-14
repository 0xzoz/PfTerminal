# Review ledger: milestone "Aggressive ships" third run

Budget: default maximum 5 (root `AGENTS.md`), counted from 0 for this record.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 1 | Amendment review + new-case design, pass 1 | `01a135bd-4733-7751-9da5-0f3f7a9fd13a` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, packet directory only ([access record](design/designer-session.md)) | changes to 6 rows, 4 missing amendments, 8 new cases, 7 questions |
| 2 | Same session, confirmation pass | same | same | same | agreed with v2 except AGG-32 (B), AGG-27, AGG-22 (applied in v3); withdrew AGG-T05, revised T07/T08; final table frozen at `090bd128a6` after the owner's answers |
| 3 | Independent evidence check r1 (fresh session) | `01a13814-f755-7283-8b0b-923ebd09c91b` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, record copy only (`git archive` of commit 07f9ff0155) | **fail**: AGG-R09 to passed (G24 never scrolled to Recent denials; #738 closed), AGG-18 to blocked, AGG-14/24/R14/R07/26/R04/10 summaries, README lists and limits, missing review/ and R06 evidence ([r1](review/evidence-check-r1.md)) |

Execution sessions are test work, not reviews: 37 executor sessions (32 groups, 5 replays), all with the candidate's
`corbanu exec` on GLM 5.3 Flash, one at a time, 2026-10-13 17:18Z to 2026-10-14 01:37Z.
