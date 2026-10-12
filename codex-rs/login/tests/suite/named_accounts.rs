//! PF-84-S06 slice B: named ChatGPT sign-in accounts refresh, reload and log
//! out through their own vault store only. Runs with
//! `CORBANU_TEST_NO_NATIVE_KEYRING=1` (the isolated test runner), so the vault
//! key stays in the disposable home.

use std::ffi::OsString;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use chrono::Duration;
use chrono::Utc;
use codex_config::types::AuthCredentialsStoreMode;
use codex_login::AuthDotJson;
use codex_login::AuthKeyringBackendKind;
use codex_login::AuthManager;
use codex_login::NamedChatgptAccount;
use codex_login::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR;
use codex_login::save_auth;
use codex_login::token_data::IdTokenInfo;
use codex_login::token_data::TokenData;
use codex_protocol::auth::AuthMode;
use codex_vault::ProviderAccountKind;
use codex_vault::ProviderAccountName;
use codex_vault::Vault;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::body_partial_json;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn jwt(payload: serde_json::Value) -> String {
    let b64 = |data: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data);
    format!(
        "{}.{}.{}",
        b64(br#"{"alg":"none","typ":"JWT"}"#),
        b64(payload.to_string().as_bytes()),
        b64(b"sig")
    )
}

/// A ChatGPT sign-in whose access token has expired a day ago.
fn sign_in(user: &str, access: &str, refresh: &str) -> AuthDotJson {
    AuthDotJson {
        auth_mode: Some(AuthMode::Chatgpt),
        openai_api_key: None,
        tokens: Some(TokenData {
            id_token: IdTokenInfo {
                raw_jwt: jwt(json!({ "sub": user })),
                ..Default::default()
            },
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            account_id: Some(format!("{user}-workspace")),
        }),
        last_refresh: Some(Utc::now() - Duration::days(30)),
        agent_identity: None,
        personal_access_token: None,
        bedrock_api_key: None,
    }
}

fn account(name: &str) -> NamedChatgptAccount {
    NamedChatgptAccount {
        provider_id: "openai".to_string(),
        name: name.to_string(),
    }
}

fn write_account(home: &TempDir, name: &str, auth: &AuthDotJson) -> Result<()> {
    Vault::new(home.path().to_path_buf()).write_provider_account(
        "openai",
        &ProviderAccountName::parse(name)?,
        ProviderAccountKind::ChatgptAuth,
        &serde_json::to_string(auth)?,
    )?;
    Ok(())
}

fn read_account(home: &TempDir, name: &str) -> Result<Option<AuthDotJson>> {
    Vault::new(home.path().to_path_buf())
        .read_provider_account(
            "openai",
            &ProviderAccountName::parse(name)?,
            ProviderAccountKind::ChatgptAuth,
        )?
        .map(|raw| serde_json::from_str(&raw).context("stored sign-in"))
        .transpose()
}

fn tokens(auth: Option<AuthDotJson>) -> Option<(String, String)> {
    auth.and_then(|auth| auth.tokens)
        .map(|tokens| (tokens.access_token, tokens.refresh_token))
}

struct Fixture {
    home: TempDir,
    base: Arc<AuthManager>,
    default_bytes: Vec<u8>,
    _env: EnvGuard,
}

/// A home with a default ChatGPT login in `auth.json` and two named
/// accounts, `work` and `play`, in the vault.
async fn fixture(server: &MockServer) -> Result<Fixture> {
    let home = TempDir::new()?;
    let env = EnvGuard::set(
        REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR,
        format!("{}/oauth/token", server.uri()),
    );
    save_auth(
        home.path(),
        &sign_in("default-user", "default-access-1", "default-refresh-1"),
        AuthCredentialsStoreMode::File,
        AuthKeyringBackendKind::default(),
    )?;
    write_account(
        &home,
        "work",
        &sign_in("work-user", "work-access-1", "work-refresh-1"),
    )?;
    write_account(
        &home,
        "play",
        &sign_in("play-user", "play-access-1", "play-refresh-1"),
    )?;
    let default_bytes = std::fs::read(home.path().join("auth.json"))?;
    let base = AuthManager::shared(
        home.path().to_path_buf(),
        /*enable_codex_api_key_env*/ false,
        AuthCredentialsStoreMode::File,
        /*forced_chatgpt_workspace_id*/ None,
        /*chatgpt_base_url*/ None,
        AuthKeyringBackendKind::default(),
        codex_login::test_support::transport_default_auth_route_config(),
    )
    .await;
    Ok(Fixture {
        home,
        base,
        default_bytes,
        _env: env,
    })
}

