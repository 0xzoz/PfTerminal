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
    // OpenAI sign-in cannot hold named accounts yet (pending S06).
    let error = app
        .select_session_provider_account("openai", Some(&name("work")))
        .await
        .expect_err("refused");
    assert!(error.contains("does not support named accounts"), "{error}");
    assert_eq!(app.harness_overrides.provider_account, None);
    assert_eq!(session_account(&app, "openai"), None);
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

/// #572: with a named Bedrock account (an AWS profile) in the vault, a
/// session on another provider is offered Bedrock's models in `/model` (and
/// the provider policy lets them be picked), so an account chosen with *Use
/// for this session* is reachable. Without one, with the feature off, or
/// once the last one is removed, Bedrock stays out of the list as before.
#[tokio::test]
async fn named_bedrock_account_offers_bedrock_models() {
    use super::super::provider_management_status::apply_named_bedrock_models;
    use super::super::provider_management_status::holds_named_bedrock_account;
    use codex_model_provider_info::AMAZON_BEDROCK_PROVIDER_ID;
    let bedrock_models = |app: &App| {
        app.model_catalog
            .try_list_models()
            .unwrap()
            .into_iter()
            .filter(|preset| preset.provider_id.as_deref() == Some(AMAZON_BEDROCK_PROVIDER_ID))
            .collect::<Vec<_>>()
    };
    let (app, home) = app_with_named_accounts("").await;
    app.model_catalog.set_provider_policy(
        crate::chatwidget::provider_model_policy::ProviderModelPolicy::new(
            crate::provider_status_host::ProviderStatusHost::from_config(
                &app.config,
                crate::provider_status_host::ProviderAccountMetadata::default(),
            ),
            codex_provider_auth::ProviderRuntimeAuthorizations::default(),
        ),
    );
    assert!(!holds_named_bedrock_account(&app.config));
    apply_named_bedrock_models(&app.model_catalog, holds_named_bedrock_account(&app.config));
    assert!(bedrock_models(&app).is_empty());

    let vault = codex_vault::Vault::new(home.path().to_path_buf());
    vault
        .write_provider_account(
            AMAZON_BEDROCK_PROVIDER_ID,
            &name("work"),
            codex_vault::ProviderAccountKind::AwsProfile,
            "corp-dev",
        )
        .expect("write account");
    let mut feature_off = app.config.clone();
    feature_off
        .features
        .disable(codex_features::Feature::NamedAccounts)
        .expect("named accounts can be turned off");
    assert!(!holds_named_bedrock_account(&feature_off));
    assert!(holds_named_bedrock_account(&app.config));

    apply_named_bedrock_models(&app.model_catalog, holds_named_bedrock_account(&app.config));
    let offered = bedrock_models(&app);
    assert!(
        offered.iter().any(
            |preset| preset.model == codex_model_provider_info::AMAZON_BEDROCK_GPT_5_5_MODEL_ID
        ),
        "{offered:?}"
    );
    assert!(
        offered
            .iter()
            .all(|preset| app.model_catalog.preset_is_selectable(preset)),
        "{offered:?}"
    );

    vault
        .remove_provider_account(AMAZON_BEDROCK_PROVIDER_ID, &name("work"))
        .expect("remove account");
    apply_named_bedrock_models(&app.model_catalog, holds_named_bedrock_account(&app.config));
    assert!(bedrock_models(&app).is_empty());
}
