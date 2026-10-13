# #613: kill switch follow-ups from the #602 review

Owner decisions: #507 (Travis, 2026-10-12): nothing restarts after the kill switch is turned off; whips and assignments
pause when it goes on and stay paused until `/orchestrate resume`.

| Item | Before | After |
| --- | --- | --- |
| A whip a person attaches, resumes or starts while the switch is on | armed until the next 45 s sweep; could fire right after switch-off | paused again at once, with the "paused: the security kill switch is on" message |
| A Claude-pane task another agent sent while the switch was on, handled after switch-off (event-queue window) | could start a turn | waits for the pane's next turn (the event carries `sent_while_kill_switch_on`) |
| Stop message, kill-switch-on review, picker line | "messages and tasks from other agents wait" | also say a Claude pane keeps only the newest task, and a task for a pane opened in `/panes` is refused instead |
| Test gaps | none for a report to a Codex parent while on, a re-sent same task, the missing-pane sender | `tui` `app::tests::issue_613::*` |
| Claude Plan Orc in a `/spawn` crew vs `agents.provider_allowlist` | investigated | filed as #646 (owner decision needed for an explicit allowlist) |

## Evidence

- Regression tests: `issue_613_a_whip_resumed_while_on_stays_paused` and `issue_613_a_task_sent_while_on_and_handled_after_off_waits` fail with
  the fixes removed and pass with them; the three test-gap tests are guards.
- tmux (GLM 5.3 Flash on zai, debug build of e6668cc9be, disposable home): kill switch turned on during a running command; the review and the stop
  message show the new text. Video: https://github.com/CorbanuCore/CorbanuTerminal/releases/download/demos/issue-613-issue613-kill-switch-texts-e6668cc9be54-2026-10-13.mp4
