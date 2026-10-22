# Integrator probe for NA-05 (not a case; not code-blind tester evidence)

Question: does in-session `/resume` of a conversation whose recorded account was removed fall back silently, or does
the TUI's launch `--account` win (NA-05 started the TUI with `--account personal`)? Two attempts, each on its own fresh
clone, same build and harness v4, driven by the integrator through `agms-tui` as the lane user.

- **Attempt 1** ([attempt1-exec-conversation-not-in-picker.txt](attempt1-exec-conversation-not-in-picker.txt)): the
  conversation was made with `corbanu exec --account work`; the TUI picker did not list it ("No results for your
  search"), so nothing was learned.
- **Attempt 2** ([probe-transcript.txt](probe-transcript.txt), [screens.jsonl](screens.jsonl),
  [actions.jsonl](actions.jsonl), [session-accounts.txt](session-accounts.txt), [relay.txt](relay.txt)): conversation
  `qzero` made in the TUI with `--account work` (relay `zwork`); `corbanu account remove zai work`; TUI started
  **without** `--account` (session on `personal` via `[provider_accounts]`); `/resume`, search `qzero`, Enter →
  "■ This conversation ran on zai account `work` and resumes on it: account `work` of provider `zai` is not configured;
  add it or pick another account in `/providers`". The next prompt `qone` ran in the current session on `personal`;
  the `qzero` conversation still records only `work`.

Both transcripts were written to the host run log (`probe-na05.sh` puts its `> probe-transcript.txt` redirect on its
own line, so the file inside the clone folder stayed empty); the copies here come from that log, redacted.
Clone safety files: `clone-*`.
