//! PF-84 (#545, #555): `corbanu resume` resolves the thread it names and the
//! provider and named account that thread ran on, so startup can resume on
//! that account instead of asking for a default key.

use std::path::Path;

use codex_model_provider::AccountRecovery;

use crate::Cli;
use crate::app_server_session::AppServerSession;
use crate::legacy_core::config::Config;
use crate::resume_picker;
use crate::session_resume::RecordedProviderAccount;
use crate::tui::Tui;

/// What `corbanu resume` (an id, `--last` or the picker) selected.
pub(crate) enum ResumeLaunchSelection {
    Selected(resume_picker::SessionSelection),
    /// No saved session matches this id or name.
    Missing(String),
}

pub(crate) fn is_resume_launch(cli: &Cli) -> bool {
    cli.resume_session_id.is_some() || cli.resume_last || cli.resume_picker
}

/// PF-84 (#555): a resume or fork runs on the account its conversation
/// recorded, which beats `[provider_accounts]`. So unless `--account` names
/// the session provider's account, that provider's account is checked once
/// the thread is chosen (see [`chosen_thread_account_error`]), not at launch.
pub(crate) fn defers_launch_account_check(cli: &Cli, config: &Config) -> bool {
    let continues_a_thread =
        is_resume_launch(cli) || cli.fork_picker || cli.fork_last || cli.fork_session_id.is_some();
    let explicit_for_session_provider = config
        .provider_account_override
        .as_deref()
        .and_then(|selection| selection.split_once(':'))
        .is_some_and(|(provider_id, _)| provider_id == config.model_provider_id);
    continues_a_thread
        && !explicit_for_session_provider
        && config
            .features
            .enabled(codex_features::Feature::NamedAccounts)
}

/// PF-84 (#426, #555): the account check before onboarding. A deferred check
/// still refuses a missing `--account` of another provider (#425).
pub(crate) fn launch_account_error(config: &Config, deferred: bool) -> Option<String> {
    if deferred {
        crate::legacy_core::config::other_provider_account_error(config)
    } else {
        crate::legacy_core::config::session_account_error(config)
    }
}

/// PF-84 (#545, #555): the account check once the thread is chosen: the
/// account it recorded, else the deferred launch check.
pub(crate) fn chosen_thread_account_error(
    config: &Config,
    recorded: Option<&RecordedProviderAccount>,
    deferred: bool,
) -> Option<String> {
    match recorded {
        Some(recorded) => resume_account_error(config, recorded, AccountRecovery::CommandLine),
        None if deferred => crate::legacy_core::config::session_account_error(config),
        None => None,
    }
}

/// Resolves the session `corbanu resume` names. The picker consumes the app
/// server, as it always has; a later step starts a new one.
pub(crate) async fn select_resume_launch_session(
    tui: &mut Tui,
    cli: &Cli,
    config: &Config,
    app_server: &mut Option<AppServerSession>,
    uses_remote_workspace: bool,
    remote_cwd_override: Option<&Path>,
    picker_context: resume_picker::SessionPickerLaunchContext,
) -> color_eyre::Result<ResumeLaunchSelection> {
    if let Some(id_str) = cli.resume_session_id.as_deref() {
        let Some(startup_app_server) = app_server.as_mut() else {
            unreachable!("app server should be initialized for --resume <id>");
        };
        let target =
            crate::lookup_session_target_with_app_server(startup_app_server, id_str).await?;
        return Ok(
            match crate::explicit_resume_selection(
                config.codex_home.as_ref(),
                id_str,
                target,
                uses_remote_workspace,
            ) {
                Some(selection) => ResumeLaunchSelection::Selected(selection),
                None => ResumeLaunchSelection::Missing(id_str.to_string()),
            },
        );
    }
    if cli.resume_last {
        let filter_cwd = crate::latest_session_cwd_filter(
            uses_remote_workspace,
            remote_cwd_override,
            config,
            cli.resume_show_all,
        );
        let Some(app_server) = app_server.as_mut() else {
            unreachable!("app server should be initialized for --resume --last");
        };
        let target = crate::lookup_latest_session_target_with_app_server(
            app_server,
            config,
            filter_cwd,
            cli.resume_include_non_interactive,
        )
        .await?;
        return Ok(ResumeLaunchSelection::Selected(match target {
            Some(target_session) => resume_picker::SessionSelection::Resume(target_session),
            None => resume_picker::SessionSelection::StartFresh,
        }));
    }
    let Some(app_server) = app_server.take() else {
        unreachable!("app server should be initialized for --resume picker");
    };
    Ok(ResumeLaunchSelection::Selected(
        resume_picker::run_resume_picker_with_app_server(
            tui,
            config,
            cli.resume_show_all,
            cli.resume_include_non_interactive,
            app_server,
            picker_context,
        )
        .await?,
    ))
}

