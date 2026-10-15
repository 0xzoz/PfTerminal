---
sprint_id: "PF-60-S08"
title: "/cost page clarity and reopen warm-up cost"
status: in_progress
plan_file: "docs/plans/active/portfolio-agent-cost-accounting-followups.md"
plan_feature: "PF-60"
execution_order: 3
owner: "Codex worker pf60-next; Travis Good accepts"
parallel_lane: "accounting"
write_scope: "codex-rs/core/src/session_startup_prewarm.rs, codex-rs/core/src/session/session.rs, codex-rs/core/src/session/mod.rs, codex-rs/core/src/accounting.rs, codex-rs/core/src/config/mod.rs, codex-rs/config/src/config_toml.rs, codex-rs/core/config.schema.json, codex-rs/core/tests/suite/accounting_responses_ws_recovery.rs, codex-rs/tui/src/chatwidget/tokens.rs, codex-rs/tui/src/chatwidget/tokens_tests.rs, docs/config.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s08-cost-page-clarity-and-reopen-warmup.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s07-price-catalogue-refresh.md, docs/plans/active/portfolio-agent-cost-accounting-followups.md, qa/portfolio/agent-cost-accounting/pf-60-s08/, qa/demos/index/PF-60-S08.md, qa/demos/specs/pf60s08-reopen-warmup-cost.toml"
integration_gate: "One PR to main: fail-before/pass-after tests, focused just test with and without developer-accounting, macOS and Linux clippy -D warnings, independent Opus 5.5 High review to APPROVE on the exact head, tmux check with a real OpenAI key in a disposable home, SOP video; Travis accepts"
worktree: "/Volumes/CorbanuDrive/Corbanu/worktrees/pf60-s08"
branch: "pf60-s08/cost-clarity-reopen-warmup"
base_commit: "45916ab64ef7faf4a5525a8dbdbfd4f706dd9f79"
depends_on: "PF-60-S04"
created: 2026-10-10
updated: 2026-10-14
---

# PF-60-S08 — `/cost` page clarity and reopen warm-up cost

## Execution mandate

- Deliver: items (v), (vi) and (vii) from Travis's 2026-10-10 decision.
  - (v) #396 item 1: an unpriced request page shows "Cache write cost: $0.000000" for 0 tokens.
  - (vi) #403: every reopen of an OpenAI conversation (`/resume`, restart, `/side`) sends a paid warm-up request,
    about $0.025 on `gpt-5.4`. It is recorded, but users don't expect a charge for reopening. Make it visible or avoidable.
  - (vii) #396 item 2: with a declared basis the line reads "(pay per use (set in your config))".
- Excludes: price changes (PF-60-S07); #352's other display nits unless Travis adds them.

## Plan linkage

- Plan: [Accounting follow-ups after PF-60](../../../plans/active/portfolio-agent-cost-accounting-followups.md); feature `PF-60`.
- Acceptance advanced: reopening shows no surprise charge; unpriced pages show no money figure.

## Code boundaries

- Existing: `codex-rs/tui/src/chatwidget/tokens.rs` (request page, basis line); `codex-rs/core/src/session_startup_prewarm.rs`;
  `codex-rs/core/src/accounting.rs::prewarm_turn_label`.
- Tests: `codex-rs/tui/src/chatwidget/tokens_tests.rs`, `codex-rs/tui/src/chatwidget/tokens/scope_tests.rs`; literal list at readiness.

## Preconditions

- [x] Plan is active (Travis, 2026-10-14; slot 2).
- [x] Dependencies are completed (PF-60-S04 archived 2026-10-10).
- [x] Worktree, branch, and base commit are exact and match the plan.
- [x] Travis chooses the warm-up option for AC3 (2026-10-14): (C), show the reopen warm-up in `/cost` and on
      reopen, plus a setting to skip it, off by default so behaviour stays as today.

## Done

- [x] Draft sprint created 2026-10-10 from Travis's decision; linked to one plan feature.
- [x] AC1 (#396.1): a component with no rate states no money; a 0-token one reads "none — 0 tokens". Components with
      a rate keep their figure, zero included.
- [x] AC2 (#396.2): the basis-only price line reads "records only its billing basis: pay per use, set in your
      config"; the `Billing basis:` line keeps its parentheses.
- [x] AC3 (#403, option C): a resumed or forked session's warm-up is labelled `prewarm:reopen:<id>` (still a
      `prewarm:` label); once it is sent the session shows a notice naming `skip_reopen_warmup`, not written to the
      conversation, so a later reopen doesn't replay it. `/cost` says "Reopen warm-up: N request(s), <cost>" on the
      first screen and "Reopen warm-up" on the request link and page. `skip_reopen_warmup = true` (default false)
      sends no warm-up on reopen; a new conversation still warms up. Earlier totals are unchanged; nothing is hidden
      or counted twice. [Gate](../../../../qa/portfolio/agent-cost-accounting/pf-60-s08/gate.md).

## Remaining

- [ ] Code-blind functional design frozen before test-result disclosure (PF-60-S10).
- [ ] Travis accepts S08; #396 and #403 closed on acceptance.

## Verification

- [x] Focused: core `test(accounting) | test(spawn) | test(prewarm) | test(websocket) | test(config)` 1094/1094 with
      `developer-accounting`, 964/964 without; codex-tui `test(tokens) | test(charge) | test(cost) | test(resume)`
      306/307 (the local-only `cost_scope` tmux test).
- [x] macOS and Linux clippy `-D warnings --tests`, with and without `developer-accounting`.
- [x] TUI: tmux with a real OpenAI key (restart reopen, `/resume`, `/side`, skip setting, Fast tier, declared
      basis) and the demo video; see the gate.
- [ ] Code-blind cases executed independently (in PF-60-S10 under the enforced gate).

## Exit evidence

- [ ] Implementation commit recorded.
- [ ] Final-tree test output linked.
- [ ] Code-blind handoff checker passes, or explicit limited-testing agreement/blocker recorded; no automatic human acceptance.
- [ ] Parallel handoff, if applicable: commit, contract versions, scope audit and combined-tree test evidence recorded.
- [ ] `Done` and `Remaining` ledgers reflect reality; #396 and #403 closed.
- [ ] Completed record moved to `docs/sprints/archive/portfolio-agent-cost-accounting-followups/`.
