# PF-84-S06 slice A gate: AWS profile accounts for Amazon Bedrock

Sprint: [PF-84-S06](../../../docs/sprints/current/unified-provider-auth/pf-84-s06-aws-and-chatgpt-accounts.md),
slice A (Travis's D4 option (a), 2026-10-11). Behind the default-off `named_accounts` flag.

## What changed

- **Gate lifted for Bedrock only.** `supports_named_accounts` now accepts the built-in Amazon Bedrock
  provider without an `auth.command`. OpenAI sign-in providers, Bedrock behind a command and other
  AWS-configured providers still fail closed (slice B lifts OpenAI).
- **A1 (storage).** An account holds one `aws_profile`: the profile name, validated
  (`[A-Za-z0-9][A-Za-z0-9._+@-]{0,63}`), not secret, shown in clear in `corbanu account list` and
  `/providers` (`Named account · AWS profile \`work\` · fp …`). Added by
  `corbanu account add amazon-bedrock <name> --kind aws-profile --value <profile>` or the `/providers`
  "Add another account" row (plain entry). `corbanu account rename` is new (any kind).
- **A2 (resolution).** A named Bedrock account signs requests with a profile-only provider
  (`ProfileFileCredentialsProvider`): never the managed Bedrock key, `AWS_BEARER_TOKEN_BEDROCK`,
  `AWS_PROFILE`, environment keys or instance credentials. Region: `aws.region`, else the profile's.
  The profile name is re-read from the vault on every request.
- **A3 (refusal).** Credentials are resolved once before the first request. A missing profile, a
  profile without a region or failing credentials end the turn with the reason and recovery text; no
  request is sent and nothing falls back. An unconfigured account is refused at session start with the
  `--kind aws-profile --value <profile>` command.
- SSO profiles resolve through the SDK (Travis: allowed; no SSO set-up UI).

## Tests (glitch, Linux, `-j10`, isolated runner, `CORBANU_TEST_NO_NATIVE_KEYRING=1`)

| Run | Result |
| --- | --- |
| `cargo clippy --tests -D warnings` on vault, aws-auth, login, model-provider, core, cli, tui | clean, plain and `developer-accounting` |
| `just test`-equivalent `-p codex-vault -p codex-aws-auth -p codex-model-provider -p codex-login` | 374 passed |
| `-p codex-cli --test provider_accounts` | 3 passed |
| `-p codex-core --lib -E 'test(provider_accounts) \| test(account)'` | 115 passed; 120 with `developer-accounting` |
| `-p codex-tui --lib` (named accounts, provider manager, status) | 419 passed, plain and `developer-accounting` |

New tests: profile-name validation; profile-only loading (`work` signs with its own key id, never
`default`; a missing profile errors; a profile without a region is refused; region override);
Bedrock named account ignores the managed key and reports missing profiles with recovery text; the
session-start refusal names `--kind aws-profile`; core stamping (Bedrock accepted, command Bedrock and
custom AWS refused); CLI add/list/rename/remove with validation; `/providers` add method and row text.

## Functional check (tmux, disposable homes)

Script: [slice_a_check.sh](s06/scripts/slice_a_check.sh) with the mock Bedrock Mantle endpoint
[mock_bedrock.py](s06/scripts/mock_bedrock.py), debug build of `714a86715`. Fake `~/.aws` files with
fake key ids only; decoy `AWS_ACCESS_KEY_ID`, `AWS_PROFILE=default` and `AWS_BEARER_TOKEN_BEDROCK` set
for every run. Captures: [s06/slice-a/](s06/slice-a/).

- `exec --account work` was signed with `AKIDWORKPROFILE` (region us-west-2), `--account alt` with
  `AKIDOTHERPROFILE`; the default account still used the bearer token
  ([cli.txt](s06/slice-a/cli.txt), [mock-requests.jsonl](s06/slice-a/mock-requests.jsonl)).
- `--account gone` (profile `nosuchprofile`) failed with the reason and recovery text and sent no
  request; `--account nope` was refused at thread start; with the flag off `--account work` was refused.
- TUI: `/status` shows `· account work`; a turn was signed by `work`'s profile; `/providers` lists the
  Bedrock accounts with profile and fingerprint; "Add another account" took a name and a profile
  ([tui-5](s06/slice-a/tui-5-profile-entry.txt), [tui-6](s06/slice-a/tui-6-saved.txt)); after a
  restart with `[provider_accounts] amazon-bedrock = "prod"` the session ran on `prod`
  ([tui-7](s06/slice-a/tui-7-restart.txt)).
- Leak scan of captures, logs and rollouts for the fake secret keys and the bearer canary: 0 files.

## Not verified

- **Live Bedrock: pending.** No test AWS account or profile is available (Travis, 2026-10-11). Needs an
  AWS account with Bedrock Mantle access in one region, a least-privilege profile and a spend cap.
- A real SSO profile (`aws sso login`) was not exercised; only static-key profiles were.

## Known limits

- Credentials are re-resolved for every request, as on the default SDK path (no cache); for SSO
  profiles that is one SSO call per request.
- Throttle and usage rows are keyed as for other named accounts (by name until the fingerprint-keying
  follow-up lands).
