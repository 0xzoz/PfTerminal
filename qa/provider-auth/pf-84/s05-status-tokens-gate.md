# PF-84 follow-up gate: token usage per account in `/status`

Travis's decision of 2026-10-11 (option b) on S05's open question: release builds get a light per-account
token summary in `/status`, with no cost records. Cost tracking (`/cost`) stays developer-accounting only.
Branch `pf-84-status-account-tokens`; behind `named_accounts`.

## What shipped

- **Core.** With named accounts on, each request's provider-reported usage (the same `record_token_usage_info`
  path that feeds **Token usage**, compaction and the completion check included) is added to the
  (provider id, account name) that served it. The token count event carries it as `account_usage`, which is
  omitted otherwise, so other events and rollouts keep their bytes. A resumed thread restores it from its
  rollout; a fork (spawned agent with forked context, side conversation) starts at zero, so nothing is counted
  twice. Names only: no key, no fingerprint.
- **App server.** `thread/tokenUsage/updated` has an optional `accountUsage`, also on the resume replay.
- **TUI.** `/status` adds **Account usage** for the active provider, summed over every thread of the session
  (coordinator, spawned agents, side conversations): one account on one line (wrapped to the card), several as
  an aligned table with the session's account marked `(active)`; `no usage yet` before the first reply. Input
  is the uncached part, as on **Token usage**; cached input and reasoning only when reported. A new session
  starts from zero; a thread switch keeps the totals.

## Tests (macOS, `just test`, isolated profile)

| Command | Result |
| --- | --- |
| `just test -p codex-tui -E 'test(/account_usage\|status\|thread_account_usage\|token\|account/)'` | 638/642; the 4 failures are environmental (below) |
| same with `--features developer-accounting` (adds `cost`) | 641/646; same 4, plus `cost_scope` tmux (below) |
| `just test -p codex-core -E 'test(/account\|token_usage\|token_info\|token_count\|accounting\|named_account\|resume\|throttle\|provider_request/)'` | 326/326 |
| same with `--features developer-accounting` | 330/331: `request_user_input_interrupt_emits_deferred_token_count` fails identically on `origin/main` with the feature (checked) |
| `just test -p codex-app-server-protocol` (schema fixtures) | 287/287 |
| `just test -p codex-app-server -E 'test(/token_usage\|thread_resume\|thread_fork\|token_count/)'` | 80/80 |
| `just test -p codex-exec -E 'test(/token_usage/)'`; `-p codex-thread-store -p codex-external-agent-migration` | 1/1; 307/307 |

Environmental TUI failures: `multi_provider_onboarding::tmux_corbanu_env_*` (3, "path must be shorter than
SUN_LEN", as in S05) and two tmux tests that wait for the footer's `TPS:`, which this worktree's long path
truncates (`claude_auth::tmux_first_run_…`) or that typed `/cost` before the session started
(`cost_scope::…`, the S05 flake). None touches `/status`.

New tests: `status/account_usage_tests.rs` (threads add up per account, other providers left out, active first,
one line, table alignment); insta snapshots `status_snapshot_account_usage_{flag_off,no_usage_yet,one_account,
several_accounts}`; `status_account_usage_flag_off_leaves_status_unchanged` (with reports recorded, flag off
renders byte-identical to no reports) and `status_account_usage_lists_the_active_providers_accounts`;
`thread_account_usage_reaches_status_and_resets_with_the_session` (App); core
`token_usage_is_attributed_per_account_only_with_named_accounts`, `resume_restores_account_usage_and_fork_does_not`
and `accounting_chat_named_account_summary_agrees_with_the_ledger` (two turns on a named account: input,
cached, output, reasoning and total equal the ledger's `DayTotals` that `/cost` sums).

Clippy `-D warnings`: clean on macOS and on glitch (`-j10`) for every touched crate, with and without
developer-accounting.

## Live tmux check (GLM 5.3 Flash, disposable home, `CORBANU_TEST_NO_NATIVE_KEYRING=1`)

Script: [`s05-status-tokens/scripts/tmux-check.sh`](s05-status-tokens/scripts/tmux-check.sh). Two named Z.AI
accounts, `work` and `personal`, both enrolled with the real key from `provider/zai_api_key` (read only inside
the consuming command). The session ran with `--account work`; a spawned agent ran on `personal`.

- **Release-path build** (debug, no developer-accounting) ([captures](s05-status-tokens/captures/release/)):
  `work · no usage yet` → after a turn `work · 5,576 input · 1,856 cached input · 16 output (13 reasoning)` →
  after the spawned agent the table `work (active) 7,364 / 22,912 / 139 / 60` and `personal 2,578 / 1,856 / 16 /
  13`. These equal the provider-reported usage summed from each thread's rollout
  ([reconcile.txt](s05-status-tokens/captures/reconcile.txt)).
- **Developer-accounting build** ([captures](s05-status-tokens/captures/developer-accounting/)): `/status`
  showed `work (active) 7,770 + 22,336 cached, 108 output` and `personal 4,423, 16 output`; `/cost` at the same
  moment showed `account work … 30,214+ tokens` (7,770 + 22,336 + 108) and `account personal … 4,439 tokens`
  (4,423 + 16). The `+` is one attempt the provider reported no usage for.
- Leak scan: no capture, log or disposable home holds the key in plain text.

Video (leak-scanned, 0 hits): `pf84-status-account-tokens` — see the [demo index](../../demos/index/PF-84-S05.md).

## Known limits

- After a resume, the summary includes the coordinator's earlier work; agents spawned before the resume count
  again only once they run, while `/cost` for that day already includes them.
- A remote app server older than this change sends no `accountUsage`, so the line reads `no usage yet`.
