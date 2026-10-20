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
  `/proc` an orphaned PID 1 is not found. Commands run without bubblewrap
  (full disk write access without managed-proxy routing, legacy Landlock)
  are unchanged, and so is the short `/proc` mount probe, which runs only
  `true`.
- When bubblewrap is active and a protected path inside a writable root is
  missing, the helper makes the empty placeholder bubblewrap mounts over
  (`O_EXCL`, so never over an existing file) and records it, before making
  it, in a per-run file under the Corbanu home
  (`<home>/sandbox-placeholders`, written crash-safely: temporary file,
  sync, rename, sync of the folder) with what it made: device, inode, mode,
  owner, size, modification and change times (#348). Each run also records
  the paths it uses (its placeholders and every empty file or folder it
  mounts at). At the start of each sandboxed run, the records of runs that
  have ended (another boot, or a pid that is gone or was reused) are swept:
  a recorded placeholder is removed only when it is still exactly what was
  made, reached without following a link, no live run's record uses it,
  and it is not a mount point in any of the user's processes (a run that
  left no record). A live run's record this version cannot read keeps
  every placeholder; a run on another machine sharing the home, or in
  another PID namespace, counts as live.
  Anything else (filled, replaced or changed since, or a placeholder whose
  identity was never recorded) is left in place and reported once, on the
  command's standard error and in `kept.log` in that folder. This removes a
  placeholder a killed run left even after the temporary folder's registry
  was lost (a reboot); nothing but a recorded path is ever removed. The
  record folder is bound read-only where a writable root exposes it (as the
  temporary folder's registry is), so the command cannot forge a record;
  under Moderate and Aggressive it is denied outright. A run waits at
  most 10 seconds for the folder's lock, then goes on without the record.
  A placeholder made by a run that then fails before bubblewrap starts is
  recorded and swept like any other. Without
  a usable record folder (a read-only home, as in a nested sandbox) nothing
  is recorded, as before.
- When bubblewrap is active, it mounts a fresh `/proc` via `--proc /proc` by default, but
  you can skip this in restrictive container environments with `--no-proc`.

**Notes**
- The CLI surface is `codex sandbox`; the host OS selects the sandbox backend.
