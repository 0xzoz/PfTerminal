# Aggressive security level: protected files

`/security` → Aggressive runs every agent command in a sandbox that writes only
in the current folder. This page explains the file protections that people ask
about most, and their limits. `/security` shows the same rules before you
confirm the level, and the inspector shows what is in force.

## Credential files

Agent commands can't read `.env`, `.env.*` and `.envrc` files (any letter
case), also ones created after the session started, and no grant lifts this.
To allow it, switch to Permissive.

**Templates are exempt.** `.env.example`, `.env.sample` and `.env.template`
(any letter case) hold no credentials, and checkouts, installs and scaffolds
write them, so agent commands can read, create and change them on every
system. A name such as `.env.example.local` is still a credential file.
Because an agent can make a template, don't rename one into `.env` when
you fill in real values: copy it instead (`cp .env.example .env`). A
template an agent made can be a hard link to a file it reads, and on Linux
and Windows also a link; on macOS an agent command can't create one as a
symbolic link or pipe. A `.env` that is a link or has other names is refused,
as below.

A deny rule of your own for `**/.env.*` in a permission profile still covers
the templates. Only a profile that denies all three of `**/.env`,
`**/.env.*` and `**/.envrc` for a folder, as Aggressive does, leaves them out.

How each system protects files created later:

- **macOS:** agent commands can't read, create, link, rename to or overwrite a
  credential file anywhere. Folders with these names (a `.env` virtualenv) still
  work.
- **Linux:** each command hides the credential files that exist when it
  starts, and a command is stopped when one appears while it runs. Limits:
  - A process that is already waiting for the file can read it in the moment
    between the file appearing and the stop.
  - Only the current folder is covered.
  - Folders on network or FUSE filesystems refuse the command.
- **Windows** (the elevated sandbox; the non-admin sandbox refuses
  these profiles): each command is denied the credential files that exist
  when it starts, and a command is stopped when one appears while it runs
  (also a file saved over a denied one, which gets a new ACL). The same limits
  as on Linux apply: a process already waiting for the file can read it in the
  moment before the stop, and only the current folder is covered. A
  credential file that appears just as a command starts refuses that command;
  run it again. A current folder that is a drive root or on a network or WSL
  share, or holds more than a million files, refuses commands.

## A credential file that is a link

A `.env` (or other credential name) that is a link or a pipe, or on macOS
and Linux a file with other hard links, can't be protected: what you write
to it lands somewhere an agent command could read under another name.
Aggressive refuses to start while the current folder holds one, and on
Linux every command is refused while it exists. The message names the file
and where the link points.

To fix a link, replace it with a regular file (delete the link, then create
the file in its place), or move it out of the workspace. What was written to
it is in the file it pointed to: unless that is a credential file too, agent
commands may have read it, so delete that file too (moving it does not hide it)
and change any secret in it.

For a file with other hard links, delete the other names (`find` with
`-samefile` lists them) or replace the file with a regular copy, and change
any secret in it.

## Grant once

Under Aggressive an approval never lifts the write protections. "Grant once"
lets one command write the files that run code later, which Aggressive
otherwise keeps read-only, and only inside the current folder (the only place
Aggressive lets commands write):

- git hooks and config kept outside a `.git` folder: a hooks folder set with
  `core.hooksPath`, such as Husky's `.husky`, or a git folder with another
  name (`--separate-git-dir`, a `.bare` layout);
- when the current folder holds your home folder: your shell and git start-up
  files (such as `.zshrc`, `.profile`, `.gitconfig`, `.config/git` and
  `.local/bin`), login agents and services (`Library/LaunchAgents`,
  `.config/systemd`, `.config/autostart`) and `.claude` settings.

A file written this way runs code later, outside the sandbox. Nothing else
changes: credential files and Corbanu's sign-in, vault, config and history stay
unreadable; Corbanu's own files and the project's `.git` folder (its hooks and config
included), `.codex` and `.agents` stay read-only; other folders and the
network stay closed.

**When it is not offered.** The approval prompt leaves Grant once out, and
says why, when it could not help: nothing the command writes is something
Grant once lifts, and the command:

- writes outside the current folder, including your own config files
  (`~/.gitconfig` from `git config --global`, `~/.npmrc` from `npm config set`,
  your pip config from `pip config set`, `gh config set`);
- writes the project's `.git`, `.codex` or `.agents`, or Corbanu's files;
- reads a protected file, or uses the macOS keychain (below);
- or shows every file it writes in its text (a plain read, a new link in the
  current folder).

Corbanu decides this from the command's text. When it can't tell what a
program writes (a script, `npm install`), Grant once is still offered, unless
the command also writes a start-up file outside the current folder, the
project's `.codex` or `.agents`, or Corbanu's files. The same reasons are
given where such a command fails.

## The macOS keychain

On macOS the sandbox doesn't let agent commands reach the keychain services.
If you approve `security find-generic-password` or another keychain command,
it runs but can't read, add or change any keychain item: it reports the item
as not found, or an error, and Corbanu says that Aggressive caused it. No
grant lifts this, and `~/Library/Keychains` is unreadable too. To allow it,
switch to Permissive.

This holds in every case, also when your organization's requirements route
agent commands through a network proxy or allow a Unix socket: Aggressive's
sandbox denies the keychain services (`com.apple.SecurityServer`,
`com.apple.securityd.xpc` and `com.apple.securityd.systemkeychain`) after
all its other rules (#758). Contained Claude panes get the same rule under
Aggressive. Certificate checks for secure connections use a different
service (`trustd`) and keep working.

## A separate database folder

Under Moderate and Aggressive, agent commands can't use Corbanu's database folder
when you move it out of the Corbanu home (`sqlite_home` or `CODEX_SQLITE_HOME`).
The whole folder is hidden from them, and so is `<folder>.db-backups` beside it.
If the folder holds the current folder or the Corbanu home, only Corbanu's own
entries in it are hidden.

**On macOS, shared folders are handled the same way.** These folders are shared by
many programs:

- `/tmp` and `/var/tmp`, also as `/private/tmp` and `/private/var/tmp`;
- the temporary folder (`TMPDIR`);
- your home folder itself.

If the database folder is one of these, only Corbanu's own entries are hidden, so
other commands can still use the folder:

- the databases, and any `*.sqlite*` file created later there (so other programs'
  databases at the top of that folder are unreadable to commands too);
- Corbanu's own database names with their journal and `-wal`/`-shm` files, which
  commands can't create either, in any letter case;
- the database init lock (`pfterminal_db_init.lock`) and the GPU lock
  (`gpu-controller.lock`);
- the `accounting-pending` and `db-backups` folders.

These stay protected even before they exist. A folder inside a shared one, such as
`/tmp/corbanu-db`, is still hidden whole.

**On Linux and Windows, a shared folder is still hidden whole.** Their sandboxes
can't stop a command from creating a file under a database's name, so hiding entries
one by one would be weaker there. Commands that need the folder fail under Moderate and
Aggressive.

Don't keep the databases in `/tmp` or `/var/tmp`: other people on the machine can
create files there too.

## Commands run outside the sandbox

Under Moderate, the Corbanu home's runtime files (the sandbox launchers in
`tmp`, shell snapshots, skills, plugins, packages and `AGENTS.md`) stay
read-only to sandboxed commands, even after a person approves a command.
**Exception:** a command that a person approves to run outside the sandbox
(an escalation request, or "retry without sandbox?") can write them, as it
can change anything else. Approve such a command only when you would run it
yourself. Under Aggressive, running outside the sandbox is not offered.
