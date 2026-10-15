# Review ledger: milestone "Aggressive ships" third targeted re-run

Budget: default maximum 5 (root `AGENTS.md`: the independent design pass and the evidence checks are charged to it),
counted from 0 for this record. Binding rule of 2026-10-14 (#447, wave2-common): going over the budget needs explicit
authorization from Travis or the coordinator, requested through a request file; no self-authorized extensions.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| – | Keystroke-driver design, attempt 1 (fresh designer) | `01a14159-4f52-7541-823b-36ba4844dca3` | claude-opus-5-5-plan, high | read-only sandbox, designer packet only | **No reply**: the session ended at 2026-10-15T21:00Z when macOS blocked the Corbanu home (`Operation not permitted` in the program's own provider auth and session store). Retried once with the same packet under the #616 rule in wave2-common. Not counted: it produced no design or verdict ([design/designer-attempt1-session.txt](design/designer-attempt1-session.txt)) |
| 1 | Keystroke-driver design (fresh designer) from the case text, the draft amendment, tools and screens only | `01a14169-48c9-7dd0-81b8-7fca3120a04a` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, designer packet only ([design/designer-packet/](design/designer-packet/), commands in [design/designer-events.jsonl.gz](design/designer-events.jsonl.gz)) | `DESIGN: COMPLETE`: AGG-R03 driver for two Corbanu Terminals, step map, 11 ambiguities; suggested wordings 3, 4, 8 and 10 taken into draft v2 ([design/designer-output.md](design/designer-output.md)) |
| 2 | Amendment-6 and driver review against the decisions, the case text and the screens | `01a14170-3154-7860-9a0b-3bc1fdbdf699` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, review packet only ([design/review-packet/](design/review-packet/)) | **CHANGES** with a final table and a final AGG-R03 driver: screen 07 must still name the restart-only protections (AGG-04/37); part B of AGG-T01 gets the relaunch check (B9) and an explicit S0 start; AGG-R03 no-prompt rule records a ran command as a failure, not a block, and the executor's `ls` happens even after an early stop; the driver's a4-01 matches only the ks-pending prompt and tolerates a trust screen after the restart or in terminal B; #739 decision quoted verbatim. Applied verbatim (table rows checked identical by script) and frozen ([design/review-output.md](design/review-output.md)) |

Budget: 2 of 5 used at the freeze; 3 remain for the independent evidence checks.
