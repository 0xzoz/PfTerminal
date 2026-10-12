# #555: forks and side conversations follow the conversation's named account

Disposable home with `CORBANU_TEST_NO_NATIVE_KEYRING=1`, `named_accounts` on, GLM 5.3 Flash (`zai`), no default Z.AI key,
`[provider_accounts] zai = "personal"`. Accounts `work` and `personal` hold the same vault key, which was used only to enrol them.
Script: a session on `--account work` answers `ping`; `corbanu resume --last` (recorded on `work`); `/fork`; the fork answers
`forked`; `/status`; `/side reply with the single word side`; `/status`.
Before: `e7be21a1a9` (main). After: this branch.

| Case | Before | After |
| --- | --- | --- |
| `/fork` of a conversation on `work` | `/status` says `account work`, but the fork's tokens are on `personal` (`fork-before-fork-status.txt`) | the fork runs on `work`; `/status` says `account work` and only `work` has usage (`fork-after-fork-status.txt`) |
| `/side` from that fork | side tokens also on `personal` (`fork-before-side-status.txt`) | side conversation on `work` (`fork-after-side-status.txt`) |

Regression tests:
- `codex-app-server` `suite::v2::provider_account_inheritance`: a fork of a loaded thread and of a stored (not loaded) thread runs
  on the source thread's account (both fail on main: they ran on `[provider_accounts]`); an explicit fork account wins.
- `codex-tui` `suite::resume_recorded_account::tmux_fork_runs_on_the_recorded_account`: `corbanu fork <id>` shows `account work`
  (fails on main's binary: `account personal`).
