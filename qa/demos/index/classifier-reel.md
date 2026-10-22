# When Corbanu asks after untrusted content: demo reel

One narrated video of what changed in how Corbanu decides when to ask after untrusted content (the post-taint
classifier): fewer false prompts (#1172, D85; #1197, #932 phase 3), new git protections (#1158, #1230, #1254 (D153),
#1262, #1273), agents' tasks to panes the person opened (#1220, D106; #1263) and Windows path spellings (#1249). It is
an asset on the [`demos` prerelease](https://github.com/CorbanuCore/CorbanuTerminal/releases/tag/demos).

- Watch online: <https://corbanu-classifier-reel.cloudflare-share.workers.dev/> (published 2026-10-22; subtitles are burned in, and the page also offers them as an English
  text track). Direct MP4: <https://corbanu-classifier-reel.cloudflare-share.workers.dev/video.mp4>, byte-identical to the release asset.
- Video: [classifier-reel-2026-10-21.mp4](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/classifier-reel-2026-10-21.mp4) (5:51, 1920x1080, 30 fps, H.264/AAC, chapters
  embedded) · subtitles: [classifier-reel-2026-10-21.srt](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/classifier-reel-2026-10-21.srt)
- Security levels are behind the default-off `security_levels` flag; the clips run under Moderate.
- Every terminal frame was recorded on 2026-10-21 (local time; 2026-10-22 UTC) for this reel with
  `scripts/demo_video.py` (GLM 5.3 Flash on Z.AI, disposable profiles, synthetic data only), on the installed signed
  build `333f0b8db7` (SHA-256 `29826a1a…`). It is a developer build (Cargo `dev` profile, `developer_marker: true` in
  its INSTALL.json), so `CORBANU_TEST_NO_NATIVE_KEYRING=1`, which `demo_video.py` requires, kept the profiles off the
  macOS keychain. The two "before" clips use the previous signed build `9750861c2a` (SHA-256 `0de41e09…`), which has
  #1172 but not #1197, #1158, #1230, #1254, #1262, #1220 or #1263, recorded with the same isolation. The clips are
  indexed in [CLS-REEL.md](CLS-REEL.md); their specs are `qa/demos/specs/clsreel-*.toml`.
- The two commit clips use Moderate with `sandbox_mode = "danger-full-access"` (and without `secretless_agent_launch` and `secret_output_gate`;
  the former keeps agent commands sandboxed) so the commit can land: under Moderate's usual workspace sandbox the project's
  `.git` is read-only (D43), and the commit would be refused by the sandbox after the same no-prompt decision. The
  agent picks the shell through `exec_command`'s `shell` parameter; a `!grep` of the session log at the end of each
  clip shows `/bin/bash` and `/bin/zsh`.
- Not recorded: #1273 merged after the recorded build (2026-10-22 02:59 UTC), so it is shown from its tests; #1249 is
  Windows-only, so it is shown from its tests and the PR's `rust-ci / Windows native tests` job (success on head
  `bbc31bf4d8`; one unrelated core test, a guardian timeout, passed on Bazel's retry).
- Clips are trimmed to the moment that proves the change; waiting is sped up (an on-screen badge shows the rate) and
  proof frames are held. Added on top: chapter cards, two evidence cards, a side panel naming the issue and the
  criterion, highlights on the proving lines, and burned-in subtitles.
- Narration: Breeze TTS, synthetic voice `corbanu-narrator-calm-v1` (the earlier reels' voice, portable voice library).

## Chapters

| Time | Chapter | Issues / decisions | Source |
| --- | --- | --- | --- |
| 00:00 | Intro | | |
| 00:43 | 1. Fewer false prompts | #1172, D85, #1197, #932 | [`clsreel-commit-before`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-commit-before-9750861c2ab0-2026-10-21.mp4) · [`clsreel-commit-after`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-commit-after-333f0b8db70f-2026-10-21.mp4) |
| 01:59 | 2. New git protections | #1158, #1230, #1254, D153, #1262, #1273 | [`clsreel-1158-git-output`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-1158-git-output-333f0b8db70f-2026-10-21.mp4) · [`clsreel-git-names`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-git-names-333f0b8db70f-2026-10-21.mp4) · evidence card: tests of PR #1273 (merged after the recorded build) |
| 03:29 | 3. Agents' tasks to your panes | #1220, D106, #1263 | [`clsreel-1220-pane-task-before`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-1220-pane-task-before-9750861c2ab0-2026-10-21.mp4) · [`clsreel-1220-pane-task-after`](https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/cls-reel-clsreel-1220-pane-task-after-333f0b8db70f-2026-10-21.mp4) |
| 04:27 | 4. Windows path spellings | #1249, #1240, #691 | evidence card: tests and Windows CI of PR #1249 |
| 05:10 | Summary and what is still open | | |
