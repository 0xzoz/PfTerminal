//! PF-84-S06 (#658): the app server's own ChatGPT backend requests (usage and
//! rate limits here; plugin listings, settings and analytics go through the
//! same sign-in) use the account the user's session runs on. A session on a
//! named ChatGPT account never sends the default login's token.

use anyhow::Result;
use app_test_support::ChatGptAuthFixture;
use app_test_support::TestAppServer;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use app_test_support::write_named_chatgpt_auth;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_config::types::AuthCredentialsStoreMode;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TOKEN: &str = "pf84-658-default-login-token";
const WORK_TOKEN: &str = "pf84-658-work-account-token";

/// A home with a default ChatGPT login and a named ChatGPT account `work`,
/// both pointing at `server` for the ChatGPT backend and model requests.
fn write_home(home: &Path, server: &MockServer, provider_accounts: &str) -> Result<()> {
    write_chatgpt_auth(
        home,
        ChatGptAuthFixture::new(DEFAULT_TOKEN)
            .account_id("acct-default")
            .plan_type("pro"),
        AuthCredentialsStoreMode::File,
    )?;
    write_named_chatgpt_auth(
        home,
        "openai",
        "work",
        ChatGptAuthFixture::new(WORK_TOKEN)
            .account_id("acct-work")
            .plan_type("pro"),
    )?;
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "openai"
chatgpt_base_url = "{uri}"
openai_base_url = "{uri}/v1"

[features]
named_accounts = true
{provider_accounts}
"#,
            uri = server.uri()
        ),
    )?;
    Ok(())
}

async fn mount_usage(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/codex/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 1,
                    "limit_window_seconds": 3600,
                    "reset_after_seconds": 120,
                    "reset_at": 1735693200
                }
            }
        })))
        .mount(server)
        .await;
}

async fn start_thread(app: &mut TestAppServer, account: Option<&str>) -> Result<()> {
    let request = app
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            provider_account: account.map(str::to_string),
            ..Default::default()
        })
        .await?;
    let response = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(request)),
    )
    .await??;
    let _ = to_response::<ThreadStartResponse>(response)?;
    Ok(())
}

async fn read_rate_limits(app: &mut TestAppServer) -> Result<()> {
    let request = app.send_get_account_rate_limits_request().await?;
    let _: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(request)),
    )
    .await??;
    Ok(())
}

/// The bearer token of every backend request `server` received for `route`,
/// in order.
async fn tokens_for(server: &MockServer, route: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.url.path() == route)
        .map(|request| {
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .trim_start_matches("Bearer ")
                .to_string()
        })
        .collect()
}

/// Whether any request `server` received carried the default login's token.
async fn default_login_was_sent(server: &MockServer) -> bool {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .any(|request| {
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains(DEFAULT_TOKEN))
        })
}

#[tokio::test]
async fn a_session_started_on_a_named_account_never_sends_the_default_login() -> Result<()> {
    let server = MockServer::start().await;
    mount_usage(&server).await;
    let home = TempDir::new()?;
    write_home(
        home.path(),
        &server,
        "[provider_accounts]\nopenai = \"work\"\n",
    )?;

    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_auto_env()
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;
    read_rate_limits(&mut app).await?;

    assert_eq!(
        tokens_for(&server, "/api/codex/usage").await,
        vec![WORK_TOKEN.to_string()]
    );
    assert!(!default_login_was_sent(&server).await);
    Ok(())
}

#[tokio::test]
async fn backend_requests_follow_a_thread_started_on_a_named_account() -> Result<()> {
    let server = MockServer::start().await;
    mount_usage(&server).await;
    let home = TempDir::new()?;
    write_home(home.path(), &server, "")?;

    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_auto_env()
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;
    // No session yet: the default login (control).
    read_rate_limits(&mut app).await?;

    start_thread(&mut app, Some("work")).await?;
    read_rate_limits(&mut app).await?;

    assert_eq!(
        tokens_for(&server, "/api/codex/usage").await,
        vec![DEFAULT_TOKEN.to_string(), WORK_TOKEN.to_string()]
    );
    Ok(())
}

/// A second user thread (another pane) on the default login while one runs on
/// `work`: requests outside a conversation are refused with the reason and
/// not sent with either account.
#[tokio::test]
async fn sessions_on_different_accounts_send_neither_login() -> Result<()> {
    let server = MockServer::start().await;
    mount_usage(&server).await;
    let home = TempDir::new()?;
    write_home(home.path(), &server, "")?;

    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_auto_env()
        .with_env_overrides(&[("OPENAI_API_KEY", None)])
        .build_initialized_with_timeout(DEFAULT_TIMEOUT)
        .await?;
    start_thread(&mut app, Some("work")).await?;
    start_thread(&mut app, /*account*/ None).await?;

    let request = app.send_get_account_rate_limits_request().await?;
    let error: JSONRPCError = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_error_message(RequestId::Integer(request)),
    )
    .await??;
    assert!(
        error
            .error
            .message
            .contains("sessions on different accounts are open"),
        "{}",
        error.error.message
    );
    // (The pane on the default login sends its own requests with it.)
    assert_eq!(
        tokens_for(&server, "/api/codex/usage").await,
        Vec::<String>::new()
    );
    Ok(())
}
