---
sprint_id: "PF-60-S07"
title: "Price catalogue refresh (#368)"
status: in_progress
plan_file: "docs/plans/active/portfolio-agent-cost-accounting-followups.md"
plan_feature: "PF-60"
execution_order: 2
owner: "Codex worker pf60-next; Travis Good accepts"
parallel_lane: "accounting"
write_scope: "codex-rs/core/src/config/mod.rs, codex-rs/core/src/accounting.rs, codex-rs/core/src/accounting_prices.rs, codex-rs/core/src/accounting_chat.rs, codex-rs/core/src/accounting_transport.rs, codex-rs/core/src/accounting_basis_tests.rs, codex-rs/core/src/accounting_chat_tests.rs, codex-rs/core/tests/suite/accounting_chat.rs, codex-rs/core/tests/suite/accounting_chat_recovery.rs, codex-rs/tui/src/chatwidget/tokens.rs, codex-rs/tui/src/chatwidget/tokens/charge.rs, codex-rs/tui/src/chatwidget/tokens/charge_tests.rs, docs/provider-accounts.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s07-price-catalogue-refresh.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s06-side-and-fork-attribution.md, docs/plans/active/portfolio-agent-cost-accounting-followups.md, qa/portfolio/agent-cost-accounting/issue-368/, qa/portfolio/agent-cost-accounting/pf-60-s07/, qa/demos/index/PF-60-S07.md, qa/demos/specs/"
integration_gate: "One PR to main: fail-before/pass-after tests, focused just test with and without developer-accounting, macOS and Linux clippy -D warnings, independent Opus 5.5 High review to APPROVE on the exact head, tmux check in a disposable home, SOP video; Travis accepts"
worktree: "/Volumes/CorbanuDrive/Corbanu/worktrees/pf60-s07"
branch: "pf60-s07/moonshot-route-pricing"
base_commit: "df3d8c5d1c521228a8dbb1035831af572afe1284"
depends_on: "PF-60-S04"
created: 2026-10-10
updated: 2026-10-14
---

# PF-60-S07 — Price catalogue refresh (#368)

## Execution mandate

- Deliver: item (iii) from Travis's 2026-10-10 decision, the five price items of #368, so that each request is priced
  from the provider's current published rate or plainly shows "no price". The exceptions are OpenRouter, Vercel and the Corbanu API,
  which are priced from the charge they report (AC1).
- **Deadline:** the Sol promotion item (AC4) must land before 2026-11-21.
- Excludes: re-pricing history; changing what Corbanu charges; Grok tiers (#127) and OpenRouter caps (#129).

## Plan linkage

- Plan: [Accounting follow-ups after PF-60](../../../plans/active/portfolio-agent-cost-accounting-followups.md); feature `PF-60`.
- Acceptance advanced: no request shows a stale or guessed price.

## Code boundaries

- Existing: `codex-rs/core/src/accounting_prices.rs`; `codex-rs/models-manager/models.json`;
  `codex-rs/model-provider-info/src/` (routes); `scripts/check_openai_api_prices.py`; the OpenRouter `billed_usd`
  parse in the accounting path.
- Evidence: `qa/portfolio/agent-cost-accounting/issue-361/README.md` (2026-10-09 readings) and a new dated reading.

## Preconditions

- [x] Plan is active (Travis, 2026-10-14; slot 2).
- [x] Dependencies are completed (PF-60-S04 archived 2026-10-10).
- [x] Worktree, branch, and base commit are exact and match the plan.
- [x] Travis chooses the Moonshot/BigModel scope (2026-10-14): Moonshot by route first, BigModel deferred; Vercel and
      the Corbanu API priced from their reported charges by AC1's rule; #786 option (a).
- [x] Travis chooses the OpenRouter rule for AC1 (2026-10-14).

## Done

- [x] Draft sprint created 2026-10-10 from Travis's decision; linked to one plan feature.
- [x] AC3: `gpt-6.1-sol` has a row at OpenAI's published rates, cache writes included.
- [x] 2026-10-14 data upkeep (#762; readings in `qa/portfolio/agent-cost-accounting/issue-368/`). AC4: Sol re-read
      before 2026-11-21, a test pins the promotion end, the calendar check warns from 2026-10-22. AC5: DeepSeek's
      calendar matches its page through 2026-12-31.
- [x] AC1 OpenRouter (Travis 2026-10-14; PR #784): priced from `usage.cost`, labelled "charged by OpenRouter", each
      page states its `Price basis`; the catalogue figure stays a labelled estimate; the ledger is unchanged.
- [x] AC1 extended (Travis 2026-10-14): Vercel (`gatewayCost`) and the Corbanu API (`usage.corbanu_charge_usd`) by
      the same rule.
- [x] AC2 Moonshot (Travis 2026-10-14: by route first): a provider configured at `https://api.moonshot.ai/v1` is
      priced from Moonshot's published rates (K3, K2.7 Code, K2.7 Code HighSpeed, K2.6), keyed by the route. K3 cache
      writes by TTL: this client names none, so the five-minute rate (a test pins the body). BigModel deferred by
      Travis; it stays "no price".
- [x] #786 (Travis, option a), Chat routes: reasoning above output in one usage report is recorded as not reported;
      the rest, charge included, is kept. Responses routes and split reports: #814.
- [x] AC6: new rates cite source and read date; prices bind at admission and the charge rule is display-only, so
      history is not re-priced. [Gate](../../../../qa/portfolio/agent-cost-accounting/pf-60-s07/gate.md).

## Remaining

- [ ] A real Moonshot request recomputed from provider-reported usage: needs a Kimi Open Platform key in the vault
      (none exists; the route was shown against a local stand-in, see the gate).
- [ ] DeepSeek's 2027 holiday dates, once the State Council publishes them (nightly-ci fails from 2026-12-01 until then).
- [ ] Code-blind functional design frozen before test-result disclosure (PF-60-S10).

## Verification

- [x] Focused: `just test -p codex-core -E 'test(accounting) | test(spawn)'` 357/357 with `developer-accounting`,
      352/352 without; `just test -p codex-tui --features developer-accounting -E 'test(charge) | test(tokens) | test(cost)'`
      131/132 (the local-only `cost_scope` tmux test; see the gate).
- [ ] Real requests recomputed from provider-reported usage at the cited prices: Corbanu API and Vercel charges read
      back exactly; Moonshot only against the stand-in (no key).
- [x] TUI: `/cost` pages for each changed route captured in a tmux run
      ([captures](../../../../qa/portfolio/agent-cost-accounting/pf-60-s07/tmux-20261014/)); two demo videos.
- [ ] Code-blind cases executed independently (in PF-60-S10 under the enforced gate).

## Exit evidence

- [ ] Implementation commit recorded.
- [ ] Final-tree test output and dated price readings linked.
- [ ] Code-blind handoff checker passes, or explicit limited-testing agreement/blocker recorded; no automatic human acceptance.
- [ ] Parallel handoff, if applicable: commit, contract versions, scope audit and combined-tree test evidence recorded.
- [ ] `Done` and `Remaining` ledgers reflect reality; #368 closed.
- [ ] Completed record moved to `docs/sprints/archive/portfolio-agent-cost-accounting-followups/`.
