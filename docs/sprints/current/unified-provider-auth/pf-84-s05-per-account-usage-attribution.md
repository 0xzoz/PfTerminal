---
sprint_id: "PF-84-S05"
title: "Per-account usage and rate-limit attribution"
status: in_progress
plan_file: "docs/plans/active/unified-provider-auth.md"
plan_feature: "PF-84"
execution_order: 27
owner: "Codex PF-84 lane owner"
parallel_lane: "account-usage"
write_scope: "codex-rs/state/, codex-rs/core/, codex-rs/tui/, codex-rs/model-provider/, codex-rs/model-provider-info/, codex-rs/vault/, codex-rs/login/, codex-rs/Cargo.lock, qa/provider-auth/pf-84/s05-gate.md, qa/provider-auth/pf-84/s05/, qa/demos/specs/pf84-usage-by-account.toml, qa/demos/specs/pf84-rate-limit-by-account.toml, qa/demos/index/PF-84-S05.md, docs/sprints/current/unified-provider-auth/pf-84-s05-per-account-usage-attribution.md, docs/sprints/current/unified-provider-auth/index.md"
integration_gate: "Codex PF-84 lane owner merges to main after just test -p codex-state -p codex-core -p codex-tui (plus touched crates) with and without developer-accounting, Linux clippy -D warnings on glitch, the tmux/GLM run and one Opus 5.5 High review"
worktree: "/Volumes/CorbanuDrive/Corbanu/worktrees/pf84-s05-usage"
branch: "feat/pf-84-s05-usage-by-account"
base_commit: "c7e6e07cbc629b8fa366de30cfacefae23b06b69"
depends_on: "PF-84-S03, PF-60-S04"
created: 2026-10-08
updated: 2026-10-11
---

# PF-84-S05 — Per-account usage and rate-limit attribution

## Execution mandate

- Deliver: every accounted request, rate-limit snapshot and provider-request lease
  records its provider account. `/status` shows the current account. Usage views
  group by (provider, account) across the coordinator and its workers in one home.
- Excludes: pricing changes; cross-home aggregation (separate homes stay separate);
  changes to the PF-60 accounting contract beyond adding the account field.

## Plan linkage

- Plan: [Unified provider onboarding and management](../../../plans/active/unified-provider-auth.md)
- Feature: `PF-84`
- Acceptance advanced: "Per-account usage".

## Code boundaries

Paths are under `codex-rs/`, at `63ea3d0cbd`.

- Accounting records: `core/src/accounting.rs:48-68, 225-269, 346-463` (keyed
  by `provider_id` today); schema in `state/accounting_migrations/`
  (`state/src/migrations.rs:7-10`).
- Rate-limit leases: `state/src/runtime/provider_requests.rs:8-12, 92, 127`. These
  are already keyed by `key_fingerprint`; make that the account fingerprint for
  every auth kind, including subscription logins.
- Display: `tui/src/chatwidget/tokens.rs:1085` (provider display name) and the `/status` card.
- Tests: accounting unit tests, a migration test, a status snapshot.

## Preconditions

- [x] Plan is active.
- [x] Dependencies are completed (PF-60-S04 archived 2026-10-10; PF-84-S03 archived; S04 `ready` behind the flag).
- [x] Worktree, branch, and base commit are exact and match the plan.
- [x] Serial in the PF-84 lane; the integration gate is recorded above.

## Done

- [x] Sprint record created and linked to one plan feature.

## Remaining

- [ ] Add `account_name` and `account_fingerprint` (12 hex, non-reversible) to
  accounting rows through an additive migration; old rows read as `default`.
- [ ] Rate-limit state per (provider, account); account B's 429 never blocks A.
- [ ] `/status` and usage views show and group by account.
- [ ] Code-blind functional design frozen before test-result disclosure, or non-user-facing N/A reason recorded.

## Verification

- [ ] Focused: `just test -p codex-core -p codex-state -p codex-tui` (accounting, migration, snapshot).
- [ ] tmux SOP: a coordinator on account A and a spawned worker on account B each make
  requests, and the usage view shows two rows with the right counts.
- [ ] GLM run: `-m glm-5.3-flash -c model_provider="zai"` on two ZAI accounts (one real,
  one fake). Only the real one accrues tokens, and the fake one records the failure.
- [ ] Video: `pf84-usage-by-account.toml` (`--sprint PF-84-S05 --publish`).
- [ ] One independent review (Opus 5.5 High).

## Security notes

- Accounting stores names and fingerprints only. The fingerprint derivation is
  salted per home so it cannot be correlated across machines.
- No credential value reaches the state DB, logs or exports (canary scan).

## Exit evidence

- [ ] Implementation commit and PR recorded.
- [ ] Test output, tmux log and video path linked.
- [ ] Code-blind handoff checker passes, or limited-testing agreement recorded.
- [ ] `Done` and `Remaining` ledgers reflect reality.
- [ ] Completed record moved to `docs/sprints/archive/unified-provider-auth/`.
