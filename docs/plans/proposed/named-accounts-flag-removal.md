---
title: "Named accounts (PF-84): flag removal"
status: draft
change_class: product-initiative
priority: P1
owner: "Codex PF-84 lane owner (proposed); Travis Good accepts"
activation_authority: "Travis Good"
activation_basis: "Carried from the completed provider-onboarding plan on 2026-10-14; activation pending a free slot, the test accounts and his decision"
target_release: "TBD"
deadline: "TBD"
created: 2026-10-14
updated: 2026-10-14
---

# Named accounts (PF-84): flag removal

Draft. Predecessor: [unified provider onboarding](../completed/main-2026-10-14-unified-provider-auth.md), closed
2026-10-14 to free a slot for the PF-60 follow-ups. PF-84-S01–S06 are accepted and archived; named accounts stay
behind the default-off `named_accounts` flag. This plan authorizes no implementation until Travis activates it.

## Carried forward (Travis, 2026-10-13, #720)

Before the flag is removed:

1. A live ChatGPT check with a disposable test ChatGPT account (sign-in, refresh written back to that account,
   default login untouched).
2. A live Bedrock check with a test AWS account and a least-privilege profile.
3. The S03 limits #425, #426 and #428 (fixed by #431 and #432) re-verified.
4. The partial #555 legs re-run (ESC-03, REG-01, GH-02, PR-01, ESC-04, ESC-01, the D3 approvals-off leg); REG-02
   decided.
5. Open follow-ups closed or documented: #588, #609, #689 (bug fixes; they need no active slot).
6. An isolated code-blind run from a frozen design on the packaged candidate, then Travis's sign-off.

Also open from the predecessor: PF-58-S01 (draft residual qualification), named human acceptance and release linkage.

## Decisions for Travis

- Activation and slot, once the test accounts exist (items 1–2 are blocked on them; mock-only testing was accepted
  for S06 on 2026-10-13).
