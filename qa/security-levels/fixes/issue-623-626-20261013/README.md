# #626, #623: Aggressive re-run defects (G11b / AGG-33, G27 / AGG-R13)

Found by the "Aggressive ships" re-run (`../../milestones/aggressive-20261011-rerun/`, runs G11b and G27). Owner
decisions (Travis, 2026-10-13) are on the issues.

| Issue | Before (candidate 6ebfa1f47c) | After |
| --- | --- | --- |
| #626 | A "don't ask again" rule approved under Permissive ran `echo '# expiry-probe' >> ~/.zshrc` unasked under Aggressive | Under Aggressive only rules a person saved while Aggressive was active apply (marked in the Corbanu home's `rules/default.rules`); every other `allow` rule is suspended, not deleted. The person is asked, and the prompt says which remembered approval was not applied and that it applies again at a less strict level. Permissive and Moderate are unchanged |
| #623 | `ln -s .agents ag` created the link (writes through it were refused) | Under Aggressive a command that creates a link to the project's `.codex`/`.agents` (or Corbanu's read-only files) is refused before it runs; nothing is created, and the warning, the model's output and the inspector say why (never granted, not even once) |

## Why #623 is a pre-run check

No OS sandbox can refuse a symbolic link by where it points: Seatbelt filters only by the link's name or type, and
Landlock and seccomp cannot read the target. The check is in core, so it is the same on macOS, Linux and Windows. It
reads `ln`/`gln` (`-s` or hard), `cp -s`/`-l`, `mklink`, PowerShell `New-Item … SymbolicLink|Junction|HardLink` and
`CreateSymbolicLink`, behind shell keywords, operators, wrappers (`sudo`, `env`, `timeout`, `find -exec`, …), in `sh -c`
scripts, heredocs and interpreter code (`os.symlink`, `symlink_to`, `fs.symlinkSync`, `FileUtils.ln_s`, `os.system('ln
…')`). It also covers each program a zsh-fork script starts, and text typed into a running shell or interpreter
(`write_stdin`), one write at a time. A target is refused when its text names a `.codex` or `.agents` folder anywhere
(also by glob or in another letter case), or when it resolves into one, as written or as the system reads it through
links. Too many paths, or lookups past 2 s, refuse.

Out of reach (writes through such a link stay denied by the sandbox): targets built at run time, a script file's links
without zsh-fork, a command typed across several writes, links copied, moved or unpacked (`cp -a`, `mv`, `rsync -l`,
`tar`, `git checkout`), tools not listed (`awk system()`, `Rscript`, PowerShell `-EncodedCommand`), remote
environments. Text that only looks like a link command (a heredoc's documentation) may be refused.

## Evidence

- Regression tests, fail before and pass after (the non-test source reverted; real Seatbelt on macOS and bubblewrap on
  glitch): `core/tests/suite/issue_626.rs`, `core/tests/suite/issue_623.rs`; unit tests
  `exec_policy::aggressive::tests::*`, `security::protected_links::tests::*`.
- tmux replays (GLM 5.3 Flash, debug build, disposable home and HOME): `replay-g11b.txt` (G11b steps: approve "don't ask
  again" under Permissive, switch to Aggressive, same command: asked, with the note; the rule kept) and `replay-g27.txt`
  (AGG-R13 step 6, the `python3 os.symlink` form and an absolute `$PWD` target: each refused, nothing created). Specs:
  `replay-g11b-agg33.toml`, `replay-g27-aggr13.toml`.
- Videos: see the PR and `qa/demos/index/issue-626.md`, `issue-623.md`.
