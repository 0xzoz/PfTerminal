# #620, #621, #622: Aggressive re-run defects (AGG-18/R17, AGG-R07, AGG-R09)

Found by the "Aggressive ships" re-run (`../../milestones/aggressive-20261011-rerun/`, runs G10, G24, G28).

| Issue | Before (candidate 6ebfa1f47c) | After |
| --- | --- | --- |
| #620 | Grant once offered for `echo '# grant-once-1' >> ~/.zshrc`; accepted, the write still failed with "operation not permitted" | No Grant once offered; the prompt and the denial say why: a start-up file outside the current folder, which agent commands can never write under Aggressive, not even once |
| #621 | `python3 -c "…os.listdir('<home>')…"` offered Grant once; no never-grantable reason | Paths inside interpreter code (`python -c`, `node -e`, `perl -e`, `ruby -e`) are recognised: no Grant once, "cannot be granted, not even once"; a path built at run time is explained from the denial line |
| #622 | Inspector "Recent denials: None" after home-listing denials | Explained sandbox denials (protected read, home listing, start-up file), Aggressive "blocked this command" notices, and additional-permission refusals are listed |

## Why #620 is "no grant" rather than "the write succeeds"

The ~/.zshrc append was not stopped by the start-up-file rule that Grant once lifts. It was stopped by
Aggressive's base sandbox: writes only in the current folder, "approved commands stay inside it too" (the
confirmed Sandbox row; decision 2 of 2026-10-06; #442 decision 9 of 2026-10-12: describe Grant once as what it
actually lifts). The start-up-file rule only applies where the sandbox could write, so a grant could never reach
~/.zshrc. Letting a grant open a file outside the folder would move the sandbox boundary: Decisions for Travis
in the PR.

## Evidence

- Regression tests (fail before, pass after): `core/tests/suite/issue_620_622.rs` (real Seatbelt; Linux bwrap
  on glitch), `security::protected_reads::tests::issue_620_*`, `issue_621_*`.
- tmux (GLM 5.3 Flash, debug build, disposable home and HOME): `after-620.txt`, `after-621.txt`,
  `after-622.txt` (approval prompts without Grant once; the inspector row).
- Videos: specs `qa/demos/specs/issue62{0,1,2}-*.toml`, links in the PR.