/// PF-84 (#545): the named account the selected thread ran on, which the app
/// server resumes it on, and a fork of it runs on (#555), unless `--account`
/// names another account of the same provider. `None` for a fresh session, a
/// remote workspace, or while named accounts are off (the app server then
/// refuses a recorded account).
pub(crate) async fn resume_recorded_provider_account(
    selection: &resume_picker::SessionSelection,
    config: &Config,
    uses_remote_workspace: bool,
) -> Option<RecordedProviderAccount> {
    let (resume_picker::SessionSelection::Resume(target)
    | resume_picker::SessionSelection::Fork(target)) = selection
    else {
        return None;
    };
    if uses_remote_workspace
        || !config
            .features
            .enabled(codex_features::Feature::NamedAccounts)
    {
        return None;
    }
    crate::session_resume::read_recorded_provider_account(target.path.as_deref()?).await
}

/// PF-84 (#545): why the thread cannot resume on the account it ran on, with
/// the same recovery as the account line, instead of asking for a default key.
/// `recovery` is `Providers` inside a running TUI (#555).
pub(crate) fn resume_account_error(
    config: &Config,
    recorded: &RecordedProviderAccount,
    recovery: AccountRecovery,
) -> Option<String> {
    // The thread resumes on the provider it recorded (#555), which can be
    // another provider than the session's.
    let provider = if recorded.provider_id == config.model_provider_id {
        &config.model_provider
    } else {
        config.model_providers.get(&recorded.provider_id)?
    };
    // Only when the thread runs on the recorded account: `--account` may
    // name another account of the provider, and that one is checked at launch.
    provider.account.as_ref().filter(|account| {
        account.provider_id == recorded.provider_id && account.name == recorded.account
    })?;
    let message = codex_model_provider::selected_account_error_with_recovery(
        config.codex_home.as_path(),
        provider,
        recovery,
    )?;
    Some(format!(
        "This conversation ran on {} account `{}` and resumes on it: {message}",
        recorded.provider_id, recorded.account
    ))
}

/// PF-84 (#555): the config loader applies the account a resumed thread
/// recorded only to the session's own provider. A thread recorded on another
/// provider resumes there, so that provider's entry carries the account too;
/// the TUI's view of the thread then names the account it runs on.
pub(crate) fn apply_inherited_account_to_its_provider(
    config: &mut Config,
    inherited_provider_account: Option<&str>,
) {
    let Some(selection) = inherited_provider_account else {
        return;
    };
    if let Err(error) =
        crate::legacy_core::config::apply_recorded_provider_account(config, selection)
    {
        tracing::warn!(%error, selection, "could not apply the resumed thread's account");
    }
}

/// PF-84 (#555): `config` as a thread recorded on another provider resumes:
/// on that provider, with its recorded account and `model`. `None` when the
/// thread runs on the session's provider or this home lacks its provider.
pub(crate) fn recorded_provider_config(
    config: &Config,
    recorded: &RecordedProviderAccount,
    model: Option<String>,
) -> Option<Config> {
    if recorded.provider_id == config.model_provider_id {
        return None;
    }
    let provider = config.model_providers.get(&recorded.provider_id)?.clone();
    let mut thread_config = config.clone();
    thread_config.model_provider_id = recorded.provider_id.clone();
    thread_config.model_provider = provider;
    if model.is_some() {
        thread_config.model = model;
    }
    Some(thread_config)
}

