---
sprint_id: "PF-60-S07"
title: "Price catalogue refresh (#368)"
status: in_progress
plan_file: "docs/plans/active/portfolio-agent-cost-accounting-followups.md"
plan_feature: "PF-60"
execution_order: 2
owner: "Codex worker pf60-next; Travis Good accepts"
parallel_lane: "accounting"
write_scope: "codex-rs/core/src/config/mod.rs, codex-rs/core/src/accounting.rs, codex-rs/core/src/accounting_prices.rs, codex-rs/core/src/accounting_chat.rs, codex-rs/core/src/accounting_transport.rs, codex-rs/core/src/accounting_basis_tests.rs, codex-rs/core/src/accounting_chat_tests.rs, codex-rs/core/tests/suite/accounting_chat.rs, codex-rs/tui/src/chatwidget/tokens.rs, codex-rs/tui/src/chatwidget/tokens/charge.rs, codex-rs/tui/src/chatwidget/tokens/charge_tests.rs, docs/provider-accounts.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s07-price-catalogue-refresh.md, docs/sprints/current/portfolio-agent-cost-accounting-followups/pf-60-s06-side-and-fork-attribution.md, docs/plans/active/portfolio-agent-cost-accounting-followups.md, qa/portfolio/agent-cost-accounting/issue-368/, qa/demos/index/PF-60-S07.md, qa/demos/specs/"
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
  from the provider's current published rate or plainly shows "no price". The exception is OpenRouter, which is priced from the
  charge it reports (AC1).
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
- [x] 2026-10-14 data upkeep (#368, readings in `qa/portfolio/agent-cost-accounting/issue-368/`):
      AC4 re-read (unchanged, no post-promotion price published; boundary test); AC5 no 2027 State Council notice yet,
      so `scripts/check_price_calendars.py` (nightly) fails from 2026-12-01 until the dates are added; AC2 read but not
      priceable as data (custom routes are not priced per token, BigModel bills CNY, slugs clash with Z.AI, K3 TTL writes).

- [x] AC1 OpenRouter (Travis 2026-10-14: reported charge first, catalogue as a labelled fallback estimate, else
      "no price"): `/cost` prices each OpenRouter request from `usage.cost` (`billed_usd`), labels it
      "charged by OpenRouter", and every request and attempt page states its `Price basis`. The ledger keeps the
      catalogue estimate unchanged (no re-pricing of records). Landed as a bounded fix under that decision
      (coordinator-assigned, PR #784) while the sprint stays draft for its other items; its code-blind cases run
      in PF-60-S10. Evidence: `qa/portfolio/agent-cost-accounting/issue-368/openrouter-charge/`.

## Remaining

- [ ] AC2 Moonshot/Kimi Open Platform (`api.moonshot.ai/v1`) and BigModel (`open.bigmodel.cn/api/paas/v4`): rows from
      the official pricing pages, with K3 cache writes by TTL; a request on each recomputes exactly (or the route stays
      "no price" with the reason recorded if no key is available).
- [ ] AC4: the GPT-5.6 Sol price is re-read before 2026-11-21; a test pins the promotion end so work after it uses the
      new published price or shows "no price" with a reason, never the promotional price.
- [ ] AC5: DeepSeek's holiday and off-peak calendar after 2026-10-07 matches DeepSeek's page.
- [ ] AC6: every changed rate cites its source URL and read date; history is not re-priced (test).
- [ ] Code-blind functional design frozen before test-result disclosure.

## Verification

- [ ] Focused: `just test -p codex-core --features developer-accounting -E 'test(accounting_prices) | test(accounting)'`; `python3 scripts/check_openai_api_prices.py`.
- [ ] Real requests recomputed from provider-reported usage at the cited prices, to the micro-dollar.
- [ ] TUI: `/cost` pages for each changed route captured in a tmux run.
- [ ] Code-blind cases executed independently (in PF-60-S10 under the enforced gate).

## Exit evidence

- [ ] Implementation commit recorded.
- [ ] Final-tree test output and dated price readings linked.
- [ ] Code-blind handoff checker passes, or explicit limited-testing agreement/blocker recorded; no automatic human acceptance.
- [ ] Parallel handoff, if applicable: commit, contract versions, scope audit and combined-tree test evidence recorded.
- [ ] `Done` and `Remaining` ledgers reflect reality; #368 closed.
- [ ] Completed record moved to `docs/sprints/archive/portfolio-agent-cost-accounting-followups/`.
