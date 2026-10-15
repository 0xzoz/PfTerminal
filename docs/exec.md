# Non-interactive mode

For information about non-interactive mode, see [this documentation](https://developers.openai.com/codex/noninteractive).

## When the Corbanu home cannot be read

If the Corbanu home becomes unreadable ("Operation not permitted") during a
run, for example while a drive is briefly blocked, `corbanu exec` waits and
retries with backoff (1 s, doubling to at most 60 s) for up to an hour,
printing each attempt, and continues the turn once access returns. If session
items could not be written meanwhile (they are kept in memory), exec waits for
their folder before it saves the session at exit, within what is left of the
same window (Ctrl-C stops waiting). If access does not return in time, the turn
fails and exec exits 1. Change the window with `--home-wait-secs` (`0` fails at
once).

A `corbanu` command started while the home is unreadable stops at once with
exit status 75 (1 when, on macOS, the process's own sandbox denies the read,
which waiting cannot fix).

Each failure is recorded, without file contents or credentials, in
`~/Library/Logs/Corbanu/home-access.log` on macOS
(`~/.local/state/corbanu/home-access.log` elsewhere): the operation, path and
error, the permissions, flags, ACL and extended-attribute names of the path
and its parents, the mount, the sandbox check, the responsible process and the
working directory.
