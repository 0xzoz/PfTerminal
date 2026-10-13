# #658: whose sign-in the backend receives during a TUI session on a named ChatGPT account

tmux, disposable homes, `CORBANU_TEST_NO_NATIVE_KEYRING=1`, local mock issuer/backend from the s06b acceptance
(`mock.py`), fake tokens only (Travis's login and `auth.json` were never used). Each home has a default ChatGPT login
(mock user `tuidefault`, in the home's own `auth.json`) and, where noted, named accounts. The TUI runs one turn, `/status`,
`/plugins` and `/apps`, then exits (script: `t658.sh`). Counts are every request the mock received during the run, by
path and by whose bearer token it carried. `auth.json` was byte-identical before and after in every run.

## BEFORE: installed dev build (main), `--account work` (named ChatGPT account, mock user `tuiwork`)
```
   2 post-unhandled /backend-api/codex/codex/analytics-events/events           tuidefault
   2 get            /backend-api/codex/connectors/directory/list               tuidefault
   1 get            /backend-api/codex/plugins/featured                        tuidefault
   7 post-unhandled /backend-api/codex/ps/mcp                                  none
  43 get            /backend-api/codex/ps/plugins/installed                    tuidefault
   6 get            /backend-api/codex/ps/plugins/list                         tuidefault
   6 get            /backend-api/codex/ps/plugins/suggested                    tuidefault
   2 get            /backend-api/codex/ps/plugins/suggested                    tuiwork
   2 get            /backend-api/codex/wham/rate-limit-reset-credits           tuidefault
   1 get            /backend-api/codex/wham/settings/user                      tuidefault
   2 get            /backend-api/codex/wham/usage                              tuidefault
   1 get            /v1/models                                                 tuiwork
   1 responses      /v1/responses                                              tuiwork
```

## AFTER: this PR, same run
```
   2 post-unhandled /backend-api/codex/codex/analytics-events/events           tuiwork
   2 get            /backend-api/codex/connectors/directory/list               tuiwork
   1 get            /backend-api/codex/plugins/featured                        tuiwork
   7 post-unhandled /backend-api/codex/ps/mcp                                  none
  41 get            /backend-api/codex/ps/plugins/installed                    tuiwork
   6 get            /backend-api/codex/ps/plugins/list                         tuiwork
   8 get            /backend-api/codex/ps/plugins/suggested                    tuiwork
   2 get            /backend-api/codex/wham/rate-limit-reset-credits           tuiwork
   1 get            /backend-api/codex/wham/settings/user                      tuiwork
   2 get            /backend-api/codex/wham/usage                              tuiwork
   1 get            /v1/models                                                 tuiwork
   1 responses      /v1/responses                                              tuiwork
```

## AFTER, named account `key` holding an OpenAI API key (`--account key`; `UNKNOWN` is that fake key): nothing that needs a ChatGPT sign-in is sent, and nothing goes out on the default login (the featured-plugins list is fetched without credentials, as for a signed-out user)
```
   1 get            /backend-api/codex/plugins/featured                        none
   1 responses      /v1/responses                                              UNKNOWN
```

## AFTER, flag off (`named_accounts = false`, no `--account`): unchanged, everything on the default login
```
   2 post-unhandled /backend-api/codex/codex/analytics-events/events           tuidefault
   2 get            /backend-api/codex/connectors/directory/list               tuidefault
   1 get            /backend-api/codex/plugins/featured                        tuidefault
   7 post-unhandled /backend-api/codex/ps/mcp                                  none
  43 get            /backend-api/codex/ps/plugins/installed                    tuidefault
   6 get            /backend-api/codex/ps/plugins/list                         tuidefault
   8 get            /backend-api/codex/ps/plugins/suggested                    tuidefault
   2 get            /backend-api/codex/wham/rate-limit-reset-credits           tuidefault
   1 get            /backend-api/codex/wham/settings/user                      tuidefault
   2 get            /backend-api/codex/wham/usage                              tuidefault
   1 get            /v1/models                                                 tuidefault
   1 responses      /v1/responses                                              tuidefault
```
