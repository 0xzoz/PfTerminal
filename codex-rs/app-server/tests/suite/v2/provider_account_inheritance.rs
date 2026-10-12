//! PF-84-S03 follow-up: a worker a client (the TUI) starts with
//! `thread/spawnAgent` inherits its parent's live named account, and a fork
//! runs on its source thread's account (#555).

use anyhow::Result;
use app_test_support::TestAppServer;
use app_test_support::to_response;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadForkParams;
use codex_app_server_protocol::ThreadForkResponse;
use codex_app_server_protocol::ThreadSpawnAgentParams;
use codex_app_server_protocol::ThreadSpawnAgentResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const WORK_KEY: &str = "pf84-s03-work-account-key";

#[tokio::test]
async fn client_started_worker_inherits_the_parent_account() -> Result<()> {
    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_assistant_message("msg-1", "pong"),
            responses::ev_completed("resp-1"),
        ]),
    )
    .await;
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[features]
named_accounts = true

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{}/v1"
wire_api = "responses"
env_key = "PF84_S03_UNSET_DEFAULT_KEY"
request_max_retries = 0
stream_max_retries = 0
"#,
            server.uri()
        ),
    )?;
    codex_vault::Vault::new(codex_home.path().to_path_buf()).write_provider_account(
        "mock_provider",
        &codex_vault::ProviderAccountName::parse("work")?,
        codex_vault::ProviderAccountKind::ApiKey,
        WORK_KEY,
    )?;

    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .build()
        .await?;
    timeout(DEFAULT_TIMEOUT, app.initialize()).await??;

    let root_request = app
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            provider_account: Some("work".to_string()),
            ..Default::default()
        })
        .await?;
    let root_response = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(root_request)),
    )
    .await??;
    let ThreadStartResponse { thread: root, .. } =
        to_response::<ThreadStartResponse>(root_response)?;

    // The worker names no account: it must not fall back to `default`.
    let child_request = app
        .send_raw_request(
            "thread/spawnAgent",
            Some(serde_json::to_value(ThreadSpawnAgentParams {
                parent_thread_id: root.id.clone(),
                agent_role: "worker".to_string(),
                agent_nickname: Some("worker-a".to_string()),
                agent_class: None,
                thread: ThreadStartParams {
                    model: Some("mock-model".to_string()),
                    ..Default::default()
                },
            })?),
        )
        .await?;
    let child_response: JSONRPCResponse = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(child_request)),
    )
    .await??;
    let ThreadSpawnAgentResponse { thread: child, .. } =
        to_response::<ThreadSpawnAgentResponse>(child_response)?;

    timeout(
        DEFAULT_TIMEOUT,
        app.start_turn_and_wait_for_completion(TurnStartParams {
            thread_id: child.id,
            input: vec![UserInput::Text {
                text: "say pong".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        }),
    )
    .await??;

    assert_eq!(
        response_mock.single_request().header("authorization"),
        Some(format!("Bearer {WORK_KEY}"))
    );
    Ok(())
}

// PF-84 (#555): `thread/fork` (TUI `/fork`, a prompt-edit fork, `/side`)
// runs on the account its source thread runs on, not on
// `[provider_accounts]` or `default`.

const PERSONAL_KEY: &str = "pf84-555-personal-account-key";
const OTHER_KEY: &str = "pf84-555-other-account-key";

fn pong(response_id: &str) -> String {
    responses::sse(vec![
        responses::ev_response_created(response_id),
        responses::ev_assistant_message(&format!("msg-{response_id}"), "pong"),
        responses::ev_completed(response_id),
    ])
}

/// A home whose `[provider_accounts]` selects `personal`, with `work`,
/// `personal` and `other` enrolled and no default key.
fn fork_home(server_uri: &str) -> Result<TempDir> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
model_provider = "mock_provider"

[features]
named_accounts = true

[provider_accounts]
mock_provider = "personal"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
env_key = "PF84_S03_UNSET_DEFAULT_KEY"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )?;
    let vault = codex_vault::Vault::new(codex_home.path().to_path_buf());
    for (name, key) in [
        ("work", WORK_KEY),
        ("personal", PERSONAL_KEY),
        ("other", OTHER_KEY),
    ] {
        vault.write_provider_account(
            "mock_provider",
            &codex_vault::ProviderAccountName::parse(name)?,
            codex_vault::ProviderAccountKind::ApiKey,
            key,
        )?;
    }
    Ok(codex_home)
}

