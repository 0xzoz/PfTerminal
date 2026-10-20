# codex-linux-sandbox

This crate is responsible for producing:

- a `codex-linux-sandbox` standalone executable for Linux that is bundled with the Node.js version of the Codex CLI
- a lib crate that exposes the business logic of the executable as `run_main()` so that
  - the `codex-exec` CLI can check if its arg0 is `codex-linux-sandbox` and, if so, execute as if it were `codex-linux-sandbox`
  - this should also be true of the `codex` multitool CLI

On Linux, Codex prefers the first `bwrap` found on `PATH`
outside the current working directory whenever it is available. If `bwrap` is
present but too old to support
`--argv0`, the helper keeps using system bubblewrap and switches to a
no-`--argv0` compatibility path for the inner re-exec. If `bwrap` is missing,
the helper falls back to the bundled `codex-resources/bwrap` binary shipped
with Codex.
Codex also surfaces a startup warning when `bwrap` is missing so users know it
is falling back to the bundled helper. Codex surfaces the same startup warning
path when bubblewrap cannot create user namespaces. WSL2 follows the normal
Linux bubblewrap path. WSL1 is not supported for bubblewrap sandboxing because
it cannot create the required user namespaces, so Codex rejects sandboxed shell
commands that would enter the bubblewrap path.

**Current Behavior**
- Legacy `SandboxPolicy` / `sandbox_mode` configs remain supported.
- Bubblewrap is the default filesystem sandbox.
- If `bwrap` is present on `PATH` outside the current working directory, the
  helper uses it.
- If `bwrap` is present but too old to support `--argv0`, the helper uses a
  no-`--argv0` compatibility path for the inner re-exec.
- If `bwrap` is missing, the helper falls back to the bundled
  `codex-resources/bwrap` path.
- If `bwrap` is missing, Codex also surfaces a startup warning instead of
  printing directly from the sandbox helper.
- If bubblewrap cannot create user namespaces, Codex surfaces a startup warning
  instead of waiting for a runtime sandbox failure.
- WSL2 uses the normal Linux bubblewrap path.
- WSL1 is not supported for bubblewrap sandboxing; Codex rejects sandboxed
  shell commands that would require the bubblewrap path before invoking `bwrap`.
- Legacy Landlock + mount protections remain available as an explicit legacy
  fallback path.
- Set `features.use_legacy_landlock = true` (or CLI `-c use_legacy_landlock=true`)
  to force the legacy Landlock fallback.
- The legacy Landlock fallback is used only when the split filesystem policy is
  sandbox-equivalent to the legacy model after `cwd` resolution.
- Split-only filesystem policies that do not round-trip through the legacy
  `SandboxPolicy` model stay on bubblewrap so nested read-only or denied
  carveouts are preserved.
- When bubblewrap is active, the helper applies `PR_SET_NO_NEW_PRIVS` and a
  seccomp network filter in-process.
- When bubblewrap is active, the filesystem is read-only by default via `--ro-bind / /`.
- When bubblewrap is active, writable roots are layered with `--bind <root> <root>`.
- When bubblewrap is active, protected subpaths under writable roots (for
  example `.git`,
  resolved `gitdir:`, and `.codex`) are re-applied as read-only via `--ro-bind`.
- When bubblewrap is active, overlapping split-policy
  entries are applied in path-specificity order so narrower writable children
  can reopen broader read-only or denied parents while narrower denied subpaths
  still win. For example, `/repo = write`, `/repo/a = none`, `/repo/a/b = write`
  keeps `/repo` writable, denies `/repo/a`, and reopens `/repo/a/b` as
  writable again.
- When bubblewrap is active, unreadable glob entries are expanded before
  launching the sandbox and matching files are masked in bubblewrap:

  ```text
  Prefer:   rg --files --hidden --no-ignore --glob <pattern> -- <search-root>
  Fallback: internal globset walker when rg is not installed
  Failure:  any other rg failure aborts sandbox construction
  ```

  Users can cap the scan depth per permissions profile:

  ```toml
  [permissions.workspace.filesystem]
  glob_scan_max_depth = 2

  [permissions.workspace.filesystem.":workspace_roots"]
  "**/*.env" = "none"
  ```

- When bubblewrap is active, symlink-in-path and non-existent protected paths inside
  writable roots are blocked by mounting `/dev/null` on the symlink or first
  missing component.
