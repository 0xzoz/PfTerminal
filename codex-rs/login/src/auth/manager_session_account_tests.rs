//! Runs under the isolated test runner (`CORBANU_TEST_NO_NATIVE_KEYRING=1`).

use std::sync::Arc;

use pretty_assertions::assert_eq;

use super::*;
use crate::auth::named_chatgpt::login_named_account_with_api_key;

fn account(name: &str) -> NamedChatgptAccount {
    NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    }
}

async fn key(manager: Option<Arc<AuthManager>>) -> Option<String> {
    manager?
        .auth()
        .await
        .and_then(|auth| auth.api_key().map(str::to_string))
}

/// #658: the process's backend requests use the account its user sessions
/// run on; never the default login while a named account is in use, and
/// nothing when sessions run on different accounts.
#[tokio::test]
async fn the_session_manager_follows_the_sessions_and_never_falls_back_to_the_default() {
    let home = tempfile::tempdir().expect("home");
    login_named_account_with_api_key(home.path(), &account("work"), "sk-canary-work")
        .expect("sign-in");
    login_named_account_with_api_key(home.path(), &account("play"), "sk-canary-play")
        .expect("sign-in");
    let base = AuthManager::from_auth_for_testing_with_home(
        CodexAuth::from_api_key("sk-canary-default"),
        home.path().to_path_buf(),
    );
    let work_manager = base.for_named_account(account("work"));
    assert_eq!(
        key(base.for_session()).await.as_deref(),
        Some("sk-canary-default")
    );

    // A launch whose account is known only when its thread starts: nothing.
    base.register_configured_sign_in(SessionSignIn::Unknown);
    assert!(base.for_session().is_none());
    assert!(base.session_account_conflict().is_some());

    // Before any session: the configured account.
    base.register_configured_sign_in(SessionSignIn::Named(account("work")));
    assert_eq!(
        key(base.for_session()).await.as_deref(),
        Some("sk-canary-work")
    );

    // The first session replaces the configured stand-in; a Z.AI pane does not count.
    let main = work_manager.register_session_sign_in(SessionSignIn::Named(account("work")));
    let zai = base.register_session_sign_in(SessionSignIn::NotOpenAi);
    for manager in [base.for_session(), work_manager.for_session()] {
        let manager = manager.expect("one account");
        assert_eq!(manager.named_account(), Some(&account("work")));
        assert_eq!(key(Some(manager)).await.as_deref(), Some("sk-canary-work"));
    }
    assert_eq!(base.session_account_conflict(), None);

    // A pane on the default login, or another named account: nothing is sent.
    let default_pane = base.register_session_sign_in(SessionSignIn::Default);
    assert!(base.for_session().is_none());
    assert_eq!(base.session_auth().await.map(|_| ()), None);
    assert!(base.session_account_conflict().is_some_and(
        |reason| reason.contains("openai:work") && reason.contains("the default login")
    ));
    default_pane.update(SessionSignIn::Named(account("play")));
    assert!(base.for_session().is_none());
    drop(default_pane);
    assert_eq!(
        key(base.for_session()).await.as_deref(),
        Some("sk-canary-work")
    );

    // A named account without a sign-in yields no auth, not the default.
    main.update(SessionSignIn::Named(account("ghost")));
    assert_eq!(key(base.for_session()).await, None);

    // Back on the default login, and after every session has ended.
    main.update(SessionSignIn::Default);
    assert!(Arc::ptr_eq(&base.for_session().expect("default"), &base));
    drop((main, zai));
    assert!(Arc::ptr_eq(&base.for_session().expect("default"), &base));
}