async fn mount_refresh(server: &MockServer, from: &str, access: &str, refresh: &str) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_partial_json(json!({ "refresh_token": from })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": access,
            "refresh_token": refresh,
        })))
        .expect(1)
        .mount(server)
        .await;
}

/// B2: two accounts refreshed at the same time each get their own rotated
/// tokens; the default login is untouched, byte for byte.
#[serial_test::serial(auth_env)]
#[tokio::test]
async fn concurrent_refreshes_write_back_to_their_own_account_only() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = MockServer::start().await;
    mount_refresh(&server, "work-refresh-1", "work-access-2", "work-refresh-2").await;
    mount_refresh(&server, "play-refresh-1", "play-access-2", "play-refresh-2").await;
    let fixture = fixture(&server).await?;

    let work = fixture.base.for_named_account_loaded(account("work")).await;
    let play = fixture.base.for_named_account_loaded(account("play")).await;
    // One manager, so one refresh lock, per account in a process.
    assert!(Arc::ptr_eq(
        &work,
        &fixture.base.for_named_account(account("work"))
    ));
    let (work_result, play_result) = tokio::join!(
        work.refresh_token_from_authority(),
        play.refresh_token_from_authority()
    );
    work_result.context("work refresh")?;
    play_result.context("play refresh")?;

    assert_eq!(
        (
            tokens(read_account(&fixture.home, "work")?),
            tokens(read_account(&fixture.home, "play")?),
        ),
        (
            Some(("work-access-2".to_string(), "work-refresh-2".to_string())),
            Some(("play-access-2".to_string(), "play-refresh-2".to_string())),
        )
    );
    assert_eq!(
        (
            work.auth().await.and_then(|auth| auth.get_token().ok()),
            play.auth().await.and_then(|auth| auth.get_token().ok()),
            fixture
                .base
                .auth()
                .await
                .and_then(|auth| auth.get_token().ok()),
        ),
        (
            Some("work-access-2".to_string()),
            Some("play-access-2".to_string()),
            Some("default-access-1".to_string()),
        )
    );
    assert_eq!(
        std::fs::read(fixture.home.path().join("auth.json"))?,
        fixture.default_bytes
    );
    Ok(())
}

/// B3: logging out of `work` revokes only its token and clears only its
/// store; `play` and the default login stay. An account without a sign-in
/// never falls back to the default login.
#[serial_test::serial(auth_env)]
#[tokio::test]
async fn logout_clears_only_that_account_and_missing_accounts_never_fall_back() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/revoke"))
        .and(body_partial_json(json!({ "token": "work-refresh-1" })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let fixture = fixture(&server).await?;

    let work = fixture.base.for_named_account_loaded(account("work")).await;
    assert!(work.logout_with_revoke().await?);
    assert_eq!(
        (
            read_account(&fixture.home, "work")?,
            tokens(read_account(&fixture.home, "play")?),
            work.auth().await.is_none(),
        ),
        (
            None,
            Some(("play-access-1".to_string(), "play-refresh-1".to_string())),
            true,
        )
    );
    assert_eq!(
        std::fs::read(fixture.home.path().join("auth.json"))?,
        fixture.default_bytes
    );
    let missing = fixture.base.for_named_account(account("missing"));
    assert!(missing.auth().await.is_none());
    Ok(())
}

struct EnvGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let original = std::env::var_os(key);
        // SAFETY: these tests run serially (`auth_env`), so updating the process environment is safe.
        unsafe {
            std::env::set_var(key, &value);
        }
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: the guard restores the original value before other tests run.
        unsafe {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