async fn start_app(codex_home: &TempDir) -> Result<TestAppServer> {
    let mut app = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .without_auto_env()
        .build()
        .await?;
    timeout(DEFAULT_TIMEOUT, app.initialize()).await??;
    Ok(app)
}

async fn say_pong(app: &mut TestAppServer, thread_id: String) -> Result<()> {
    timeout(
        DEFAULT_TIMEOUT,
        app.start_turn_and_wait_for_completion(TurnStartParams {
            thread_id,
            input: vec![UserInput::Text {
                text: "say pong".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        }),
    )
    .await??;
    Ok(())
}

/// A thread started on `work` that has answered one turn.
async fn work_thread(app: &mut TestAppServer) -> Result<String> {
    let request = app
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            provider_account: Some("work".to_string()),
            ..Default::default()
        })
        .await?;
    let response = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(request)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(response)?;
    say_pong(app, thread.id.clone()).await?;
    Ok(thread.id)
}

async fn fork(
    app: &mut TestAppServer,
    thread_id: String,
    provider_account: Option<&str>,
) -> Result<ThreadForkResponse> {
    let request = app
        .send_thread_fork_request(ThreadForkParams {
            thread_id,
            provider_account: provider_account.map(str::to_string),
            ..Default::default()
        })
        .await?;
    let response = timeout(
        DEFAULT_TIMEOUT,
        app.read_stream_until_response_message(RequestId::Integer(request)),
    )
    .await??;
    to_response::<ThreadForkResponse>(response)
}

fn bearers(response_mock: &responses::ResponseMock) -> Vec<Option<String>> {
    response_mock
        .requests()
        .iter()
        .map(|request| request.header("authorization"))
        .collect()
}

#[tokio::test]
async fn fork_of_a_loaded_thread_runs_on_its_account() -> Result<()> {
    let server = responses::start_mock_server().await;
    let response_mock =
        responses::mount_sse_sequence(&server, vec![pong("resp-1"), pong("resp-2")]).await;
    let codex_home = fork_home(&server.uri())?;
    let mut app = start_app(&codex_home).await?;

    let source = work_thread(&mut app).await?;
    let forked = fork(&mut app, source, /*provider_account*/ None).await?;
    assert_eq!(
        forked.provider_account.as_deref(),
        Some("mock_provider:work")
    );
    say_pong(&mut app, forked.thread.id).await?;

    assert_eq!(
        bearers(&response_mock),
        vec![
            Some(format!("Bearer {WORK_KEY}")),
            Some(format!("Bearer {WORK_KEY}")),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn fork_of_a_stored_thread_runs_on_its_recorded_account() -> Result<()> {
    let server = responses::start_mock_server().await;
    let response_mock =
        responses::mount_sse_sequence(&server, vec![pong("resp-1"), pong("resp-2")]).await;
    let codex_home = fork_home(&server.uri())?;
    let mut app = start_app(&codex_home).await?;
    let source = work_thread(&mut app).await?;
    app.shutdown_gracefully().await?;

    // A new app server has not loaded the thread: the fork takes the account
    // its newest turn recorded.
    let mut app = start_app(&codex_home).await?;
    let forked = fork(&mut app, source, /*provider_account*/ None).await?;
    assert_eq!(
        forked.provider_account.as_deref(),
        Some("mock_provider:work")
    );
    say_pong(&mut app, forked.thread.id).await?;

    assert_eq!(
        bearers(&response_mock),
        vec![
            Some(format!("Bearer {WORK_KEY}")),
            Some(format!("Bearer {WORK_KEY}")),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn an_explicit_fork_account_beats_the_source_account() -> Result<()> {
    let server = responses::start_mock_server().await;
    let response_mock =
        responses::mount_sse_sequence(&server, vec![pong("resp-1"), pong("resp-2")]).await;
    let codex_home = fork_home(&server.uri())?;
    let mut app = start_app(&codex_home).await?;

    // `other` is neither the source's account nor `[provider_accounts]`.
    let source = work_thread(&mut app).await?;
    let forked = fork(&mut app, source, Some("other")).await?;
    assert_eq!(forked.provider_account.as_deref(), Some("mock_provider:other"));
    say_pong(&mut app, forked.thread.id).await?;

    assert_eq!(
        bearers(&response_mock),
        vec![
            Some(format!("Bearer {WORK_KEY}")),
            Some(format!("Bearer {OTHER_KEY}")),
        ]
    );
    Ok(())
}
