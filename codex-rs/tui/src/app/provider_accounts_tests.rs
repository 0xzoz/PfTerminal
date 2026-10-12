use super::*;
use crate::app::test_support::make_test_app;
use crate::test_support::PathBufExt;
use pretty_assertions::assert_eq;
use tempfile::TempDir;
use tempfile::tempdir;

fn name(raw: &str) -> ProviderAccountName {
    ProviderAccountName::parse(raw).expect("valid name")
}

async fn app_with_named_accounts(extra_config: &str) -> (App, TempDir) {
    let mut app = make_test_app().await;
    let home = tempdir().expect("home");
    std::fs::write(
        home.path().join("config.toml"),
        format!("[features]\nnamed_accounts = true\n{extra_config}"),
    )
    .expect("config");
    app.config.codex_home = home.path().to_path_buf().abs();
    app.refresh_in_memory_config_from_disk()
        .await
        .expect("load config");
    (app, home)
}

fn session_account(app: &App, provider_id: &str) -> Option<String> {
    app.config.model_providers[provider_id]
        .account
        .as_ref()
        .map(|account| account.name.clone())
}

#[tokio::test]
async fn a_session_account_that_cannot_load_changes_nothing() {
    let (mut app, _home) = app_with_named_accounts("").await;
    // A local server has no credential, so no named accounts (OpenAI sign-in
    // can hold them since S06).
    let error = app
        .select_session_provider_account("ollama", Some(&name("work")))
        .await
        .expect_err("refused");
    assert!(error.contains("does not support named accounts"), "{error}");
    assert_eq!(app.harness_overrides.provider_account, None);
    assert_eq!(session_account(&app, "ollama"), None);
}

#[tokio::test]
async fn one_session_selection_covers_one_provider() {
    let (mut app, _home) = app_with_named_accounts("").await;
    assert_eq!(
        app.select_session_provider_account("zai", Some(&name("work")))
            .await,
        Ok(None)
    );
    assert_eq!(session_account(&app, "zai").as_deref(), Some("work"));

    // Another provider's selection is never silently dropped.
    let error = app
        .select_session_provider_account("openrouter", Some(&name("alt")))
        .await
        .expect_err("refused");
    assert!(
        error.contains("already selects zai account `work`"),
        "{error}"
    );
    assert_eq!(
        app.harness_overrides.provider_account.as_deref(),
        Some("zai:work")
    );

    // Back to the default account, then another provider is free.
    assert_eq!(
        app.select_session_provider_account("zai", None).await,
        Ok(Some("zai:work".to_string()))
    );
    assert_eq!(session_account(&app, "zai"), None);
    assert!(
        app.select_session_provider_account("openrouter", Some(&name("alt")))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn rename_carries_the_default_along() {
    let (mut app, home) = app_with_named_accounts("[provider_accounts]\nzai = \"work\"\n").await;
    codex_vault::Vault::new(home.path().to_path_buf())
        .write_provider_account(
            "zai",
            &name("work"),
            codex_vault::ProviderAccountKind::ApiKey,
            "pf84-s04-rename-canary",
        )
        .expect("write");
    let message = app
        .rename_provider_account("zai", &name("work"), &name("home"))
        .await
        .expect("renamed");
    assert_eq!(
        message,
        "Renamed zai account `work` to `home`. It stays the default for new sessions."
    );
    let config = std::fs::read_to_string(home.path().join("config.toml")).expect("config");
    assert!(config.contains("zai = \"home\""), "{config}");
    assert_eq!(session_account(&app, "zai").as_deref(), Some("home"));
}

#[tokio::test]
async fn a_default_set_by_a_higher_layer_is_reported() {
    let (mut app, _home) = app_with_named_accounts("").await;
    app.cli_kv_overrides.push((
        "provider_accounts.zai".to_string(),
        toml::Value::String("work".to_string()),
    ));
    let error = app
        .persist_default_provider_account("zai", None)
        .await
        .expect_err("outranked");
    assert!(error.contains("another config layer"), "{error}");
}

/// PF-84: only a model switch that changes the sign-in account refreshes the
/// model list (and says so); one between models of the same account does not.
#[test]
fn only_a_sign_in_account_change_is_a_switch() {
    use crate::app::event_dispatch::account_switch_message;
    use crate::app::event_dispatch::sign_in_account_switch;
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::NamedProviderAccount;

    let openai = ModelProviderInfo::create_openai_provider(/*base_url*/ None);
    let mut work = openai.clone();
    work.account = Some(NamedProviderAccount {
        provider_id: "openai".to_string(),
        name: "work".to_string(),
    });
    let zai = codex_model_provider_info::built_in_model_providers(/*openai_base_url*/ None)["zai"]
        .clone();
    let work_account = codex_login::NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: "work".to_string(),
    };

    assert_eq!(
        sign_in_account_switch(&openai, &work),
        Some(Some(work_account.clone()))
    );
    assert_eq!(sign_in_account_switch(&work, &zai), Some(None));
    assert_eq!(sign_in_account_switch(&work, &work), None);
    assert_eq!(sign_in_account_switch(&openai, &zai), None);
    assert_eq!(
        account_switch_message(Some(&work_account)),
        "Connectors, usage and the model list now use openai account `work`; model list refreshed."
    );
    assert_eq!(
        account_switch_message(None),
        "Connectors, usage and the model list now use the default account; model list refreshed."
    );
}

/// PF-84: only `Confirmed { Applied }` counts as applied before the reply;
/// an uncertain outcome or a plain acceptance does not refresh the list.
#[test]
fn only_an_applied_confirmation_counts() {
    use codex_app_server_protocol::ThreadSettingsUpdateOutcome;
    use codex_app_server_protocol::ThreadSettingsUpdateResponse;

    assert!(crate::app_server_session::applied_before_reply(
        &ThreadSettingsUpdateResponse::Confirmed {
            outcome: ThreadSettingsUpdateOutcome::Applied,
        }
    ));
    assert!(!crate::app_server_session::applied_before_reply(
        &ThreadSettingsUpdateResponse::Confirmed {
            outcome: ThreadSettingsUpdateOutcome::Uncertain,
        }
    ));
    assert!(!crate::app_server_session::applied_before_reply(
        &ThreadSettingsUpdateResponse::Accepted {}
    ));
}
