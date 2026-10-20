# Review ledger: acc-tart1 (PF-60 follow-up live run)

Budget: 5 rounds in total for design and evidence checks (acc-tart1 brief; wave2-common REVIEW BUDGET rule: an
extension needs Travis's or the coordinator's authorization through `acc-tart1.request.md`; none self-authorized).

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 1 | Code-blind design (fresh designer) from the intent packet | `01a156fc-532e-7ee0-a6c0-0096f4ca3b19` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, packet directory only | 24 cases ([design/designer-output.md](design/designer-output.md), [designer-session.md](design/designer-session.md)) |
| 2 | Independent design review against intent, owner decisions and harness facts | `01a15705-7322-7e62-8fba-775f047cca89` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, review packet only | **CHANGES**, 25 amendments (new case FUP-25), applied as written, not re-reviewed ([design/review-output.md](design/review-output.md), [review-session.md](design/review-session.md)) |

Budget at the freeze: 2 of 5 used; 3 remain for independent evidence checks.

| # | Kind | Agent / session | Model | Isolation | Result |
| --- | --- | --- | --- | --- | --- |
| 3 | Independent evidence check r1 (fresh session) | `01a157d1-376d-7cc2-aece-a2cdf576e998` | claude-opus-5-5-plan, high | instruction-only, read-only sandbox, record copy only (`git archive` of commit 0991967f0e) | **fail**. Confirmed: the freeze, packet identity, amendments, isolation (109 probes per group, 28 receipts), caps, key deletion and the totals. Corrections: FUP-01's sum omitted the cache-read cost; FUP-16's sum carried FUP-10's Request 1 slip; FUP-10's Request 1 derivation and the skipped intermediate audits; FUP-17's navigation facts and the pty.raw source; FUP-14's code-derived claims labelled; FUP-02 passed -> blocked; FUP-03's sum wording; the shakedown's real OpenAI WebSocket (probable ~$0.029 warm-up; D16 total now stated as ~$0.063, over $0.05); screen archives (Z, O only); limits (designer and reviewer isolation, lock release and UTM end state, now in common/host-end-check.txt; corrections to the hash-bound policy go to common/effective-policy-addendum.txt). All applied ([r1](review/evidence-check-r1.md)) |
