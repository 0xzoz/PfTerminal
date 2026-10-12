# #545: resume on the recorded named account

Disposable home with `CORBANU_TEST_NO_NATIVE_KEYRING=1`, `named_accounts` on, GLM 5.3 Flash (`zai`), no default Z.AI key.
Accounts `work` and `personal` hold the same vault key, which was used only to enrol them.
`pong` was recorded on `work`, `ping` on `personal`.
Before: `5b2dfbbada` (main). After: this branch.

| Case | Before | After |
| --- | --- | --- |
| `corbanu resume --last` | API-key screen (`before-resume-last.txt`) | resumes `ping`, a new turn answers, `/status` shows `account personal` (`after-resume-last-status.txt`) |
| `corbanu resume` (picker) | API-key screen before the picker (`before-resume-picker.txt`) | picker first; `ping` resumes (`after-picker-ping.txt`); Esc starts a new session, which needs a key, so onboarding follows (`after-picker-esc.txt`) |
| `/resume` in a session on `[provider_accounts] zai = "work"` | `ping` runs on `personal`, but `/status` says `account work` (`before-inapp-resume-status.txt`) | `/status` shows the resumed thread's account; `/new` is back on `default` (`after-inapp-resume-status.txt`) |
| `work` removed, `resume <pong id>` | API-key screen (`before-missing-account.txt`) | exits and names `work` with `corbanu account add zai work` (`after-missing-account.txt`); the picker does the same (`after-picker-pong.txt`) |
| `work` removed, `/resume` → `pong` | — | error in chat; stays on the current thread (`after-inapp-missing.txt`) |
| `resume <pong id> --account personal` | — | explicit account wins: `/status` shows `account personal` |

Regression tests: `codex-tui` `suite::resume_recorded_account` (tmux). Both fail on the before binary (`CARGO_BIN_EXE_corbanu=<before>`) and pass after.
Video: `qa/demos/index/ISSUE-545.md`.
