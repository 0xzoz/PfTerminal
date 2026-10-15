# Non-interactive mode

For information about non-interactive mode, see [this documentation](https://developers.openai.com/codex/noninteractive).

## When the Corbanu home cannot be read

If the Corbanu home becomes unreadable ("Operation not permitted"), for
example while a drive is briefly blocked, `corbanu exec` waits and retries
with backoff (1 s, doubling to at most 60 s) for up to an hour, printing each
attempt, and continues the turn once access returns. If access does not return
in time, it fails and exits 1. Change the window with `--home-wait-secs`
(`0` fails at once). A denial by the process's own sandbox is not waited out.

Each failure is recorded, without file contents or credentials, in
`~/Library/Logs/Corbanu/home-access.log` on macOS
(`~/.local/state/corbanu/home-access.log` elsewhere): the operation, path and
error, the permissions, flags, ACL and extended-attribute names of the home and
its parents, the mount, the sandbox check, the responsible process and the
working directory.
