# PF-84 live ChatGPT check (2026-10-21, D79)

Flag-removal prerequisite 1 of the PF-84 plan: a live check with a disposable ChatGPT account. Travis created a
**free** ChatGPT test account (not his own) and approved the device-code sign-in in his browser. No token, password or
account email appears here; the product's masked email hint is shown as `<email-hint>`.

**Result: PASS for everything exercised; two items not verified** (refresh written back, mid-session removal; see
"Not verified"). One low wording defect filed: #1205.

## Setup

- **Build:** the installed signed build, `corbanu 0.1.48`, main `2f9d05c51e` (INSTALL.json; SHA-256 `a4f9804b…c494`
  matches; Developer ID K5N2T25C23). It is a `dev`-profile build with debug assertions. It includes #1090 (#1085),
  #1096, #1140 (#1129) and #1163 (#1144). It does **not** include #1192 (D86).
- **Home:** a throwaway home under `.codex-work/pf84-live1/`, with `CODEX_HOME`, `CORBANU_HOME` and `PFTERMINAL_HOME`
  pointing at it and `HOME` also pointing to a throwaway directory. Its config set
  `cli_auth_credentials_store = "file"` and `[features] named_accounts = true`. See [scripts/env.sh](scripts/env.sh).
- **Default login:** an OpenAI API key that is fake (never valid), stored in `auth.json`. This is deliberately not the
  test account. A request that fell back to it would fail with `401 invalid_api_key`, so any fallback would show.
- **Named account:** `corbanu login --account t1 --device-auth`. The URL and one-time code went to the coordinator,
  and Travis approved within 2 minutes ([01](captures/01-device-code-signin.txt)).
- **Keychain:**
  - `CORBANU_TEST_NO_NATIVE_KEYRING=1` was set; the guard applies because the build has debug assertions.
  - Every product run went through `sandbox-exec` with [scripts/nokeychain.sb](scripts/nokeychain.sb). The profile
    denies running `/usr/bin/security` and the keychain/security-agent XPC services, so no prompt could appear.
    It allows `com.apple.SecurityServer`, because TLS trust evaluation needs it.
  - The binary ran directly, not through the `~/.local/bin` launcher, and `SSH_CONNECTION` was unset, so the
    launcher's `security unlock-keychain` path never ran.
  - Result: the vault key ended up in the home's file fallback (`secrets/keyring-fallback/*.key`), not a keychain
    item ([04](captures/04-turn-accounts-hashes-vault.txt)).
  - The operator's real home was not touched: its `auth.json` and `secrets/` were unchanged since before the run.
- **Models:** the free plan's catalog for `t1` listed `gpt-6-luna`, `gpt-5.6-terra` and `gpt-5.6-luna`. Turns used
  `gpt-5.6-luna`. The Z.AI switch used GLM 5.3 Flash.

## Results

