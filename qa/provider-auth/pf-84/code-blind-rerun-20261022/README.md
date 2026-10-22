# PF-84 code-blind re-run of D136, NA-05 and NA-06 (pf84-rerun, 2026-10-22)

Re-run of the three cases that the [first code-blind run](../code-blind-20261021/README.md) left failed or
incomplete, on a build with the fixes for owner decisions D171–D175 (#1299, #1300, #1301). The owner approved one
sign-in of the free ChatGPT test account for D136 (D181). Same method as the first run: frozen design, isolated Tart
VMs, GLM 5.3 Flash testers on Z.AI that see only the packet, and integrator checks of the relay and files.

**Result: all three cases passed.**

| Case | Covers | Run | Tester | Final | Integrator checks |
| --- | --- | --- | --- | --- | --- |
| D136 | owner item: account removed while a session runs on it (D171) | S1 | passed | **passed** | See [D136](#d136). No `odefault` request (no `api.openai.com` request at all). No new `chatgpt.com` connection opened between the removal and `/quit`. `auth.json` and `config.toml` unchanged. |
| NA-05 | AC3/AC4 in-app `/resume` (D172 (a)) | S2 | passed | **passed** | `rone`→`zpersonal` 09:05:37Z, `rtwo`→`zpersonal` 09:06:26Z, nothing on `zwork` after the removal. The `<gid>` file records `gzero` and `gaone` on `work`, and `rtwo` on `personal`. |
| NA-06 | AC1 rows, add, rename (D174) | S3R | passed | **passed** | `nren`→`zadded` 09:38:35Z; nothing at `nold`. S3 is not counted (tester error, below). |

Set-up cases in S2 ran with their amendment-v2 text and also passed, matching the first run's R2:
- **PR-01:** relay `pzero` `zwork`, `pone` `zpersonal`, the re-anchor `zwork`, `ptwo` `zwork`, `pthree` `zpersonal`, `pfour` `zenv`. The file records `work, personal, work, work`.
- **GH-02:** `resume <gid>` after removing `work` exits 1, naming `work` with `corbanu account add zai work`. As before, the `[advisory]` that it should mention `ghost` fails.

Evidence for each group is under [runs/](runs/). That includes the tester's `result.md`, the action and screen logs, `relay-product-requests.txt` and `sessions-accounts.txt` (the account each conversation turn records).

## Build

- **Case build:** a private build of `main` `f61397ca41`, the merge of #1299. That commit also contains #1300 (`80d1146019`) and #1301 (`20f438ec28`), and `postmerge-ci` run 39742917339 passed on it.
  - **How it was built:** Rust 1.99.1, `cargo build --locked -p codex-cli --bin corbanu` with the developer-accounting features of `dev-build-online.sh`, in its own target folder.
  - **Signing:** `codesign --force --options runtime --timestamp --identifier com.corbanu.corbanu` with the Developer ID "Travis Good (K5N2T25C23)"; `codesign --verify --strict` passes.
  - **Digest:** SHA-256 `ba9feaa443ec18f29abc73a27137cc8f4696899ea4a1c3dada84c8d2df280006`. Every tester checked it against the package in its VM.
  - **Not installed:** it is copied only into the VMs. The installed binary was not changed.
  - [harness/build.sh](harness/host/build.sh), [harness/package-files-run.txt](harness/package-files-run.txt).
- **Sign-in tool, D136 VM only:** the installed signed build `0e2c02529d` (SHA-256 `3833f804…`). The owner's brief named it for the device-code sign-in, before the case build existed. Its package is listed in [harness/package-files-signin.txt](harness/package-files-signin.txt).
  - The `t1` sign-in and the home it lives in (`oai1`) were made with this build.
  - The case build then ran on that home.
- **Companion files:** the guest package holds only `corbanu`, `codex-code-mode-host` and the resources. That is the acc-tart1 package with `corbanu` replaced. `rg`, `corbanu-acp` and `corbanu-walletd` are not included; no case uses them.
- `named_accounts = true` in every test home, as in the first run.

## Design: amendment v3

The first run's frozen design and its amendment v2 stay in force. Amendment v3 replaces only D136, NA-05 and NA-06.
- **Inputs:** [design-v3/run-observations-v3.md](design-v3/run-observations-v3.md) says what the first run found for those three cases, plus the owner's decisions since (D171–D174), in user terms.
- **Designer:** a fresh read-only Opus 5.5 High session with the packet only, wrote [design-v3/designer-output.md](design-v3/designer-output.md). Its prompt is [prompt.md](design-v3/prompt.md).
- **Reviewer:** a separate fresh session ([review-prompt.md](design-v3/review-prompt.md)) returned `VERDICT: CHANGES`. Its revised text is frozen word for word as [design-v3/amendments-v3.md](design-v3/amendments-v3.md) (SHA-256 `ff365c23…`, [FROZEN.sha256](design-v3/FROZEN.sha256)). Open questions: [open-questions-v3.md](design-v3/open-questions-v3.md).
- **No integrator edits.**
- **Access records:** [design-v3/access-record.md](design-v3/access-record.md). Both sessions read only packet files.
- **What changed:**
  - D136 and NA-05 now expect what decisions 1 and 2 say:
    - D136: refused requests with recovery text, and `account unavailable` in `/status`.
    - NA-05: the conversation opens on the launch account with a one-line notice naming both accounts.
  - NA-06's rename step now makes the tester check that the highlighted line contains "Rename" before pressing Enter.
  - New group plan:
    - S1: D136 alone, in `oai1`.
    - S2: PR-01 → GH-02 → NA-05.
    - S3: NA-06.

The tester packet: the frozen design, amendments v2 and v3, both observation files, `tools.md`, `review-facts.md`,
the reference screens and one prompt per group. File list: [harness/tester-packet/packet-files.sha256](harness/tester-packet/packet-files.sha256). The prompts are the first run's R-group prompts, changed only in the amendment-v3 sentence, the digests and the assignment: [harness/tester-packet/prompts/](harness/tester-packet/prompts/).

## Isolation and execution

The same harness as the first run's last version (v4: `--stdin`, `C-c` keeps the terminal), with these changes ([harness/host-changes.diff](harness/host-changes.diff)):

- **New paths:** new paths and lock holder.
- **New mediation CA:** a new synthetic CA and leaf for the guest mediator, because the first run's leaf key was deleted with its work folder. Their digests are in [harness/guest-files-not-in-cb1-record.sha256](harness/guest-files-not-in-cb1-record.sha256), together with three guest files that the first record did not archive and that came from the same harness lineage (`agms-mcp-notes`, `product-config.toml`, `fixture.tar`).
- **Kept VM for D136:**
  - `signin`: prepares `oai1` and runs the coordinator's device-code sign-in in the clone that D136 later runs in.
  - `pause`: shuts the clone down from inside and stops it, without deleting it.
  - `resume`: boots it again.
  - `reseal`: re-applies pf and checks the loopback DNS responder.
  - `group-continue`: runs the executor in the prepared run folder, without wiping or re-preparing it.
- **Re-stage clears the old stage:** a re-stage now clears the guest's old stage first.
  - The first S1 stage after the resume had kept the sign-in packet's files next to the new ones.
  - That attempt was stopped before any tester started.
  - The stage was cleared and repeated, and the second stage held only the run packet ([runs/S1/clone-gate-out.txt](runs/S1/clone-gate-out.txt) is from the repeated gate).
- **Watcher:** if the first device code expired unapproved, the watcher stopped the sign-in script. No new code was to be posted before 16:30Z (owner instruction). It was not needed.

Other facts of the run:

- **One VM at a time:** each group ran in one Tart clone of `corbanu-qa-base`, with `workers-20261002/vm-tart.lock` held.
  - The lock was taken at 08:15Z.
  - The D136 clone was kept, stopped, from 08:25 to 08:35Z. The lock stayed held and no other VM ran.
  - S2, S3 and S3R each got a fresh clone.
  - Every clone was deleted at the end, and the lock was released at 09:41Z.
- **Memory:** 38–44 % free throughout.
- **No real key in any guest:** a host relay added the Z.AI key (vault label `zai_key_3`, fetched at relay start, never printed).
- **ChatGPT traffic:** a blind pass-through carried the ChatGPT traffic (`chatgpt.com:443`, `auth.openai.com:443`), and only for the D136 VM. The test account's tokens existed only inside that guest.
- **Sign-in (D181):**
  - The coordinator relayed the device code from `workers-20261002/pf84-rerun.signin.md`, which held the URL and code only.
  - The code was posted at 08:20:40Z. Per the coordinator, it was approved with the free test account at 08:24:14Z: "Signed in OpenAI account `t1` (ChatGPT sign-in · <email-hint> · Free)". See [runs/S1/signin.txt](runs/S1/signin.txt), code redacted.
  - `t1` was removed by D136 itself. The sign-out step then found it gone. Every lane home was deleted before the clone was deleted.
- **Safety checks per clone:**
  - 0 `SECURITYAGENT-SEEN` lines, so no system password prompt.
  - 0 synthetic credential files left.
  - The mediator leaf key and the placeholder key file were absent.
  - See `runs/*/clone-*`. The real login keychain and the owner's home were not touched.

## D136

Window A started on `t1` at 08:42:57Z with the case build. That was about 18 minutes after the sign-in, and the clone had been paused for 10 of them.

- **Before the removal:** `beforeone` was answered at 08:43:11Z, and x0 showed `OpenAI · account t1`.
- **Removal:** at 08:43:31Z another terminal ran `account remove openai t1`. It exited 0 with "Signed out and removed openai account `t1`." The relay shows the revoke exchange with `auth.openai.com` at 08:43:31–32Z. `account list` then showed no named accounts.
- **x1:** x1-1 (08:44:03Z) still showed `· account t1`, a stale reading that D124 allows. x1-2 (08:44:12Z) showed `OpenAI · account unavailable`, with the Account row "account `t1` has no sign-in: `corbanu login --account t1` or `/providers`".
- **The two prompts after the removal:** `afterone` (08:44:28Z) and `aftertwo` (08:46:07Z) were both refused, with no reply:

  ```text
  ■ Fatal error: OpenAI: account `t1` has no sign-in (it may have been removed or signed out); sign in with `corbanu
  login --account t1` (add `--with-api-key` for an API key), or pick another account (`/providers` in the TUI,
  `--account` on the command line)
  ```
- **x3 (08:47:49Z):** `account unavailable`, the recovery on the Account row, and Account usage still listing `t1 · 13,193 input · 6 output`.
- **Afterwards:** `/providers` had no `t1` row, the final `account list` had no `t1`, and there was no sign-in or onboarding screen.
- **Relay:**
  - There was no `api.openai.com` request in the group, so no fallback to the default login (`odefault`).
  - `chatgpt.com` traffic at `beforeone` (08:43:09–12Z).
  - `[advisory]` no-use window: from step 4 (08:43:50Z) to `/quit` (08:48:31Z), **no new connection** to `chatgpt.com` or `auth.openai.com` was opened. The last one opened at 08:43:32Z, right after the removal command.
  - Two connections opened at session start (08:42:58Z and 08:43:00Z) stayed open through the window. They closed at 08:45:59Z (idle, 15 KB up in total) and at `/quit` (45 KB up in total).
  - The pass-through logs only total bytes per connection, so it cannot show that nothing at all crossed those two after the removal. The totals fit the pre-removal traffic, and the refusal text shows the product refused before sending.
  - The first run, by contrast, opened new `chatgpt.com` connections at `afterone` and `aftertwo`, which were then answered.
- **Files:**
  - `auth.json` SHA-256 `6b2807a4…` is the same right after prep and at the end.
  - `config.toml` is unchanged (`9ee73afc…`) and has no `openai = "t1"` entry.
  - The vault files at the end are `.vault.lock` and `local.age`.
  - `[observe]` The conversation file records a turn on `t1` for `beforeone`, and also for the refused `afterone` and `aftertwo` ([runs/S1/sessions-accounts.txt](runs/S1/sessions-accounts.txt)).

## NA-05

The TUI was started with `--account personal` in `zai3`, right after GH-02, which had removed `work` and set `zai = "ghost"`. In-app `/resume` of `<gid>`, recorded only on `work`, showed:

> • This conversation ran on zai account `work`, which is no longer available; it resumes on `personal`, the account
> selected for this session (with `--account` or in `/providers`).

r2 then showed `<gid>` and `· account personal`. Its Account usage listed `personal (active)` and `work` with that conversation's earlier counts. `rtwo` was answered on `personal`, and the earlier turns `gzero` and `gaone` were visible. Relay and files checks: see the table above.

## NA-06

- **S3 (not counted, tester error):** the tester opened `added`'s actions ([runs/S3/screens.jsonl](runs/S3/screens.jsonl), `n6-actions`) and took `n6-rename-hl` with the highlight still on "1. Start a new session on this account".
  - It then pressed Enter in the same command. The action log ([runs/S3/actions.jsonl](runs/S3/actions.jsonl)) shows no `Down` key between the two screens.
  - Its report says it pressed `Down` twice and that Rename was highlighted. Both the screen and the action log contradict that.
  - The product did what the highlighted action says: a new session on `added`.
  - As amendment v3's fail rules allow, the integrator classifies this as a tester error. S3 was replayed unchanged as S3R in a fresh clone, as G6R and G8R were in the first run.
- **S3R (counted):**
  - The rows for `work` and `personal` show a kind and a fingerprint. `personal` carries "this session · default for new sessions".
  - The add, with a masked key, saved `added` with a fingerprint.
  - The actions read "Start a new session on this account", "Make default", "Rename" and "Remove" (D174).
  - Two `Down` presses highlighted "3. Rename". Enter opened "Rename Z.AI account `added`", and `renamed` was saved: "Renamed zai account `added` to `renamed`".
  - `account list` shows `renamed` and no `added`. `exec --account renamed` printed `nren`, and `exec --account added` exited 1 with the not-configured text.
- **`[observe]` survey:**
  - S3's list had 17 rows: 6 providers, the three Z.AI rows, 6 more providers, Ollama and LM Studio. It showed 8 at a time, with no hint, and wrapped at the end.
  - In S3R, the tester saw 15 rows: no Ollama or LM Studio rows, and no wrap. Its survey stopped at the 30-press limit.
  - Same build and the same prepared home; the difference was not investigated. The survey is observe-only.

## Other observations

- **Large downloads at TUI start (advisory, also in the first run).** At TUI start with the ChatGPT account, `chatgpt.com` sent about 38 MB four times in parallel (08:42:58–08:43:26Z), and about 24 MB once more (08:43:10–08:43:33Z). The home's remote plugin catalog cache is 41.5 MB. The first run's G9 shows the same four 38 MB transfers. Filed as #1324 (advisory); it does not affect PF-84's criteria.
- `[observe]` Refused turns are still written to the conversation file as turns on the removed account (D136 above).

## Remaining

Travis's Windows gate (a separate agent runs it) and sign-off; then #1080.
