//! PF-84 (#545): `corbanu resume` resolves the thread it names and the named
//! account that thread ran on, so startup can resume on that account instead
//! of asking for a default key.

use std::path::Path;

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

/// Resolves the session `corbanu resume` names. The picker consumes the app
/// server, as it always has; a later step starts a new one.
pub(crate) async fn select_resume_launch_session(
    tui: &mut Tui,
    cli: &Cli,
    config: &Config,
    app_server: &mut Option<AppServerSession>,
    uses_remote_workspace: bool,
    remote_cwd_override: Option<&Path>,
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
pub(crate) fn resume_account_error(
    config: &Config,
    recorded: &RecordedProviderAccount,
) -> Option<String> {
    // Only when the session runs on the recorded account: `--account` may
    // name another account of the provider, and that one is checked at launch.
    config.model_provider.account.as_ref().filter(|account| {
        account.provider_id == recorded.provider_id && account.name == recorded.account
    })?;
    let message = crate::legacy_core::config::session_account_error(config)?;
    Some(format!(
        "This conversation ran on {} account `{}` and resumes on it: {message}",
        recorded.provider_id, recorded.account
    ))
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

        let message = resume_account_error(&config, &recorded("zai", "work"));

        assert_eq!(
            message.as_deref(),
            Some(
                "This conversation ran on zai account `work` and resumes on it: account `work` \
                 of provider `zai` is not configured; add it with `corbanu account add zai work` \
                 or pick another with `--account` (see `corbanu account list`)"
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_explicit_account_or_another_provider_is_not_the_recorded_account()
    -> std::io::Result<()> {
        // `--account personal` of the same provider replaces the recorded
        // account; launch checks that one, not this.
        let home = TempDir::new()?;
        let config = zai_config(&home, Some("personal"), "zai:work").await?;
        assert_eq!(
            resume_account_error(&config, &recorded("zai", "work")),
            None
        );

        // An account recorded on another provider does not apply to a session
        // that resumes on `zai`.
        let home = TempDir::new()?;
        let config = zai_config(&home, /*explicit*/ None, "kimi:work").await?;
        assert_eq!(config.model_provider.account, None);
        assert_eq!(
            resume_account_error(&config, &recorded("kimi", "work")),
            None
        );
        Ok(())
    }
}