| # | Check | Result | Evidence |
|---|---|---|---|
| 1 | Device-code sign-in of a named account | PASS. `Signed in OpenAI account t1 (ChatGPT sign-in · <email-hint> · Free)`. `login status` still reports the default API key; `login --account t1 status` and `account list` show `t1`. | [01](captures/01-device-code-signin.txt) |
| 2 | Requests under the account use it | PASS. `exec --account t1 -m gpt-5.6-luna` answered `PF84-LIVE-OK` (the default login is a fake key, so only `t1` could serve it). The config's default model `gpt-6-sol` was refused by the server as "not supported when using Codex with a ChatGPT account", which shows the request went out on the ChatGPT sign-in. The model list was cached per account (`models-cache/openai/t1/`). | [02](captures/02-exec-named-account.txt) |
| 3 | No cross-use | PASS. The same prompt without `--account` went to `api.openai.com` with the fake default key and got `401 invalid_api_key`. `t1` was not used. | [03](captures/03-exec-default-control.txt) |
| 4 | Default login never modified | PASS. `auth.json` had the same SHA-256 before sign-in and after every step, including sign-out (#595/#658/#1085). | [04](captures/04-turn-accounts-hashes-vault.txt) |
| 5 | `/status` shows the account | PASS. `Model provider: OpenAI · account t1`, `Account: ChatGPT sign-in · <email-hint> · Free (account t1)` and `Account usage: t1 · …`. After the first reply on `t1`, the limits came from `t1`. | [resume](captures/tui-status-resume.txt), [switch back](captures/tui-status-switchback.txt) |
| 6 | Resume keeps the account | PASS. The TUI `resume` of the exec thread ran on `t1` (`PF84-RESUME-OK`). | [resume](captures/tui-status-resume.txt), [04](captures/04-turn-accounts-hashes-vault.txt) |
| 7 | Switching keeps the account | PASS. `/model` → Z.AI ran on `zai:default`. Switching back to OpenAI printed "Connectors, usage and the model list now use openai account `t1`" and the next turn ran on `t1`. Wording defect on the Z.AI side: #1205. | [switch back](captures/tui-status-switchback.txt), [04](captures/04-turn-accounts-hashes-vault.txt) |
| 8 | Fork keeps the account | PASS. `corbanu fork <id>`: `/status` shows `OpenAI · account t1` and `Forked from: …`. | [fork](captures/tui-status-fork.txt) |
| 9 | Spawned agent and reload keep the account (#1129/#1140/#1163) | PASS. In the fork, a spawned child showed `gpt-5.6-luna low · account t1` and finished on `t1`. After quitting and resuming the fork, a follow-up task reloaded that child, and it ran on `t1` again: both child turns record `provider_account = t1`. | [spawn](captures/tui-fork-spawn.txt), [reload](captures/tui-resume-child-reload.txt), [04](captures/04-turn-accounts-hashes-vault.txt) |
| 10 | Removing the account refuses, no fallback | PASS. `logout --account t1` revoked and removed the account, and `account list` became empty. Afterwards, `exec resume <id>`, `exec --account t1`, TUI `resume <id>` and TUI `fork <id>` were all refused before any request. Each refusal names `t1` with its recovery (`corbanu login --account t1` or `--account`). None of them reached the default login (no 401). | [resume](captures/tui-resume-after-remove.txt), [fork](captures/tui-fork-after-remove.txt), [10](captures/10-refusals-after-removal.txt) |

`turn_context` per turn: every OpenAI turn in the four rollouts is `openai:…:t1`, and the one Z.AI turn is
`zai:glm-5.3-flash:default` ([04](captures/04-turn-accounts-hashes-vault.txt)).

## Not verified

- **Refresh written back to the account** (part of prerequisite 1). The product refreshes proactively only within
  5 minutes of the access token's expiry, or after 8 days, and the fresh token was nowhere near that. Forcing a refresh
  would mean editing the encrypted vault or the clock. Coverage stays the mock refresh tests from S06.
- **Removal while a session is running.** A second sign-in was offered twice (09:03Z, 09:18Z) for this check. Both
  codes expired unused, so only removal between sessions (row 10) is covered.
- **D86** (`/status` in homes with no named account): #1192 is not in this build. It does not apply to this home,
  which has a named account.
- **App server / daemon (#1085)** was not driven directly. On resume, `/status` showed "Limits: data not available
  yet" until the first reply, which matches the documented hold.

## Observations (not filed)

- The `/model` picker lists OpenAI models that are not in `t1`'s catalog (`gpt-6-sol`, `gpt-6-astra`, `gpt-6.1-sol`,
  `gpt-5.6-sol`). Some of them carry "ChatGPT sign-in may reject it". `exec`'s header names the provider but not the
  account.

## Cleanup

`t1` was signed out (tokens revoked by `logout --account`). The throwaway home and its credential files (`auth.json`,
`secrets/` with the vault and the key file) were deleted, and `ls` confirmed they were gone. The Z.AI key was only an
environment variable of the tmux server, which was then stopped.
