# Tart evaluation evidence (2026-10-16, #864)

Raw outputs behind [docs/qa/tartEvaluation.md](../../docs/qa/tartEvaluation.md). Tart 2.41.1;
TART_HOME `/Volumes/CorbanuDrive/Corbanu/.codex-work/tart-home`; one Tart VM at a time while UTM
"macOS 3" ran qual-mod2 (never touched).

- `pull.txt`: image pull start, duration in seconds, end.
- `prepare-base.out`: the final base build (`corbanu-qa-base`).
- `cycles/`: one JSON row per `cycle.sh` run. `final-base.jsonl` holds the reported median. The
  others come from the earlier base builds: `base1-*` (Gatekeeper still off) and `base2-ssh-fda-off`.
  In `base1-a-trial`, run 2's model turn failed: the vault lookup ran while the host's Corbanu was
  being reinstalled, and run 3 stopped at a binary hash mismatch for the same reason; both led to
  the private binary copy and the retry in `cycle.sh`. Binaries: `cb42614c…` in `base1-a-trial`,
  `ecc044fb…` (the build installed at 10:53Z) in the rest.
- `fidelity.txt`: `fidelity.sh` on the final base. `fidelity-ssh-fda-off.txt`: the same checks on a
  base built without Full Disk Access for SSH (throwaway keychain password replaced by `<throwaway>`).
- `suspend.txt`: `tart suspend`/resume and clone-of-suspended timings (manual test, then deleted).
- `case-runs/GNN/`: per group, the host timeline, isolation gate summary, lane preflight, the
  executor's own probe (`preflight-executor.json`), its report (`result.md`), the guard count, the
  executor exit and the key-erase confirmation. Run with:
  `HARNESS=.codex-work/moderate-first-20261014/h PACKAGE=.codex-work/moderate-first-20261014/candidate2/package PACKET=.codex-work/moderate-first-20261014/stage/packet scripts/qa/tart/case-run.sh G05 G15 G10 G22`.
  Screens, PTY logs and product logs stayed outside the repository.
