# PF-84 isolated code-blind acceptance run (pf84-cb1, 2026-10-21/22)

Prerequisite 6 of the [named-accounts flag-removal plan](../../../../docs/plans/proposed/named-accounts-flag-removal.md)
(owner decision D137): a code-blind acceptance run from a frozen design, in isolated VMs, on a build with #1192 and
#1250, with `named_accounts` ON as draft #1080 will make it. It also re-runs the partial #555 legs and #426's two
remaining legs, covers D136 (removing an account while a session runs on it, live, with the free ChatGPT test account)
and the D86/D124/D125 `/status` behaviour. Out of scope by owner decision: live Bedrock (D99), a live refresh saved back
(D135), #1205/#1208 (D138).

**Result: 28 cases: 25 passed (3 of them after the integrator reclassified a result caused by the harness or by a
design premise), 1 not complete, 2 failed.** One product failure blocks: **D136** (after the account is removed, the
running session shows no recovery text and keeps answering on it). **NA-05** failed against its frozen text, but an
integrator probe shows the behaviour follows the documented `--account` precedence, silently; it needs an owner
decision. Per case: [Results](#results).

## Build

- `/Volumes/CorbanuDrive/Corbanu/.codex-work/integration-build-20260922a/integration-package-dev/bin/corbanu.31b9f3ca`,
  copied privately into each guest as `/opt/agms/pkg/bin/corbanu` (read-only, root-owned). SHA-256
  `bbae7d37dff9023268efdd05a8a4408ccf144ee51ba911afd39adc0e9ddce1b8` (checked by the coordinator's probe and by each
  tester; also in every clone's `provision.out`). `codesign --verify --strict` passes; TeamIdentifier `K5N2T25C23`,
  signed 2026-10-21 12:55 (local). `--version`: `corbanu 0.1.48`.
- Source: `main` `31b9f3cad8`, which contains #1192 (`76d99da427`) and #1250 (`d6c15422a9`) (`git merge-base
  --is-ancestor`). The installed binary was not used or changed (the designer/reviewer sessions used it as a client
  only, as every worker does).
- Companion binaries in the guest package (`codex-code-mode-host`, `corbanu-acp`, `corbanu-walletd`, `rg`) are the ones
  staged by the utm-par3 run; [harness/package-files.txt](harness/package-files.txt) lists every digest.
- `named_accounts = true` in every test home's `config.toml` except the two feature-off homes (the candidate's default
  is still off; #1080 flips it).

## Method

### Design (frozen before any run)

- **Designer:** fresh read-only Opus 5.5 High session (`corbanu exec -s read-only`, claude-plan), packet only:
  [design/designer-packet/](design/designer-packet/) (intent written in user terms from the plan, the archived sprint
  "Done" lists, the owner decisions and the #555/#426 gaps; the user doc `docs/provider-accounts.md`; the 2026-10-12
  reference screens; [tools.md](design/designer-packet/tools.md), the test-machine facts). Output:
  [design/designer-output.md](design/designer-output.md).
- **Reviewer:** a separate fresh read-only Opus 5.5 High session with the same packet plus
  [review-facts.md](design/review-packet/review-facts.md) (harness answers to the designer's questions, no product
  answers). `VERDICT: CHANGES`; its revised design is frozen word for word as
  [design/design-cases.md](design/design-cases.md) (SHA-256 `cfad42f5…`, [FROZEN.sha256](design/FROZEN.sha256)); open
  questions in [design/open-questions.md](design/open-questions.md). No integrator edits.
- **Amendment v2** (after the first run; results-informed): the first run found steps that the design could not reach
  (below). A new fresh designer and a separate fresh reviewer, with [run-observations.md](design/amend-v2/run-observations.md),
  wrote [design/amend-v2/amendments-v2.md](design/amend-v2/amendments-v2.md) (frozen from the reviewer's
  `VERDICT: CHANGES`, SHA-256 `fe132491…`). It changes only starting conditions, actions and conventions of NA-01,
  NA-04, NA-10, PR-01, GH-02, NA-05, NA-06, NA-07, ST-02 and ESC-04, and states that no expected result may be weakened.
- Access records: [design/access-record.md](design/access-record.md), [design/amend-v2/access-record.md](design/amend-v2/access-record.md)
  (every command each session ran: packet files only).

### Isolation and execution

- **Machine:** Tart, `TART_HOME=/Volumes/CorbanuDrive/Corbanu/.codex-work/tart-home`, one fresh clone of
  `corbanu-qa-base` (macOS 26.6.2) per group, deleted after it; one VM at a time, holding
  `workers-20261002/vm-tart.lock` (taken after utm-par3 had released it and it had stayed free for 5 minutes). The
  harness is utm-par3's sealed-guest harness (pf: SSH from the host, DHCP and the pinned Z.AI addresses only; DNS
  refused; tester lanes `agmx1..3` reach only the loopback mediator), changed as listed under
  [Harness](#harness-changes-in-this-run). Memory stayed 45–46 % free (`memory-*.txt`).
- **No real key in the guest.** All test-home keys are synthetic (`agms-fake-<label>-<digits>`). The guest mediator
  records the label of the key each product request presents, drops it, and forwards Z.AI requests through an SSH
  reverse tunnel to a host relay that adds the real Z.AI key (fetched with `corbanu vault auth-helper` at relay start,
  never printed). It answers `api.openai.com` itself with 401, and Z.AI requests with no key or a `bad…` label with 401.
- **Live ChatGPT test account (D136), G9 only.** The coordinator ran `corbanu login --account t1 --device-auth` in the
  guest lane; only the device URL and code left the guest, in `workers-20261002/pf84-cb1.signin.md`. Travis approved
  attempt 1 (posted 22:02:44Z, signed in 22:14:54Z: "ChatGPT sign-in · <email-hint> · Free"). ChatGPT traffic went
  through a blind pass-through (host `chatgpt.com:443`/`auth.openai.com:443` only, TLS not opened), so the test
  account's tokens existed only inside the guest. D136 itself removed `t1` ("Signed out and removed openai account
  `t1`"); the coordinator's sign-out step then found it already gone, and the clone's lane homes were deleted
  (0 entries left) before the clone was deleted. Collected evidence never contains vault, key or `auth.json` files.
- **Testers:** one fresh GLM 5.3 Flash (`zai`) `corbanu exec` session per group, in the lane, with the frozen cases and
  `agms-tui` only; each first re-ran the isolation probe and checked both digests.
- **Integrator checks** (not visible to testers): the relay table per group (`relay-product-requests.txt`), conversation
  files, home file lists, and one integrator probe ([integrator-probe-na05/](integrator-probe-na05/)), which is not a
  case and not code-blind tester evidence.
- **Order:** G9 ran first so that the human sign-in fell in working hours; groups are independent (fresh machines).
- **Safety checks per clone:** 0 `SECURITYAGENT-SEEN` lines (no system password prompt); synthetic credential files left:
  0; mediator leaf key and placeholder key file absent (`runs/*/clone-*.txt`). The real login keychain and the
  owner's home were not touched.

### Harness changes in this run

| Version | Change | Clones |
| --- | --- | --- |
| v1 | utm-par3 harness + prepared homes ([agms-prep](harness/guest/agms-prep)), key-label mediator, OpenAI 401, ChatGPT pass-through, `--home`, exit/terminal-settings line | shakedown, G9, G1 |
| v2 | prep writes a baseline file list/hashes (`homes-after-prep.txt`); sign-out no longer stops on an error | G2 + |
| v3 | `agms-tui start --stdin TEXT` (pipe a key) | G3–G8 (unused until amendment v2) |
| v4 | a `C-c` no longer ends the terminal wrapper before the exit line (lost REG-01d's window in G8) | G6R, G8R, R1–R4, probe |

Shakedown (one clone, no tester): every home built, each account kind attributed correctly by the relay, OpenAI 401,
device-code screen reached through the pass-through and cancelled unapproved ([harness/shakedown/](harness/shakedown/)).

## Results

"Run" is the group whose result counts (G = frozen design, R = amendment v2, G6R/G8R = unchanged replays).
Relay/files lines were checked by the integrator; detail in [harness/host/integrator-notes.md](harness/host/integrator-notes.md).

| Case | Covers | Run | Tester | Final | Notes |
| --- | --- | --- | --- | --- | --- |
| NA-01 | AC1, AC6 CLI add/list/remove | R1 | passed | **passed** | G1 blocked by design: `account add` refuses a typed key ("pipe the secret in"). Relay: `xone`→`zextra`; after removal nothing sent. `[observe]` a key given as an argument is echoed by the usage error. |
| NA-02 | AC2 `--account` on exec/TUI | G1 | passed | **passed** | Relay: `work`, `zai:work`→`zwork`; no flag→`zpersonal`; `default`→`zenv`. |
| NA-03 | AC3 missing account | G1 | passed | **passed** | Exit 1 with `corbanu account add zai ghost`; nothing sent; no onboarding. |
| NA-04 | AC3/AC6 rejected key | R1 | passed | **passed** | Two 401s carrying only `badkey`; no fallback. |
| PR-01 | #555 leg, precedence | R2 | passed | **passed** | `/status` and relay at every step: `work`, `personal`, `work`, `personal`, `default`. G2 failed on a premise (see GH-02). `[observe]` Q1: right after a turn on `personal`, a no-flag resume used `personal` (the latest recorded account). |
| GH-02 | #555 leg | R2 | passed | **passed** | After removing `work`, `resume <gid>` exits 1 naming `work` with `corbanu account add zai work`. `[advisory]` `ghost` not mentioned (as in 10-13). |
| NA-05 | AC3/AC4 in-app resume | R2 | failed | **failed (owner decision)** | In a TUI started with `--account personal`, in-app `/resume` of a conversation recorded only on removed `work` resumed it on `personal` with no message (relay `rtwo`→`zpersonal`; session file: turns 1–2 `work`, turn 3 `personal`). Integrator probe: started **without** `--account`, the same resume is refused with REF-10's message. So the launch `--account` wins, as `docs/provider-accounts.md` "Choosing an account" says, but silently. |
| NA-06 | AC1 rows, add, rename | R3 | failed | **not complete** | Rows, kinds, fingerprints and markers passed; add with masked key passed. Rename not exercised: the tester chose "Use for this session" by mistake; the product then rightly refused renaming the in-use account. |
| ESC-03 | #555 leg | G3 | passed | **passed** | Same session, account and transcript after Esc. Draft clause: no route found (`/resume <text>` searches); open question Q2. |
| NA-07 | AC1/AC2/AC3 use/make-default/remove in use | R3 | passed | **passed** | G3 blocked (row 10 hidden, below). |
| NA-08 | AC4 forks | G4 | passed | **passed** | `/fork`, edit, `/side` all `zwork`. |
| NA-09 | AC4 `corbanu fork`, spawned agent | G4 | passed | **passed** | Fork and spawned agent all `zwork`; `fork --account personal`→`zpersonal`. |
| NA-10 | AC5/D86 `/status` format | R1 | failed | **passed (premise)** | One line failed: "Account usage table with `work (active)` and `personal`" after "Use for this session". That action starts a **new** session (its own text: "New session: requests to zai use account `work`"), so a one-row table is consistent; multi-account tables were shown in PR-01/NA-05. G4 blocked (row hidden). |
| ST-01 | D86/D125 no named accounts | G5 | passed | **passed** | Plain `/status` in `zaione`. |
| ST-02 | D124/D86/D125 | R4 | failed | **passed (premise)** | Account added by another process shows on the 2nd `/status` (D124), removed → plain; no `/status` late or stalled. d5-2 "plain" failed only because each "Use … for this session" starts a new session that never used a named account. Tester piped the literal `k` as both keys; the relay shows the request used `extrb`'s stored key (`sha256("k")`). |
| ESC-04 | #555 leg onboarding | R4 | passed | **passed** | Onboarding list and masked Z.AI key screen as REF-12/14; finishing needs scrolling to row 17 "Done". `[observe]` the fresh home then used `glm-5.2`. G5 blocked (Done never reached). |
| NA-11 | AC2/AC3 no default credentials | G6R | passed | **passed** | `work` works without onboarding. G6's tester stopped (model output limit). |
| ESC-01 | #555 leg | G6R | failed | **passed (harness)** | Exit 0, no onboarding, terminal left `icanon isig echo icrnl opost` (the gap: restore now observed directly). The tester failed "echo once": the harness shows a typed line twice (terminal echo + `cat`), which is what a restored terminal does. |
| I426-A | #426 leg 1 | G6R | passed | **passed** | `resume --account ghost <id>`, `zai:ghost`, `--last`: exit 1 with recovery text, no onboarding even at 2 s, nothing sent. |
| NA-12 | AC2/AC4 custom provider | G7 | passed | **passed** | `zpx` default→`zenv`, `work`→`zpxwork`, resume and exec keep `work`. |
| I426-B | #426 leg 2 | G7 | passed | **passed** | Custom provider `--account ghost`: exit 1 with recovery, no picker, no OpenAI sign-in, nothing sent. |
| REG-01a | AC8 flag off | G8 | passed | **passed** | Plain `/status`; `[provider_accounts]` ignored with a warning; `/fork` plain. |
| REG-01b | AC8 flag off resume | G8 | passed | **passed** | Resume of a `work` conversation refused with the feature-off text (raw "Could not resume session…" wording, as #656); `--account zai:default` works. |
| REG-01c | AC8 picker Esc (gap) | G8 | passed | **passed** | Picker reached with the feature off (usable credentials) and Esc behaves as before: closes the gap. |
| REG-01d | AC8 no credentials | G8R | passed | **passed** | Onboarding as on the earlier build; exit 0; terminal normal. G8 lost its window (harness v4 fix). |
| NA-13 | AC6/AC7/REG-02 live ChatGPT | G9 | passed | **passed** | `t1` row "ChatGPT sign-in · <email-hint> · Free · fp …"; `login status` "API key configured (fingerprint …)"; live answer on `t1`; masked API-key entry; no `odefault` request in step 3. Baseline `auth.json` hash taken by the tester at group start equals the end hash; the prep-time hash was not captured (fixed from G2). |
| D136 | owner item | G9 | failed | **failed** | See below. |
| ST-03 | D86/D125 default OpenAI | G9 | passed | **passed** | No `Model provider` or `Account usage` row; `odefault` rejected with a provider-level recovery. |

Totals (final): passed 25 (3 reclassified: ESC-01 harness, NA-10 and ST-02 design premise), not complete 1 (NA-06
rename), failed 2 (D136; NA-05 pending an owner decision). Tester verdicts in the counted runs: 22 passed, 6 failed.

### D136 in detail

Window A ran on `t1` and answered `beforeone`. At 22:24:07Z another terminal ran `corbanu account remove openai t1`
(exit 0, "Signed out and removed …"; the relay shows an `auth.openai.com` exchange at 22:24:08Z); `account list` then
showed no named accounts.

- Passed: no fallback (no `api.openai.com` request carrying `odefault` from 22:23:43Z to 22:28:54Z); no sign-in or
  onboarding screen; `/status` said "Account: account t1 is not signed in"; `/providers` had no `t1` row; nothing
  re-created `t1`; `config.toml` unchanged.
- **Failed:** no recovery text (`corbanu login --account t1`, `/providers`, `--account`) appeared by the last `/status`;
  `/status` kept "Model provider: OpenAI · account t1" and the `t1` usage row.
- `[observe]` (open question Q1): both later prompts (`afterone` 22:24:46Z, `aftertwo` 22:26:33Z) were **answered**,
  over `chatgpt.com` (22:24:48Z, 22:26:35Z): the session kept working on the removed account's sign-in held in memory.

## Defects filed

| Issue | Severity | What |
| --- | --- | --- |
| [#1286](https://github.com/CorbanuCore/CorbanuTerminal/issues/1286) | blocks D136 | After a named account is removed elsewhere, the running session keeps using it and shows no recovery; `/status` still names it. |
| [#1287](https://github.com/CorbanuCore/CorbanuTerminal/issues/1287) | needs decision | In-app `/resume` of a conversation whose recorded account is gone silently re-accounts it to the launch `--account`. |
| [#1288](https://github.com/CorbanuCore/CorbanuTerminal/issues/1288) | advisory | `/providers` (8-row window) and onboarding lists hide rows with no hint; a third account or "Done" is invisible until scrolled. |
| [#1289](https://github.com/CorbanuCore/CorbanuTerminal/issues/1289) | low | Add-account name field: Enter right after fast typing adds a newline; a key pasted next shows in clear in the name field. |
| [#1290](https://github.com/CorbanuCore/CorbanuTerminal/issues/1290) | low | Escape on the `/providers` device-code screen leaves the sign-in waiting (banner stays; the auth connection stayed open until quit). |

## What this run does not show

- A live refresh saved back (D135), Bedrock (D99), the daemon/remote app-server path, Windows (Travis's gate).
- NA-06's rename (tester error); ESC-03's draft clause (no route exists); D124's "vault cannot be read" path.
- `auth.json` unchanged from prep time in G9 (the tester's own start-of-group hash equals the end hash).
- The tester model (GLM 5.3 Flash) slipped several times (wrong menu item, a literal placeholder key, a report in the
  wrong folder); every slip is recorded and none is counted as a product result.

## Review

Independent read-only Opus 5.5 High review of this record: [REVIEW.md](REVIEW.md).
