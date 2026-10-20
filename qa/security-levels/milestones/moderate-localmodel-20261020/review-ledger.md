# Review ledger: Moderate local-model re-run (2026-10-20)

Budget: 5 rounds (wave-2 rule), design and evidence together, as in the earlier Moderate records.

| Round | What | Who | When (UTC) | Verdict | Notes |
| --- | --- | --- | --- | --- | --- |
| 1 | Designer: amendment v5 | fresh read-only Opus 5.5 High, code-blind | 08:11:37-08:16 | DESIGN: COMPLETE | `design/designer-output.md` |
| 2 | Design review | fresh read-only Opus 5.5 High, code-blind | 08:23:00-08:27 | CHANGES | Final version frozen word for word. Attempt 1 (08:17:44Z) was cut off by the provider's content filter with no verdict and retried once unchanged, counted as this round. A launch at 08:17:29Z was stopped before any reply to add `screens-v3/`. |
| 3 | Evidence check r1 | fresh read-only Opus 5.5 High | 09:30:34-09:40 | fail | Dispositions supported. Corrections applied: MOD-09 also fails result 8 for item 5 (no "one expected result"), the G-1 reading of item 5 disclosed; the "no real key" claim now shows where `auth.json` comes from (`harness/seal/`); the `tools.md` edit during the design added to the limits. Recommended items also applied: the child's command outputs (`runs/R02/product-excerpt.txt`), #1077's wording, file names in the post-freeze diff, the AGG-17..33 note removed. `review/evidence-check-r1.md` |
| 4 | Evidence check r2 | fresh read-only Opus 5.5 High | 09:42:29-09:52 | fail | Dispositions and round-3 corrections confirmed. Corrections applied: `harness/seal/` named as exported after the run (origin, hashes; freeze bullet corrected); the key claim narrowed to "no real key copied in; coordinator key files synthetic"; MOD-13 precedent described as the same "Security level:" form. Recommended items applied (R01 gate check in the host-logs limit; R02 excerpt in the export bullet). `review/evidence-check-r2.md` |
