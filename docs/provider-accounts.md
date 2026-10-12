# Named provider accounts (`named_accounts`, off by default)

One key per provider is not enough when you keep a work and a personal account,
or want a worker on a different account than its coordinator. Named accounts
let a provider hold several accounts besides `default`, each a named key or
token in the vault (`corbanu account add <provider> <name>`, `corbanu account
list`). Turn the feature on with `--enable named_accounts` or
`[features] named_accounts = true`.

## Choosing an account

For a provider, the first of these wins: an explicit `--account` for that
provider (or a spawn's `account`), the account a resumed thread recorded (or a
worker's parent runs on), `[provider_accounts] <provider> = "<name>"`,
`default`.

- `--account <name>` selects an account of the provider you launch with. On
  `resume`, write `<provider>:<name>` if the thread runs on another provider.
- `--account <provider>:<name>` for **another** provider selects that
  provider's account for whatever in the session runs on it: spawned agents
  and TUI workers given that provider, and a TUI switch to it. The session's
  own provider keeps its own selection. `corbanu exec` cannot switch
  providers, so it refuses such an account when no spawned agent can run on
  that provider (`agents.enabled = false`, or `agents.provider_allowlist`
  excludes it). It cannot see the model's catalog, so with both
  `multi_agent` features off it accepts the account even when the model
  enables no spawn tool.
- `--account default` always means today's credentials, also with the feature
  off.

## Fail closed

A missing account is an error, never the default account. The session
provider's account and the account `--account` names (of any provider) are
checked before the session starts; another provider's `[provider_accounts]`
entry is checked when something starts on that provider:

```
account `ghost` of provider `zai` is not configured; add it with
`corbanu account add zai ghost` or pick another with `--account`
(see `corbanu account list`)
```

`corbanu exec` and resume fail at thread start. The TUI exits with this text
before any sign-in screen, also when resuming (choose the account with
`--account`); with a remote app server, the server refuses at thread start.
A resumed thread runs on its recorded account; with the feature off, the
resume is refused and names the recorded account (`--account
<provider>:default` resumes on the default credentials). A rejected key names the account and
its `corbanu account add` recovery; the default credential is unchanged.

## Amazon Bedrock: AWS profile accounts

A Bedrock account stores only the name of an AWS profile (not secret, shown in
`corbanu account list` and `/providers`; up to 64 letters, digits and `-_.+@`,
starting with a letter or digit):

```
corbanu account add amazon-bedrock work --kind aws-profile --value corp-dev
corbanu account rename amazon-bedrock work prod
corbanu exec --account prod "..."
```

Its requests are signed with that profile's credentials from `~/.aws/config` and
`~/.aws/credentials` (or the files `AWS_CONFIG_FILE` and
`AWS_SHARED_CREDENTIALS_FILE` name), and nothing else: not the managed Bedrock
key, `AWS_BEARER_TOKEN_BEDROCK`, `AWS_PROFILE`, `AWS_ACCESS_KEY_ID` or
instance or container credentials (a profile whose `credential_source` asks for
them is refused). The region is `model_providers.amazon-bedrock.aws.region`
if set, else the profile's `region`. SSO profiles work once you have run
`aws sso login --profile <p>` yourself. A profile that is missing, has no
region or whose credentials fail is refused with that reason before any
request; Corbanu never falls back to another profile. The `default` account
keeps today's order (managed key, then `AWS_BEARER_TOKEN_BEDROCK`, then the
AWS SDK chain).
