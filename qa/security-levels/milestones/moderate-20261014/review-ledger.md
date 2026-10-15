# Review ledger: milestone "Moderate ships", first run

Budget: default maximum 5 (root `AGENTS.md`), counted from 0 for this record. Extensions only with explicit
authorization from Travis or the coordinator (root `AGENTS.md`, "Review budget extensions", 2026-10-14), requested
through `.codex-work/workers-20261002/qual-mod1.request.md`; the integrator never self-authorizes.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 1 | Code-blind design | `01a13ad5-b332-7610-a491-7202b0eaa190` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, packet directory only ([access record](design/designer-session.md)) | 33 cases, 12 questions |
| 2 | Design review against the owner decisions (fresh session) | `01a13add-0c6c-79e0-b31a-d0c595bd15ae` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, review packet only ([access record](design-review/reviewer-session.md)) | **CHANGES**: nine minimum changes ([output](design-review/review-output.md)); answered with amendment table v1 and Q&A v2 |
| 3 | Same session, confirmation pass | same | same | same | **CHANGES**: three wording fixes ([output](design-review/confirm-output.md)), applied as listed in table v2, not independently re-checked |
| 4 | Independent evidence check r1 (fresh session) | `01a13e72-f12d-7200-8e0a-5d27860a6674` | claude-opus-5-5-plan, high | read-only sandbox, a copy of this record at `9d95c79eb2` only ([prompt](review/evidence-check-r1-prompt.md)), 07:24Z-07:37Z on 2026-10-15 | **fail** ([output](review/evidence-check-r1.md)): MOD-15, 17, 22, 29 to failed; summary fixes (MOD-03, 08, 09, 10, 11, 26, 28); replay G18b note described as a clarification; harness and fixture deviations; named limits. All applied; issues #845 and #846 filed |

Budget after the design: 3 of 5 used; 2 remained for the evidence check.

Execution sessions are test work, not reviews; they are listed in the README.
