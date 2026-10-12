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
- **Windows:** a file created while a command runs is readable to that command;
  commands that start later can't read it.

## A credential file that is a link

A `.env` (or other credential name) that is a link, a pipe, or a file with
other hard links can't be protected: what you write to it lands somewhere an
agent command could read under another name. Aggressive refuses to start
while the current folder holds one, and on Linux every command is refused
while it exists. The message names the file and where the link points.

To fix it, replace the link with a regular file (delete the link, then copy the
file it pointed to into its place), or move it out of the workspace. A file
with other hard links: copy it to a new file and rename that over it.

## Grant once

Under Aggressive an approval never lifts the write protections. "Grant once"
lifts one: it lets one command write your shell and git start-up files (such
as `.zshrc`, `.profile`, `.gitconfig`, `.config/git` and `.local/bin`). It
applies only where Aggressive lets commands write, the current folder, so it
only matters when that folder holds them (for example, Corbanu runs in your
home folder). A start-up file written this way runs code later, outside the
sandbox.

Nothing else changes: credential files and the Corbanu home stay unreadable;
Corbanu's own files and the project's `.git`, `.codex` and `.agents` stay
read-only; other folders and the network stay closed.

## Commands run outside the sandbox

Under Moderate, the Corbanu home's runtime files (the sandbox launchers in
`tmp`, shell snapshots, skills, plugins, packages and `AGENTS.md`) stay
read-only to sandboxed commands, even after a person approves a command.
**Exception:** a command that a person approves to run outside the sandbox
(an escalation request, or "retry without sandbox?") can write them, as it
can change anything else. Approve such a command only when you would run it
yourself. Under Aggressive, running outside the sandbox is not offered.
