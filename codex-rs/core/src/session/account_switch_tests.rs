//! PF-84: a mid-session switch between a named ChatGPT/OpenAI sign-in
//! account and the default login (or another named account) moves every
//! account-bound service, not only model requests. Runs under the isolated
//! test runner (`CORBANU_TEST_NO_NATIVE_KEYRING=1`).

use std::sync::Arc;

use codex_login::NamedChatgptAccount;
use codex_login::login_named_account_with_api_key;
use codex_model_provider_info::NamedProviderAccount;
use codex_protocol::auth::AuthMode;
use pretty_assertions::assert_eq;

use super::Session;
use super::SessionSettingsUpdate;
use super::tests::build_test_config;
use super::tests::make_session_and_context_for_config;

/// The default login of these sessions (see `make_session_and_context_for_config`).
const DEFAULT_KEY: &str = "Test API Key";
const WORK_KEY: &str = "sk-fake-work-switch-0001";
const PLAY_KEY: &str = "sk-fake-play-switch-0002";

fn account(name: &str) -> NamedChatgptAccount {
    NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    }
}

/// A session on the default OpenAI login whose catalog also has
/// `openai-work` and `openai-play`: OpenAI on named accounts `work` and
/// `play`, each signed in with its own API key in the home's vault.
async fn session_with_accounts() -> (Arc<Session>, tempfile::TempDir) {
    let (session, home) = unshared_session_with_accounts().await;
    (Arc::new(session), home)
}

async fn unshared_session_with_accounts() -> (Session, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let mut config = build_test_config(home.path()).await;
    for (name, key) in [("work", WORK_KEY), ("play", PLAY_KEY)] {
        login_named_account_with_api_key(home.path(), &account(name), key).expect("sign-in");
        let mut provider = config.model_provider.clone();
        provider.account = Some(NamedProviderAccount {
            provider_id: "openai".to_string(),
            name: name.to_string(),
        });
        config
            .model_providers
            .insert(format!("openai-{name}"), provider);
    }
    let (session, _turn_context) = make_session_and_context_for_config(config).await;
    (session, home)
}

async fn switch_to(session: &Session, provider_id: &str) {
    session
        .update_settings(SessionSettingsUpdate {
            model_provider: Some(provider_id.to_string()),
            ..Default::default()
        })
        .await
        .expect("switch provider");
}

fn active_key(session: &Session) -> Option<String> {
    session
        .services
        .auth_manager()
        .auth_cached()
        .and_then(|auth| auth.get_token().ok())
}

/// Connectors (the MCP runtime and its Apps auth) reconnect with the account
/// the session switched to: named, back to the default, and another named one.
#[tokio::test]
async fn connectors_follow_a_mid_session_account_switch() {
    let (session, _home) = session_with_accounts().await;
    session.refresh_mcp_if_dirty().await;
    assert_eq!(active_key(&session).as_deref(), Some(DEFAULT_KEY));

    for (provider_id, key, named) in [
        ("openai-work", WORK_KEY, Some(account("work"))),
        ("openai", DEFAULT_KEY, None),
        ("openai-play", PLAY_KEY, Some(account("play"))),
        ("openai-work", WORK_KEY, Some(account("work"))),
    ] {
        switch_to(&session, provider_id).await;
        let auth_manager = session.services.auth_manager();
        assert_eq!(
            (
                active_key(&session).as_deref(),
                auth_manager.named_account()
            ),
            (Some(key), named.as_ref()),
            "{provider_id}"
        );
        assert!(
            session.mcp_refresh.is_pending(),
            "{provider_id}: connectors were not marked for reconnection"
        );
        session.refresh_mcp_if_dirty().await;
        assert!(
            session
                .services
                .mcp_runtime
                .current_auth_matches(auth_manager.auth_cached().as_ref()),
            "{provider_id}: connectors still use the previous account"
        );
    }
}

/// The sign-in the ledger prices requests under follows the switch (a named
/// API-key account here, the default ChatGPT login before and after).
#[tokio::test]
async fn accounting_follows_a_mid_session_account_switch() {
    let (session, _home) = session_with_accounts().await;
    session.services.active_auth.replace_for_testing(
        codex_login::AuthManager::from_auth_for_testing_with_home(
            codex_login::CodexAuth::create_dummy_chatgpt_auth_for_testing(),
            session.codex_home().await.to_path_buf(),
        ),
    );
    let mode = || crate::accounting_extensions::accounting_auth_mode(&session);
    assert_eq!(mode().await, Some(AuthMode::Chatgpt));

    switch_to(&session, "openai-work").await;
    assert_eq!(mode().await, Some(AuthMode::ApiKey));
    switch_to(&session, "openai").await;
    assert_eq!(mode().await, Some(AuthMode::Chatgpt));
}

/// A switch that keeps the same sign-in (another model of the same
/// provider) leaves the connectors alone.
#[tokio::test]
async fn a_switch_that_keeps_the_account_does_not_reconnect() {
    let (session, _home) = session_with_accounts().await;
    switch_to(&session, "openai-work").await;
    session.refresh_mcp_if_dirty().await;
    let before = session.services.auth_manager();

    let collaboration_mode = session.collaboration_mode().await.with_updates(
        Some("gpt-5.4".to_string()),
        /*effort*/ None,
        /*developer_instructions*/ None,
    );
    session
        .update_settings(SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .await
        .expect("update settings");

    assert!(Arc::ptr_eq(&before, &session.services.auth_manager()));
    assert!(!session.mcp_refresh.is_pending());
}

/// The process's model list (what `model/list` and the picker offer) is
/// fetched and filtered with the account the session switched to.
#[tokio::test]
async fn the_model_list_follows_a_mid_session_account_switch() {
    let (mut session, home) = unshared_session_with_accounts().await;
    let default_auth = codex_login::AuthManager::from_auth_for_testing_with_home(
        codex_login::CodexAuth::create_dummy_chatgpt_auth_for_testing(),
        home.path().to_path_buf(),
    );
    session
        .services
        .active_auth
        .replace_for_testing(Arc::clone(&default_auth));
    let mut config = build_test_config(home.path()).await;
    let only_chatgpt =
        |slug: &str, supported_in_api: bool| codex_protocol::openai_models::ModelInfo {
            supported_in_api,
            ..codex_models_manager::model_info::model_info_from_slug(slug)
        };
    config.model_catalog = Some(codex_protocol::openai_models::ModelsResponse {
        models: vec![
            only_chatgpt("pf84-api-model", true),
            only_chatgpt("pf84-chatgpt-only-model", false),
        ],
    });
    session.services.models_manager =
        crate::thread_manager::build_models_manager(&config, default_auth);
    let session = Arc::new(session);
    let listed = || {
        let mut slugs: Vec<String> = session
            .services
            .models_manager
            .try_list_models()
            .expect("models")
            .into_iter()
            .map(|preset| preset.model)
            .collect();
        slugs.sort();
        slugs
    };
    let both = vec![
        "pf84-api-model".to_string(),
        "pf84-chatgpt-only-model".to_string(),
    ];
    assert_eq!(listed(), both);

    switch_to(&session, "openai-work").await;
    assert_eq!(listed(), vec!["pf84-api-model".to_string()]);
    switch_to(&session, "openai").await;
    assert_eq!(listed(), both);
}
