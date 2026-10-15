# PF-60-S09 gate — verification paths for the unverified flows (#402)

Travis's 2026-10-14 decision on #402: verify ChatGPT login with a mock sign-in server only; record TensorCash as not
applicable until access is granted; test image generation and realtime for real on the OpenAI API key (vault
`openai-api-key`, small spend). This sprint builds the paths and runs each once. A FAIL becomes an issue, not a fix
here. The drivers are in [drivers/](drivers/README.md), written so the PF-60-S10 executor can re-run them without
reading source.

Candidate for every run: a developer-accounting debug build of `main` 45916ab64e plus PF-60-S08 (PR #821, code head
`a4ef32068d`). Nothing in S08 touches ChatGPT login, image generation or realtime. All runs used disposable homes with
`CORBANU_TEST_NO_NATIVE_KEYRING=1`; API-key homes were deleted after each run.

## Results

| AC | Flow | Result | Evidence |
| --- | --- | --- | --- |
| AC1 | ChatGPT login (sign-in stand-in) | **PASS (stand-in only)** | Signed in with `corbanu login --device-auth` against the stand-in. The client kept its built-in routes (`auth.openai.com`, `chatgpt.com/backend-api/codex`); only the network path was redirected (CONNECT proxy + throwaway CA). Three turns: catalogued `gpt-5.4`, uncatalogued `gpt-mock-unlisted`, and the `priority` tier. All read `Covered by your subscription (not billed per request)`, with no spend, no "no price" and no bill to check ([catalogued](captures/chatgpt-catalogued-first-screen.txt), [uncatalogued](captures/chatgpt-uncatalogued-first-screen.txt), [priority](captures/chatgpt-priority-tier-first-screen.txt)). Video: [ChatGPT login](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/pf-60-s09-pf60s09-chatgpt-login-cost-a4ef32068d9b-2026-10-14.mp4). |
| AC2 | Image generation, API key | **PASS (real)** | The model called the image tool. The `gpt-image-2` request was recorded as pay per use, with 48 input and 229 output tokens reported. 48 × $5/M + 229 × $30/M = $0.007110, which is `/cost`'s figure exactly ([first screen](captures/image-first-screen.txt), [technical](captures/image-technical.txt)). The video run read 60 + 229 tokens, so $0.007170, again exact. Video: [image cost](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/pf-60-s09-pf60s09-image-cost-a4ef32068d9b-2026-10-14.mp4). |
| AC2 | Image generation, ChatGPT login (stand-in) | **PASS (stand-in only)** | The stand-in requested the image tool. The client sent `chatgpt.com/backend-api/codex/images/generations`, and `/cost` lists `gpt-image-2 — Covered by your subscription` ([first screen](captures/chatgpt-image-first-screen.txt)). |
| AC3 | Realtime, API key | **FAIL — #824** | The driver ran real sessions through `corbanu app-server` ([runs](captures/realtime-driver-runs.txt)). WebSocket: `gpt-realtime-1.5` answered "OK" (billed), but the ledger has no attempt for it, only the thread's warm-up. WebRTC with audio output: the call is recorded as a `realtime:` attempt, but OpenAI refuses it (HTTP 400, "AVAS requires OpenAI-Alpha: quicksilver=v2"). WebRTC with text output: the client refuses before sending. Even where a call is recorded, realtime has no token counts, so nothing can be recomputed. |
| AC4 | TensorCash | **Not applicable (Travis, 2026-10-14)** | Until repository access is granted for the acceptance account. |

## Checks

- No Rust code changed in this sprint, so there are no Rust tests or clippy runs; the drivers are Python, stdlib only.
  `python3 -m py_compile` passes on them, and `make_certs.sh` was run once.
- `python3 docs/plans/check.py`, `python3 docs/sprints/check.py`, `git diff --check`.
- Key scan: the OpenAI key was found in 0 committed or captured files. It appeared only in the disposable homes'
  `auth.json`, and those homes were deleted.
- Spend: three real realtime sessions, two image generations and their GPT-6 Luna turns, under $0.05 in all.

## Observations

- The stand-in serves no apps/MCP endpoint, so a ChatGPT-login session shows "MCP client for `codex_apps` failed to
  start". That's expected with the stand-in.
- The subscription screens say paid usage beyond a plan "isn't visible here … check your provider's usage page". That's
  the overage note, not a bill for the turn.