/// PF-84 (#555): why a thread recorded on another provider cannot resume
/// when this home has no credentials for that provider, instead of opening
/// the session provider's onboarding.
pub(crate) fn missing_provider_credentials_error(recorded: &RecordedProviderAccount) -> String {
    let provider_id = &recorded.provider_id;
    format!(
        "This conversation ran on {provider_id} account `{}` and resumes on it: provider \
         `{provider_id}` has no credentials in this home; set them up with `corbanu -c \
         model_provider=\"{provider_id}\"` (or in `/providers`) and resume again, or resume on \
         a named account with `--account {provider_id}:<name>` (see `corbanu account list`)",
        recorded.account
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_core::config::ConfigBuilder;
    use crate::legacy_core::config::ConfigOverrides;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    async fn zai_config(
        home: &TempDir,
        explicit: Option<&str>,
        inherited: &str,
    ) -> std::io::Result<Config> {
        std::fs::write(
            home.path().join("config.toml"),
            "model = \"glm-5.3-flash\"\nmodel_provider = \"zai\"\n\n[features]\nnamed_accounts = true\n",
        )?;
        ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .harness_overrides(ConfigOverrides {
                provider_account: explicit.map(str::to_string),
                inherited_provider_account: Some(inherited.to_string()),
                ..Default::default()
            })
            .build()
            .await
    }

    fn recorded(provider_id: &str, account: &str) -> RecordedProviderAccount {
        RecordedProviderAccount {
            provider_id: provider_id.to_string(),
            account: account.to_string(),
        }
    }

    #[tokio::test]
    async fn missing_recorded_account_is_named_with_its_recovery() -> std::io::Result<()> {
        let home = TempDir::new()?;
        let config = zai_config(&home, /*explicit*/ None, "zai:work").await?;

        let message = resume_account_error(
            &config,
            &recorded("zai", "work"),
            AccountRecovery::CommandLine,
        );

        assert_eq!(
            message.as_deref(),
            Some(
                "This conversation ran on zai account `work` and resumes on it: account `work` \
                 of provider `zai` is not configured; add it with `corbanu account add zai work` \
                 or pick another with `--account` (see `corbanu account list`)"
            )
        );
        // Inside the TUI (`/resume`) the recovery is `/providers` (#555).
        assert_eq!(
            resume_account_error(
                &config,
                &recorded("zai", "work"),
                AccountRecovery::Providers
            )
            .as_deref(),
            Some(
                "This conversation ran on zai account `work` and resumes on it: account `work` \
                 of provider `zai` is not configured; add it or pick another account in \
                 `/providers`"
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_explicit_account_is_not_the_recorded_account() -> std::io::Result<()> {
        // `--account personal` of the same provider replaces the recorded
        // account; launch checks that one, not this.
        let home = TempDir::new()?;
        let config = zai_config(&home, Some("personal"), "zai:work").await?;
        assert_eq!(
            resume_account_error(
                &config,
                &recorded("zai", "work"),
                AccountRecovery::CommandLine
            ),
            None
        );

        // The same for an explicit account of the thread's other provider.
        let home = TempDir::new()?;
        let mut config = zai_config(&home, Some("kimi-code:personal"), "kimi-code:work").await?;
        apply_inherited_account_to_its_provider(&mut config, Some("kimi-code:work"));
        assert_eq!(
            resume_account_error(
                &config,
                &recorded("kimi-code", "work"),
                AccountRecovery::CommandLine
            ),
            None
        );
        Ok(())
    }

    /// PF-84 #555: a thread recorded on another provider resumes on that
    /// provider's recorded account, which the session's config now names.
    #[tokio::test]
    async fn a_thread_on_another_provider_carries_its_recorded_account() -> std::io::Result<()> {
        let home = TempDir::new()?;
        let mut config = zai_config(&home, /*explicit*/ None, "kimi-code:work").await?;
        // The loader applies an inherited account only to the session's provider.
        assert_eq!(config.model_providers["kimi-code"].account, None);

        apply_inherited_account_to_its_provider(&mut config, Some("kimi-code:work"));
        assert_eq!(
            config.model_providers["kimi-code"]
                .account
                .as_ref()
                .map(|account| account.name.as_str()),
            Some("work")
        );
        assert_eq!(config.model_provider.account, None);
        assert_eq!(
            resume_account_error(
                &config,
                &recorded("kimi-code", "work"),
                AccountRecovery::CommandLine
            )
            .as_deref(),
            Some(
                "This conversation ran on kimi-code account `work` and resumes on it: account \
                 `work` of provider `kimi-code` is not configured; add it with `corbanu account \
                 add kimi-code work` or pick another with `--account` (see `corbanu account list`)"
            )
        );

        let thread_config = recorded_provider_config(
            &config,
            &recorded("kimi-code", "work"),
            Some("kimi-for-coding".to_string()),
        )
        .expect("kimi-code is a built-in provider");
        assert_eq!(
            (
                thread_config.model_provider_id.as_str(),
                thread_config.model.as_deref(),
                thread_config
                    .model_provider
                    .account
                    .as_ref()
                    .map(|account| account.name.as_str()),
            ),
            ("kimi-code", Some("kimi-for-coding"), Some("work"))
        );
        // A thread on the session's own provider needs no other config.
        assert!(
            recorded_provider_config(&config, &recorded("zai", "work"), /*model*/ None).is_none()
        );
        Ok(())
    }
}