- When bubblewrap is active, the helper explicitly isolates the user namespace via
  `--unshare-user` and the PID namespace via `--unshare-pid`.
- When bubblewrap is active and network is restricted without proxy routing, the helper also
  isolates the network namespace via `--unshare-net`.
- In managed proxy mode, the helper uses `--unshare-net` plus an internal
  TCP->UDS->TCP routing bridge so tool traffic reaches only configured proxy
  endpoints.
- In managed proxy mode, after the bridge is live, seccomp blocks new
  AF_UNIX/socketpair creation for the user command.
- When the policy carries a `:user_namespaces` deny entry (Corbanu adds it
  under the Moderate and Aggressive security levels, #826), seccomp makes
  `unshare`/`clone` with `CLONE_NEWUSER` fail with `EPERM` and `clone3` with
  `ENOSYS` (libraries fall back to `clone`). Bubblewrap gets the same
  programs through `--add-seccomp-fd`, so its own PID 1 is filtered too, and
  the inner stage installs them again. The command's environment
  carries `CODEX_SANDBOX_USER_NAMESPACES_DENIED=1`; a helper started there
  that cannot create the namespace bubblewrap needs says so and exits 1.
- When bubblewrap is active, no sandboxed process outlives the helper's
  session (#935). bubblewrap binds the sandbox's PID 1 to its parent
  (`--die-with-parent`) only after setup, so a bubblewrap that died during
  setup used to leave the command running on its own. The helper now runs
  bubblewrap under a supervisor: a fork in its own process group, set as a
  child subreaper, so an orphaned PID 1 becomes its child. The supervisor
  kills bubblewrap and every remaining child with `SIGKILL` (for a PID 1
  that is the whole PID namespace, double-forked daemons included), reaps
  them and exits as bubblewrap did, when any of these happens:
  - bubblewrap exits (normally, the command finished);
  - the helper exits or is killed (its lifeline pipe closes);
  - the helper's parent exits (a pidfd on the parent, checked with
    `getppid()` after opening it; kernels without pidfds get
    `PR_SET_PDEATHSIG`, which the helper forwards as `SIGTERM`);
  - a credential file appears (the credential watcher cuts the lifeline).
  This applies on every security level that runs bubblewrap, Permissive
  included. The supervisor lists its children through `/proc`; without
  `/proc` an orphaned PID 1 is not found. The short `/proc` mount probe,
  which runs only `true`, is unchanged.
- Commands run without bubblewrap (full disk write access without
  managed-proxy routing, legacy Landlock) run under the same supervisor
  (#1006), so they cannot leave a process behind either. The helper no
  longer execs the command: the supervisor starts it, restricted as before,
  in the helper's process group and session (the terminal's foreground
  group, if it was), with the signal mask and `SIGCHLD` action the helper
  was started with. Without a PID namespace, the supervisor stops what is
  left by killing its children, which takes in every orphaned process (it is
  a subreaper), and repeating until none is left. It does that when the
  command exits, when the helper exits or is killed (a process-group kill
  included: the supervisor leaves the group once the command is in it) and
  when the helper's parent exits (a pidfd; without one, `PR_SET_PDEATHSIG`
  kills the helper). The helper and the supervisor then exit as the command
  did. Signals:
  - one sent to the helper's process group (as Corbanu sends them) or by
    the terminal reaches the command directly; the helper ignores it;
  - `SIGHUP`, `SIGINT`, `SIGQUIT` and `SIGTERM` sent to the group are
    relayed to the command once it has moved to a process group of its own
    (an interactive shell does), and a terminal hangup's `SIGHUP` (sent to
    the session leader, now the helper) is always relayed;
  - one sent to the helper's pid alone no longer reaches the command, except
    `SIGKILL`, which stops it.
  The command's parent is now the supervisor, and the command is no longer
  its session's or group's leader. This guards against a command that
  daemonizes, not a hostile one: the command runs as the same user, outside
  a PID namespace, and could kill the supervisor itself.
- When bubblewrap is active, it mounts a fresh `/proc` via `--proc /proc` by default, but
  you can skip this in restrictive container environments with `--no-proc`.

**Notes**
- The CLI surface is `codex sandbox`; the host OS selects the sandbox backend.
