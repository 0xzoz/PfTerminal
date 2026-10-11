# #442: Travis's decisions of 2026-10-11 (Aggressive)

1. `.env` files **created after** an Aggressive session starts are protected too, on every platform.
2. The project's `.codex` and `.agents` are **always read-only** under Aggressive; "Grant once" no longer lifts them.

(#434 and #440: no change; recorded on the issues.)

## 1. Credential files by name

The Aggressive profile now denies the names `.env`, `.env.*` and `.envrc` (the launch preflight's workspace names,
generalised) as deny globs, in addition to the explicit per-path denials of the files the preflight found:

| Platform | Mechanism | Covers | Limits |
| --- | --- | --- | --- |
| macOS (Seatbelt) | Regex deny rules from the globs `/**/<name>` and `<workspace>/**/<name>` (`file-read*`, `file-write-unlink`) | Any such file anywhere, existing or created later, by any process; links named `.env`; hard links, renames and copies fail | Creating or overwriting such a file is still allowed (it just can't be read back) |
| Linux (bubblewrap) | 1. At every command start, every match below the workspace is masked (glob expansion), plus what a `.env` link points to. 2. From before that scan until the command ends, an inotify watch on every folder below the workspace stops the command (SIGKILL) when such a name is created, linked or moved in, by anyone | Files created between commands (by the agent or the person) and files created while a command runs | Bubblewrap can only mask existing paths, and seccomp/Landlock cannot match names, so a file created during a command is not hidden: the command is stopped instead (fail closed). A process already waiting for the file could read it in the moment before the stop. Only the workspace is covered, not `.env` files elsewhere (macOS covers them). A folder reached through a link inside the workspace is not watched. Watch setup failure or more than 100,000 folders refuses the command; a lost-event overflow stops it. The file is left in place (its creator is unknown) |
| Windows | Unchanged mechanism: globs are expanded to deny ACEs on existing files at each command start | Files that exist when a command starts | Deny ACEs are per object and ACLs cannot match names, so a file created **during** a command inherits the folder's ACL and is readable to that command. Not fixed here (no Windows host to test a watcher); see "For Travis" |

The launch check refuses Aggressive when the profile lacks these rules, and the denial explanation (#464) now covers
such files ("also ones created during the session"). The /security review and inspector say ".env, .env.* and .envrc
files, also ones created later" instead of ".env files found at start".

## 2. `.codex` and `.agents`

Core's protected-path rules keep `<workspace>/.codex` and `.agents` read-only with a grant, like the Corbanu home's
runtime entries. Grant review, inspector ("Agent settings" row) and the /security Sandbox row say so.

Finding: in the real Aggressive profile, `.git`, `.codex` and `.agents` are already read-only through the sandbox's
workspace-metadata protection, which no grant lifts. So on main a confirmed grant did not make `.codex` writable
(tmux check below), but it also does **not** make `.git/hooks` or `.git/config` writable, although the grant review
says it does. Only Core's rules (exercised by the core tests with a plain workspace-write profile) lifted them.

## Evidence

- Regression tests, failing before:
  - macOS, real `sandbox-exec`: `tui security::aggressive::tests::issue_442_credential_files_created_later_stay_unreadable_under_seatbelt`
    (an agent command creates `.env.local`, `app/.env`, `.envrc`; the person creates `.env.production` and
    `../other-project/.env`; the next command reads none, also not via copy, hard link, symlink or rename; Permissive
    still reads it). Before: the synthetic values were printed.
  - `issue_442_launch_check_requires_credential_names` (launch check).
  - Linux, real bwrap on glitch (`linux-sandbox` `issue_442_*`, 3 tests): created by a sandboxed command (stopped, then
    unreadable), created by the person between commands and through a `.env` link, created while a command runs
    (stopped before it reads). Before (watcher off): all three failed, two leaked the value.
  - Core: `pf_25_s01::issue_442_a_confirmed_grant_never_lifts_codex_or_agents` (before: `codex=0 agents=0`),
    `read_denials::issue_442_a_grant_keeps_codex_and_agents_read_only`,
    `pf_23_s01::issue_442_env_file_created_mid_session_is_denied_and_explained`,
    `pf_23_s01::issue_442_permissive_is_unchanged`.
- tmux functional check (GLM 5.3 Flash, disposable home, `CORBANU_TEST_NO_NATIVE_KEYRING=1`, Aggressive after restart),
  [scripts](scripts), [captures](captures):
  - before (main `2e2e2f8d82`): the agent's `.env.local` and the person's `.env.production` were printed.
  - after: both denied with the Aggressive explanation; `.codex` write denied after Grant once; review text updated.
