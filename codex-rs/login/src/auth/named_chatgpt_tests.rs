use std::sync::Arc;

use codex_keyring_store::tests::MockKeyringStore;
use codex_protocol::auth::AuthMode;
use pretty_assertions::assert_eq;

use super::*;

fn account(name: &str) -> NamedChatgptAccount {
    NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    }
}

fn api_key_login(key: &str) -> AuthDotJson {
    AuthDotJson {
        auth_mode: Some(AuthMode::ApiKey),
        openai_api_key: Some(key.to_string()),
        tokens: None,
        last_refresh: None,
        agent_identity: None,
        personal_access_token: None,
        bedrock_api_key: None,
    }
}

#[test]
fn each_account_reads_and_writes_only_its_own_label() {
    let home = tempfile::tempdir().expect("home");
    let vault = Vault::new_with_keyring_store(
        home.path().to_path_buf(),
        Arc::new(MockKeyringStore::default()),
    );
    let work = VaultAccountAuthStorage::with_vault(vault.clone(), &account("work")).expect("work");
    let play = VaultAccountAuthStorage::with_vault(vault.clone(), &account("play")).expect("play");
    assert_eq!(work.load().expect("empty"), None);

    work.save(&api_key_login("sk-canary-work")).expect("save work");
    play.save(&api_key_login("sk-canary-play")).expect("save play");
    assert_eq!(
        (work.load().expect("work"), play.load().expect("play")),
        (
            Some(api_key_login("sk-canary-work")),
            Some(api_key_login("sk-canary-play"))
        )
    );
    // The default account's file store is never written.
    assert!(!home.path().join("auth.json").exists());

    assert!(work.delete().expect("delete work"));
    assert_eq!(
        (work.load().expect("gone"), play.load().expect("kept")),
        (None, Some(api_key_login("sk-canary-play")))
    );
    assert!(!work.delete().expect("already gone"));
}

#[test]
fn an_unreadable_sign_in_never_quotes_its_value() {
    let home = tempfile::tempdir().expect("home");
    let vault = Vault::new_with_keyring_store(
        home.path().to_path_buf(),
        Arc::new(MockKeyringStore::default()),
    );
    let name = ProviderAccountName::parse("work").expect("name");
    vault
        .write_provider_account(
            "openai",
            &name,
            ProviderAccountKind::ChatgptAuth,
            "sk-canary-not-json",
        )
        .expect("write");
    let storage = VaultAccountAuthStorage::with_vault(vault, &account("work")).expect("storage");
    let error = storage.load().expect_err("not json").to_string();
    assert!(
        error.starts_with("the stored sign-in of account `work` is unreadable")
            && !error.contains("canary"),
        "{error}"
    );
    assert!(!format!("{storage:?}").contains("canary"));
}
