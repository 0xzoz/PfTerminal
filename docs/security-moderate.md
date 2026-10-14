# Moderate security level

`/security` → Moderate protects you from instructions hidden in files, tool
output and web or MCP content, and keeps secrets away from agent commands,
while normal work continues. It keeps your own approval, sandbox, network and
web search settings and adds the protections below. The review in
`/security` lists the same rows before you confirm, and the inspector (`i`)
shows what is in force.

Moderate needs the untrusted-content labels (the `source_envelopes` feature):
without them Corbanu refuses every model request under Moderate, so
`/security` doesn't save Moderate until the feature is on.

## What Moderate adds

- **Commands stay in the sandbox.** No agent command runs outside your
  sandbox, even when you approve a request to run outside it. "Retry without
  sandbox?" isn't offered, and commands you allowed with "don't ask again"
  skip the prompt but stay in the sandbox.
- **Protected files.** Agent commands can't read the credential files in your
  home folder (such as `.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.netrc`,
  `.npmrc` and `~/Library/Keychains`) or Corbanu's sign-in, vault, config,
  history and state. They can't write files that run code later: your shell
  and git start-up files, git hooks and config, the project's `.codex` and
  `.agents`, and Corbanu's own files. Your fresh approval of one command
  lifts this for that command, except for Corbanu's runtime files and what
  `secretless_agent_launch` keeps; a remembered or automatic approval never
  does. A project's `.env` files aren't covered; Aggressive covers them.
- **Untrusted content.** Tool, MCP, web, agent and memory output reaches the
  model labelled untrusted. After it, protected actions need your fresh
  approval: the vault, credentials, security settings, disclosure, value
  transfer, code you haven't seen, MCP calls that change or send data, and
  tools Corbanu hasn't classified. Cached, remembered or automatic approvals
  don't count, and with approvals off these actions are refused.
- **Model keys.** From the next start, provider API keys are held by the
  credential broker, as under Aggressive. `broker_model_auth = false` in your
  own config keeps it off.
- **Child agents.** Agents started in this Corbanu Terminal (`/spawn` and the
  agent tools) run with Moderate enforced. Claude panes are refused, because
  Claude Code runs outside the sandbox, unless `contained_external_agents` and
  `secretless_agent_launch` run them in it.

Not available yet: financial preview and approvals, isolated retrieval,
screened search, brokered browser login, quarantine, the untrusted-content
detectors and Agent Sweep.

## Choosing, restarting and going back

When you confirm Moderate, Core's rules (the sandbox, protected files and
untrusted content) apply at once in every session of this Corbanu Terminal,
and grants and "for session" approvals end. The credential broker starts at
the next start; press `r` to restart right away. Your `config.toml` isn't
changed.

Choosing Permissive or Aggressive from Moderate shows what changes first.
Back to Permissive takes effect when you restart; until then Moderate stays
on. Aggressive applies Core's stricter rules at once and the rest at the
restart. The kill switch (`/security` → `g`) works under every level.

## Limits

- If your own settings turn the sandbox off (full access), commands that the
  protected-file rules don't cover run without one. Choose a sandbox in your
  settings to keep every command in it.
- MCP servers, apps, hooks and `!` shell commands run outside the sandbox, as
  under every level.
- An agent command can start `corbanu exec` or `review`, and Moderate doesn't
  refuse it, but inside the sandbox the new agent can't read Corbanu's config,
  sign-in or security state, so it stops at start (with
  `secretless_agent_launch` on, also when you approve the command).
- A remote app server can't enforce a level chosen here; Corbanu refuses to
  start against one until you choose Permissive.

See [Aggressive security level: protected files](security-aggressive.md) for
the protected-file rules both levels share.
