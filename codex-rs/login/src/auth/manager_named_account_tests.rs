//! Runs under the isolated test runner (`CORBANU_TEST_NO_NATIVE_KEYRING=1`).

use std::sync::Arc;

use codex_protocol::config_types::ForcedLoginMethod;
use pretty_assertions::assert_eq;

use super::*;
use crate::auth::named_chatgpt::login_named_account_with_api_key;

fn account(name: &str) -> NamedChatgptAccount {
    NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    }
}

/// The manager itself refuses a sign-in the login restrictions rule out, and
/// a named manager hands default-account providers back to its base.
#[tokio::test]
async fn restricted_sign_ins_never_load_and_the_base_stays_the_default() {
    let home = tempfile::tempdir().expect("home");
    login_named_account_with_api_key(home.path(), &account("work"), "sk-canary-restricted")
        .expect("sign-in");
    let mut base = AuthManager::new(
        home.path().to_path_buf(),
        /*enable_codex_api_key_env*/ false,
        crate::AuthCredentialsStoreMode::File,
        /*forced_chatgpt_workspace_id*/ None,
        /*chatgpt_base_url*/ None,
        crate::AuthKeyringBackendKind::default(),
        crate::test_support::transport_default_auth_route_config(),
    )
    .await;
    base.forced_login_method = Some(ForcedLoginMethod::Chatgpt);
    let base = Arc::new(base);
    let work = base.for_named_account_loaded(account("work")).await;
    assert_eq!(
        (
            work.auth().await.is_none(),
            Arc::ptr_eq(&work.default_account_manager(), &base),
        ),
        (true, true)
    );
}

/// An unreadable stored sign-in can still be signed out (no revoke).
#[tokio::test]
async fn an_unreadable_sign_in_is_still_removed() {
    let home = tempfile::tempdir().expect("home");
    let name = codex_vault::ProviderAccountName::parse("work").expect("name");
    codex_vault::Vault::new(home.path().to_path_buf())
        .write_provider_account(
            "openai",
            &name,
            codex_vault::ProviderAccountKind::ChatgptAuth,
            "not json",
        )
        .expect("write");
    let base = AuthManager::from_auth_for_testing_with_home(
        CodexAuth::from_api_key("sk-default"),
        home.path().to_path_buf(),
    );
    let work = base.for_named_account(account("work"));
    assert_eq!(work.logout_with_revoke().await.expect("logout"), true);
    assert!(
        codex_vault::Vault::new(home.path().to_path_buf())
            .provider_account_kinds("openai", &name)
            .expect("kinds")
            .is_empty()
    );
}

/// #658: the process's backend requests follow the session's account: its
/// named account's sign-in, never the default login, and nothing (not the
/// default) when that account has no usable sign-in.
#[tokio::test]
async fn the_session_manager_follows_the_session_account_and_never_falls_back() {
    let home = tempfile::tempdir().expect("home");
    login_named_account_with_api_key(home.path(), &account("work"), "sk-canary-work")
        .expect("sign-in");
    let base = AuthManager::from_auth_for_testing_with_home(
        CodexAuth::from_api_key("sk-canary-default"),
        home.path().to_path_buf(),
    );
    let key = |manager: Arc<AuthManager>| async move {
        manager
            .auth()
            .await
            .and_then(|auth| auth.api_key().map(str::to_string))
    };
    assert_eq!(
        key(base.for_session()).await.as_deref(),
        Some("sk-canary-default")
    );

    let work = base.for_named_account(account("work"));
    assert!(work.follow_session_account(Some(account("work"))));
    assert!(!base.follow_session_account(Some(account("work"))));
    for manager in [base.for_session(), work.for_session()] {
        assert_eq!(manager.named_account(), Some(&account("work")));
        assert_eq!(key(manager).await.as_deref(), Some("sk-canary-work"));
    }

    assert!(base.follow_session_account(Some(account("ghost"))));
    assert_eq!(key(base.for_session()).await, None);

    assert!(base.follow_session_account(/*account*/ None));
    assert!(Arc::ptr_eq(&base.for_session(), &base));
}
