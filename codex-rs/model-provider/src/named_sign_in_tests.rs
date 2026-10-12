//! PF-84-S06 slice B: a provider on a named OpenAI account uses only that
//! account's sign-in, and a provider on the default account never borrows a
//! named one (a session swapped to a named manager switching providers).
//! Runs under the isolated test runner (`CORBANU_TEST_NO_NATIVE_KEYRING=1`).

use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::NamedChatgptAccount;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::NamedProviderAccount;
use pretty_assertions::assert_eq;

use crate::provider::ModelProvider;
use crate::provider::create_model_provider;

fn openai(account: Option<&str>) -> ModelProviderInfo {
    let mut info = ModelProviderInfo::create_openai_provider(/*base_url*/ None);
    info.account = account.map(|name| NamedProviderAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    });
    info
}

#[tokio::test]
async fn named_and_default_openai_providers_never_mix_sign_ins() {
    let home = std::env::temp_dir().join(format!("pf84-s06-mix-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    let work = NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: "work".to_string(),
    };
    codex_login::login_named_account_with_api_key(&home, &work, "sk-work-canary")
        .expect("named sign-in");
    let base = AuthManager::from_auth_for_testing_with_home(
        CodexAuth::from_api_key("sk-default-canary"),
        home.clone(),
    );
    let key = |auth: Option<CodexAuth>| auth.and_then(|auth| auth.api_key().map(str::to_string));

    let named = create_model_provider(openai(Some("work")), Some(base.clone()));
    let swapped = base.for_named_account(work);
    let default_on_swapped = create_model_provider(openai(None), Some(swapped));
    let missing = create_model_provider(openai(Some("missing")), Some(base));
    assert_eq!(
        (
            key(named.auth().await),
            key(default_on_swapped.auth().await),
            key(missing.auth().await),
        ),
        (
            Some("sk-work-canary".to_string()),
            Some("sk-default-canary".to_string()),
            None,
        )
    );
}
