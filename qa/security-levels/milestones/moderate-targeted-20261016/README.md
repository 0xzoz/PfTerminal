# Milestone "Moderate ships": targeted re-run (2026-10-16)

**Status: design frozen, execution pending.** This file is completed after the run.

Re-run of 17 of the 34 Moderate cases after the fixes for the first run's defects, per Travis's decisions of
2026-10-15 (all 13 as recommended) and the coordinator's brief of 2026-10-16. First run:
[../moderate-20261014/](../moderate-20261014/) (16 passed, 15 failed, 3 blocked).

- Amendment v3: written by a fresh read-only code-blind designer from the frozen cases, amendments v2 and the
  decisions only ([design/designer-prompt.md](design/designer-prompt.md), packet [design/designer-packet/](design/designer-packet/)),
  reviewed by a separate fresh read-only reviewer (VERDICT: CHANGES; its final version frozen word for word:
  [design/amendments-v3-final.md](design/amendments-v3-final.md)).
- Executor packet v3: [executor-packet/v3/](executor-packet/v3/) (14 groups Q01-Q14).
- Candidate: [candidate/candidate-manifest.md](candidate/candidate-manifest.md).
- Not re-run: MOD-11 and MOD-06's git-commit part (pending owner decision D5, #837); the 16 cases that passed.
- Frozen before any VM start: [design/FROZEN.sha256](design/FROZEN.sha256).
