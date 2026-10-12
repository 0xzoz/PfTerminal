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

## Usage per account in `/status`

With the feature on, `/status` names the session's account on the provider
line and adds an **Account usage** line: the tokens this session used on each
account of the current provider, as the provider reported them. It covers the
whole session: spawned agents (each toward its own account), side
conversations, automatic and `/review` reviews, and memory extraction. Only
token counts are shown: no prices and no cost records.

- One account is one line, such as
  `work · 35,100 input · 12,000 cached input · 6,247 output (300 reasoning)`.
  Before the first reply it reads `no usage yet`.
- Several accounts are a short table with the session's account marked
  `(active)`.
- **Input** is the uncached part, as on the **Token usage** line; cached input
  and reasoning appear only when the provider reports them, and reasoning is
  part of the output. Reviews are counted here but not on **Token usage**, so
  the rows can add up to more.
- Accounts appear by name only. Other providers' accounts are not listed; switch
  to that provider to see them.
- A new session starts from zero. A resumed conversation shows its earlier
  counts again, its spawned agents' included once the TUI reattaches them.
  Memory extraction outside a turn shows with the next count.

Developer-accounting builds also have `/cost`, which shows the same requests by
provider, account and model for one UTC day. For a session that stays within
one day, its token counts per account match these, except for background
memory consolidation, which `/cost` records under the conversation and
`/status` does not show.

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

## OpenAI: ChatGPT and API-key sign-in accounts

A named OpenAI account holds its own sign-in, ChatGPT or OpenAI API key, in the
encrypted vault (no keychain item, never `auth.json`):

```
corbanu login --account work                  # browser sign-in
corbanu login --account work --device-auth    # device code
printenv WORK_OPENAI_KEY | corbanu login --account work --with-api-key
corbanu login --account work status
corbanu logout --account work                 # revokes and clears only `work`
corbanu exec --account work "..."
```

In `/providers`, *Add another account* on OpenAI offers a device-code ChatGPT
sign-in or a masked OpenAI API key. Rows show only the kind, an email hint
(`w…@example.com`), the plan and a fingerprint.

- The default login stays where it is (`auth.json` or the `Codex Auth`
  keychain item) and is never read, refreshed, replaced or logged out by a
  named account. `OPENAI_API_KEY`, `CODEX_API_KEY` and externally provided
  tokens are never used for a named account.
- A refreshed token is saved back to that account only. One process shares
  one sign-in and refresh lock per account; other processes pick a refresh
  up from the vault.
- A session on a named account uses it for what needs the sign-in: model
  requests, refresh, connectors, usage and the `/status` account line. The
  model list is fetched with the account the process started on, and the
  rate limits `/status` shows before the first reply come from the default
  login.
- `forced_login_method` and `forced_chatgpt_workspace_id` apply to named
  accounts too: a sign-in of the other kind or another workspace is refused.
- Signing the same ChatGPT user in under two accounts (or as `default` and a
  named account) is allowed with a warning: they share that user's limits,
  and signing out of one can end the other's session.
- A refresh the server rejects ends the turn with `sign in again with
  corbanu login --account <name>`; the default login is unchanged.
