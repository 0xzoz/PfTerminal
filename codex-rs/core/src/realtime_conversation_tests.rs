use super::AGENT_FINAL_MESSAGE_PREFIX;
use super::HANDOFF_STREAM_TRUNCATION_MARKER;
use super::RealtimeHandoffState;
use super::RealtimeSessionKind;
use super::RealtimeStreamedItem;
use super::realtime_api_key;
use super::realtime_delegation_from_handoff;
use super::realtime_request_headers;
use super::realtime_text_from_handoff_request;
use super::wrap_realtime_delegation_input;
use crate::context::RealtimeDelegationSource;
use async_channel::bounded;
use codex_api::RealtimeEventParser;
use codex_protocol::models::MessagePhase;
use codex_protocol::protocol::CodexResponseHandoffMode;
use codex_protocol::protocol::RealtimeHandoffRequested;
use codex_protocol::protocol::RealtimeTranscriptEntry;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

fn pf_30_realtime_start(
    model_client: crate::client::ModelClient,
    base_url: String,
    sdp: Option<String>,
) -> super::RealtimeStart {
    let mut api_provider = model_client
        .provider_info()
        .to_api_provider(/*auth_mode*/ None)
        .unwrap();
    api_provider.base_url = base_url;
    super::RealtimeStart {
        accounting: None,
        api_provider,
        realtime_sideband_base_url: None,
        extra_headers: None,
        client_managed_handoffs: false,
        flush_transcript_tail_on_session_end: true,
        codex_responses_as_items: false,
        codex_response_item_prefix: None,
        codex_response_handoff_mode: CodexResponseHandoffMode::default(),
        codex_response_handoff_channel_prefixes: None,
        realtime_call_api_provider: None,
        session_config: codex_api::RealtimeSessionConfig {
            instructions: "unadmitted-initial-instructions".into(),
            initial_items: Vec::new(),
            model: None,
            session_id: None,
            event_parser: RealtimeEventParser::V1,
            session_mode: codex_api::RealtimeSessionMode::Conversational,
            output_modality: codex_protocol::protocol::RealtimeOutputModality::Audio,
            voice: codex_protocol::protocol::RealtimeVoice::Alloy,
        },
        model_client,
        sdp,
    }
}

#[tokio::test]
async fn pf_30_s01_realtime_transports_deny_protected_start_before_network() {
    use codex_security_policy::SecurityLevel;
    let (session, _) = crate::session::tests::make_session_and_context().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    for level in [SecurityLevel::Moderate, SecurityLevel::Aggressive] {
        for sdp in [None, Some("unadmitted-sdp".into())] {
            let model_client = session
                .services
                .model_client()
                .as_ref()
                .clone()
                .with_ingress_level(level);
            let manager = super::RealtimeConversationManager::new();
            let result = manager
                .start(pf_30_realtime_start(model_client, base_url.clone(), sdp))
                .await;
            let error = result.err().expect("protected realtime must fail closed");
            assert!(error.to_string().contains("source admission"), "{error}");
            assert!(manager.running_state().await.is_none());
        }
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pf_30_s01_realtime_live_strengthening_stops_next_frame() {
    use crate::security::EffectivePolicyInitialization;
    use crate::security::EffectivePolicyView;
    use crate::security::PersistedHumanSecurityState;
    use crate::security::TrustedSecurityController;
    use codex_protocol::protocol::ConversationTextParams;
    use codex_protocol::protocol::ConversationTextRole;
    use codex_security_policy::PolicyPrincipal;
    use codex_security_policy::PrincipalKind;
    use codex_security_policy::RevocationState;
    use codex_security_policy::SecurityLevel;
    use codex_security_policy::SecuritySettings;
    use futures::StreamExt;
    use std::time::Duration;

    let (session, _) = crate::session::tests::make_session_and_context().await;
    let view = EffectivePolicyView::default();
    let controller = TrustedSecurityController::initialize(
        &view,
        PersistedHumanSecurityState::new(
            SecuritySettings::new(SecurityLevel::Permissive),
            PolicyPrincipal::new(PrincipalKind::Human, "fixture-human").unwrap(),
            RevocationState::new(),
        )
        .unwrap(),
        session.thread_id,
        codex_protocol::SessionId::from(session.thread_id),
        EffectivePolicyInitialization::Root,
    )
    .unwrap();
    let model_client = session
        .services
        .model_client()
        .as_ref()
        .clone()
        .with_ingress_policy(SecurityLevel::Permissive, view);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (frames_tx, frames_rx) = async_channel::unbounded();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        while let Some(Ok(message)) = websocket.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                frames_tx.send(text.to_string()).await.unwrap();
            }
        }
    });
    let manager = super::RealtimeConversationManager::new();
    let output = manager
        .start(pf_30_realtime_start(
            model_client,
            base_url,
            /*sdp*/ None,
        ))
        .await
        .unwrap();
    let initial = tokio::time::timeout(Duration::from_secs(5), frames_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(initial.contains("unadmitted-initial-instructions"));
    manager
        .text_in(ConversationTextParams {
            text: "permissive-text".into(),
            role: ConversationTextRole::User,
        })
        .await
        .unwrap();
    let text = tokio::time::timeout(Duration::from_secs(5), frames_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(text.contains("permissive-text"));
    controller
        .apply_confirmed_change(
            controller
                .confirm_level_change(SecurityLevel::Moderate, RevocationState::new())
                .unwrap(),
        )
        .unwrap();
    manager
        .text_in(ConversationTextParams {
            text: "protected-canary".into(),
            role: ConversationTextRole::User,
        })
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), output.events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, codex_api::RealtimeEvent::Error(error) if error.contains("source admission"))
    );
    // Await task completion before checking the recording, so absence is not a
    // race with the sender. No transcript tail may re-enter Core after denial.
    manager.shutdown().await.unwrap();
    while let Ok(frame) = frames_rx.try_recv() {
        assert!(!frame.contains("protected-canary"));
    }
    assert!(output.transcript_tail_rx.try_recv().is_err());
    server.abort();
}

#[test]
fn prefers_handoff_input_transcript_over_active_transcript() {
    let handoff = RealtimeHandoffRequested {
        handoff_id: "handoff_1".to_string(),
        item_id: "item_1".to_string(),
        input_transcript: "ignored".to_string(),
        active_transcript: vec![
            RealtimeTranscriptEntry {
                role: "user".to_string(),
                text: "hello".to_string(),
            },
            RealtimeTranscriptEntry {
                role: "assistant".to_string(),
                text: "hi there".to_string(),
            },
        ],
    };
    assert_eq!(
        realtime_text_from_handoff_request(&handoff),
        Some("ignored".to_string())
    );
}

#[test]
fn extracts_text_from_handoff_request_active_transcript_if_input_missing() {
    let handoff = RealtimeHandoffRequested {
        handoff_id: "handoff_1".to_string(),
        item_id: "item_1".to_string(),
        input_transcript: String::new(),
        active_transcript: vec![RealtimeTranscriptEntry {
            role: "user".to_string(),
            text: "hello".to_string(),
        }],
    };
    assert_eq!(
        realtime_text_from_handoff_request(&handoff),
        Some("user: hello".to_string())
    );
}

#[test]
fn wraps_handoff_with_transcript_delta() {
    let handoff = RealtimeHandoffRequested {
        handoff_id: "handoff_1".to_string(),
        item_id: "item_1".to_string(),
        input_transcript: "delegate this".to_string(),
        active_transcript: vec![
            RealtimeTranscriptEntry {
                role: "user".to_string(),
                text: "hello".to_string(),
            },
            RealtimeTranscriptEntry {
                role: "assistant".to_string(),
                text: "hi there".to_string(),
            },
        ],
    };
    assert_eq!(
        realtime_delegation_from_handoff(&handoff),
        Some(
            "<realtime_delegation>\n  <input>delegate this</input>\n  <transcript_delta>user: hello\nassistant: hi there</transcript_delta>\n</realtime_delegation>"
                .to_string()
        )
    );
}

#[test]
fn extracts_text_from_handoff_request_input_transcript_if_messages_missing() {
    let handoff = RealtimeHandoffRequested {
        handoff_id: "handoff_1".to_string(),
        item_id: "item_1".to_string(),
        input_transcript: "ignored".to_string(),
        active_transcript: vec![],
    };
    assert_eq!(
        realtime_text_from_handoff_request(&handoff),
        Some("ignored".to_string())
    );
}

#[test]
fn ignores_empty_handoff_request_input_transcript() {
    let handoff = RealtimeHandoffRequested {
        handoff_id: "handoff_1".to_string(),
        item_id: "item_1".to_string(),
        input_transcript: String::new(),
        active_transcript: vec![],
    };
    assert_eq!(realtime_text_from_handoff_request(&handoff), None);
}

#[test]
fn wraps_realtime_delegation_input() {
    assert_eq!(
        wrap_realtime_delegation_input(
            "hello",
            /*transcript_delta*/ None,
            RealtimeDelegationSource::Handoff,
        ),
        "<realtime_delegation>\n  <input>hello</input>\n</realtime_delegation>"
    );
}

#[test]
fn wraps_realtime_delegation_input_with_xml_escaping() {
    assert_eq!(
        wrap_realtime_delegation_input(
            "use a < b && c > d",
            Some("saw <that>"),
            RealtimeDelegationSource::Handoff,
        ),
        "<realtime_delegation>\n  <input>use a &lt; b &amp;&amp; c &gt; d</input>\n  <transcript_delta>saw &lt;that&gt;</transcript_delta>\n</realtime_delegation>"
    );
}

#[test]
fn wraps_realtime_delegation_input_with_xml_escaping_without_transcript() {
    assert_eq!(
        wrap_realtime_delegation_input(
            "use a < b && c > d",
            /*transcript_delta*/ None,
            RealtimeDelegationSource::Handoff,
        ),
        "<realtime_delegation>\n  <input>use a &lt; b &amp;&amp; c &gt; d</input>\n</realtime_delegation>"
    );
}

#[tokio::test]
async fn clears_active_handoff_explicitly() {
    let (tx, _rx) = bounded(1);
    let state = RealtimeHandoffState {
        output_tx: tx,
        last_output: Arc::new(Mutex::new(None)),
        stream: Arc::new(Mutex::new(Default::default())),
        client_managed_handoffs: false,
        codex_responses_as_items: false,
        codex_response_item_prefix: None,
        codex_response_handoff_mode: CodexResponseHandoffMode::Thinking,
        codex_response_handoff_channel_prefixes: Arc::new(BTreeMap::new()),
        session_kind: RealtimeSessionKind::V1,
        event_parser: RealtimeEventParser::V1,
    };

    state.stream.lock().await.active_handoff = Some("handoff_1".to_string());
    assert_eq!(
        state.stream.lock().await.active_handoff.clone(),
        Some("handoff_1".to_string())
    );

    state.stream.lock().await.active_handoff = None;
    assert_eq!(state.stream.lock().await.active_handoff.clone(), None);
}

#[test]
fn streamed_handoff_preserves_a_bounded_final_tail() {
    let mut item = RealtimeStreamedItem {
        handoff_id: "handoff_1".to_string(),
        phase: Some(MessagePhase::FinalAnswer),
        bem_channel_parser: None,
        prefix_final_message: true,
        sent_bytes: 0,
        buffered_text: String::new(),
        tail_text: String::new(),
        truncated: false,
        last_flush_at: Instant::now(),
        flush_scheduled: false,
    };
    item.push_text(&format!("HEAD{}TAIL", "x".repeat(/*n*/ 5_000)));

    let first = item
        .drain_stream_chunk()
        .expect("oversized output should retain a streamable head");
    let final_chunk = item
        .drain_final_chunk()
        .expect("oversized output should retain a final tail");
    let output = format!("{first}{final_chunk}");

    assert!(output.len() <= 4_000);
    assert!(output.starts_with(&format!("{AGENT_FINAL_MESSAGE_PREFIX}HEAD")));
    assert!(output.contains(HANDOFF_STREAM_TRUNCATION_MARKER));
    assert!(output.ends_with("TAIL"));
}

#[test]
fn streamed_v3_handoff_omits_the_final_message_prefix() {
    let mut item = RealtimeStreamedItem {
        handoff_id: "handoff_1".to_string(),
        phase: Some(MessagePhase::FinalAnswer),
        bem_channel_parser: None,
        prefix_final_message: false,
        sent_bytes: 0,
        buffered_text: String::new(),
        tail_text: String::new(),
        truncated: false,
        last_flush_at: Instant::now(),
        flush_scheduled: false,
    };
    item.push_text("done");

    assert_eq!(item.drain_final_chunk(), Some("done".to_string()));
}

#[test]
fn uses_quicksilver_alpha_header_for_realtime_v1() {
    let headers = realtime_request_headers(
        Some("session_1"),
        Some("sk-test"),
        RealtimeEventParser::V1,
        "codex_work_desktop",
    )
    .expect("headers")
    .expect("headers");

    assert_eq!(
        headers
            .get("openai-alpha")
            .and_then(|value| value.to_str().ok()),
        Some("quicksilver=v1")
    );
}

#[test]
fn omits_quicksilver_alpha_header_for_realtime_v2() {
    let headers = realtime_request_headers(
        Some("session_1"),
        Some("sk-test"),
        RealtimeEventParser::RealtimeV2,
        "codex_work_desktop",
    )
    .expect("headers")
    .expect("headers");

    assert!(headers.get("openai-alpha").is_none());
}

#[test]
fn uses_frameless_alpha_header_for_realtime_v3() {
    let headers = realtime_request_headers(
        Some("session_1"),
        Some("sk-test"),
        RealtimeEventParser::FramelessBidi,
        "codex_work_desktop",
    )
    .expect("headers")
    .expect("headers");

    assert_eq!(
        headers
            .get("openai-alpha")
            .and_then(|value| value.to_str().ok()),
        Some("quicksilver=v2")
    );
}

#[test]
fn realtime_headers_include_only_non_default_originator() {
    let default_originator = codex_login::default_client::originator();
    for (originator, expected_header) in [
        ("codex_work_desktop", Some("codex_work_desktop")),
        (default_originator.value.as_str(), None),
    ] {
        let headers = realtime_request_headers(
            Some("session_1"),
            Some("sk-test"),
            RealtimeEventParser::RealtimeV2,
            originator,
        )
        .expect("headers")
        .expect("headers");

        assert_eq!(
            headers
                .get("originator")
                .and_then(|value| value.to_str().ok()),
            expected_header
        );
    }
}

/// A realtime call is a model request the operator paid for, and it recorded
/// nothing. The realtime protocol this client parses carries no usage anywhere,
/// so the call is recorded with its tokens unknown - which is what the provider
/// said - rather than left invisible.
#[tokio::test]
async fn pf_60_s03_realtime_call_is_recorded() -> anyhow::Result<()> {
    use codex_state::SqliteConfig;
    use codex_state::StateRuntime;
    use codex_state::ThreadMetadataBuilder;
    use codex_utils_absolute_path::AbsolutePathBuf;

    let server = wiremock::MockServer::start().await;
    let endpoint = format!("{}/v1", server.uri());
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/realtime/calls"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("Location", "/v1/live/rtc_fixture")
                .set_body_string("v=answer\r\n"),
        )
        .mount(&server)
        .await;

    let home = tempfile::tempdir()?;
    let mut config = crate::session::tests::build_test_config(home.path()).await;
    config.model_provider = codex_model_provider_info::ModelProviderInfo {
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        supports_websockets: false,
        ..codex_model_provider_info::ModelProviderInfo::create_openai_provider(Some(
            endpoint.clone(),
        ))
    };
    config.model_provider_id = "openai".into();
    config.accounting = crate::config::AccountingMode::Provider {
        scope: uuid::Uuid::new_v4(),
        provider_id: "openai".into(),
        wire_api: codex_model_provider_info::WireApi::Responses,
        approved_endpoint: endpoint.clone(),
        approved_query: None,
        pricing: crate::config::PriceAuthority::Unavailable,
        basis_source: Default::default(),
        account: None,
    };
    config
        .features
        .enable(codex_features::Feature::Sqlite)
        .expect("sqlite for the ledger");

    let (mut session, _context) =
        crate::session::tests::make_session_and_context_for_config(config).await;
    let db = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(home.path().to_path_buf())?),
        "openai".into(),
    )
    .await?;
    session.services.state_db = Some(db.clone());
    // Accounting attributes attempts to a persisted thread; ephemeral sessions
    // run uncollected, so the fixture needs a live thread like a real session.
    crate::session::tests::open_thread_persistence(&mut session).await;
    let owner = Arc::new(session);
    db.upsert_thread(
        &ThreadMetadataBuilder::new(
            owner.thread_id,
            home.path().join("realtime-fixture.jsonl"),
            chrono::Utc::now(),
            codex_protocol::protocol::SessionSource::Cli,
        )
        .build("openai"),
    )
    .await?;

    let accounting = super::realtime_accounting(&owner, "fixture-sub", &endpoint)
        .await
        .expect("realtime accounting attaches");
    owner
        .services
        .model_client()
        .create_realtime_call_with_headers(
            "v=offer\r\n".to_string(),
            codex_api::RealtimeSessionConfig {
                instructions: String::new(),
                initial_items: Vec::new(),
                model: Some("gpt-realtime-1.5".into()),
                session_id: None,
                event_parser: RealtimeEventParser::V1,
                session_mode: codex_api::RealtimeSessionMode::Conversational,
                output_modality: codex_protocol::protocol::RealtimeOutputModality::Audio,
                voice: codex_protocol::protocol::RealtimeVoice::Alloy,
            },
            Default::default(),
            /*api_provider_override*/ None,
            &accounting.0.responses_accounting,
        )
        .await?;

    let pool = db
        .sqlite()
        .open_read_only_pool(&db.sqlite().state_db_path())
        .await?;
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT payload FROM draft_accounting_attempts ORDER BY rowid")
            .fetch_all(&pool)
            .await?;
    let turns: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            serde_json::from_str::<serde_json::Value>(row).ok()?["turn"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        turns
            .iter()
            .filter(|turn| turn.starts_with("realtime:"))
            .count(),
        1,
        "turns recorded: {turns:?}"
    );
    Ok(())
}

#[test]
fn realtime_sends_the_openai_api_key_env_fallback_only_to_openai() {
    let env_fallback =
        codex_login::CodexAuth::from_api_key_env("sk-env", codex_login::OPENAI_API_KEY_ENV_VAR);
    let oss = codex_model_provider_info::create_oss_provider_with_base_url(
        "http://localhost:11434/v1",
        codex_model_provider_info::WireApi::Responses,
    );
    let openai = codex_model_provider_info::ModelProviderInfo::create_openai_provider(
        /*base_url*/ None,
    );

    assert!(realtime_api_key(Some(&env_fallback), &oss).is_err());
    assert_eq!(
        realtime_api_key(Some(&env_fallback), &openai).expect("openai key"),
        "sk-env"
    );
    let stored = codex_login::CodexAuth::from_api_key("sk-stored");
    assert_eq!(
        realtime_api_key(Some(&stored), &oss).expect("stored key"),
        "sk-stored"
    );
}

/// How the stand-in realtime server ends a session it accepted.
#[derive(Clone, Copy)]
enum StandInEnd {
    /// Keeps the session open until the client closes it.
    ClientCloses,
    /// Drops the connection after the client's first message.
    ServerDrops,
}

/// A local stand-in for the realtime WebSocket endpoint: accepts sessions at
/// `http://127.0.0.1:<port>/v1` and answers nothing. No paid session is
/// opened.
async fn realtime_stand_in(end: StandInEnd) -> anyhow::Result<String> {
    use futures::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}/v1", listener.local_addr()?);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(socket).await else {
                    return;
                };
                let _ = ws.next().await;
                match end {
                    StandInEnd::ClientCloses => while let Some(Ok(_)) = ws.next().await {},
                    StandInEnd::ServerDrops => drop(ws),
                }
            });
        }
    });
    Ok(endpoint)
}

/// A persisted session whose OpenAI provider and realtime endpoint are the
/// stand-in at `endpoint`, recording under `accounting`.
async fn realtime_ledger_session(
    home: &std::path::Path,
    endpoint: &str,
    accounting: crate::config::AccountingMode,
) -> anyhow::Result<(
    Arc<crate::session::session::Session>,
    Arc<codex_state::StateRuntime>,
)> {
    use codex_state::SqliteConfig;
    use codex_state::StateRuntime;
    use codex_state::ThreadMetadataBuilder;
    use codex_utils_absolute_path::AbsolutePathBuf;

    let mut config = crate::session::tests::build_test_config(home).await;
    config.model_provider = codex_model_provider_info::ModelProviderInfo {
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        supports_websockets: false,
        ..codex_model_provider_info::ModelProviderInfo::create_openai_provider(Some(
            endpoint.to_string(),
        ))
    };
    config.model_provider_id = "openai".into();
    config.accounting = accounting;
    config
        .features
        .enable(codex_features::Feature::Sqlite)
        .expect("sqlite for the ledger");
    let (mut session, _context) =
        crate::session::tests::make_session_and_context_for_config(config).await;
    let db = StateRuntime::init(
        SqliteConfig::from_sqlite_home(AbsolutePathBuf::try_from(home.to_path_buf())?),
        "openai".into(),
    )
    .await?;
    session.services.state_db = Some(db.clone());
    crate::session::tests::open_thread_persistence(&mut session).await;
    let owner = Arc::new(session);
    db.upsert_thread(
        &ThreadMetadataBuilder::new(
            owner.thread_id,
            home.join("realtime-fixture.jsonl"),
            chrono::Utc::now(),
            codex_protocol::protocol::SessionSource::Cli,
        )
        .build("openai"),
    )
    .await?;
    Ok((owner, db))
}

fn realtime_websocket_params() -> codex_protocol::protocol::ConversationStartParams {
    codex_protocol::protocol::ConversationStartParams {
        client_managed_handoffs: false,
        flush_transcript_tail_on_session_end: false,
        codex_responses_as_items: false,
        codex_response_item_prefix: None,
        codex_response_handoff_mode: CodexResponseHandoffMode::default(),
        codex_response_handoff_channel_prefixes: None,
        model: Some("gpt-realtime-1.5".into()),
        output_modality: codex_protocol::protocol::RealtimeOutputModality::Text,
        include_startup_context: false,
        initial_items: Vec::new(),
        prompt: Some(Some("fixture prompt".into())),
        realtime_session_id: None,
        transport: Some(codex_protocol::protocol::ConversationStartTransport::Websocket),
        version: Some(codex_protocol::protocol::RealtimeConversationVersion::V2),
        voice: None,
    }
}

/// The ledger's attempts and observations, as stored.
async fn realtime_ledger(
    db: &codex_state::StateRuntime,
) -> anyhow::Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let pool = db
        .sqlite()
        .open_read_only_pool(&db.sqlite().state_db_path())
        .await?;
    let read = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            let rows: Vec<String> = sqlx::query_scalar(sql)
                .fetch_all(&pool)
                .await
                .unwrap_or_default();
            rows.iter()
                .map(|row| serde_json::from_str(row))
                .collect::<Result<Vec<serde_json::Value>, _>>()
        }
    };
    let attempts = read("SELECT payload FROM draft_accounting_attempts ORDER BY rowid").await?;
    let observations =
        read("SELECT payload FROM draft_accounting_observations ORDER BY rowid").await?;
    pool.close().await;
    Ok((attempts, observations))
}

/// Wait until the ledger holds an observation, or give up after ten seconds.
async fn realtime_ledger_settled(
    db: &codex_state::StateRuntime,
) -> anyhow::Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let ledger = realtime_ledger(db).await?;
        if !ledger.1.is_empty() || Instant::now() > deadline {
            return Ok(ledger);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The one recorded realtime session: its route, model and provider, its
/// start (admission) and end times, and no token count at all.
fn assert_one_unknown_session(
    attempts: &[serde_json::Value],
    observations: &[serde_json::Value],
    started_after_ms: i64,
) {
    let realtime: Vec<&serde_json::Value> = attempts
        .iter()
        .filter(|attempt| {
            attempt["turn"]
                .as_str()
                .is_some_and(|turn| turn.starts_with("realtime:"))
        })
        .collect();
    assert_eq!(realtime.len(), 1, "attempts: {attempts:#?}");
    assert_eq!(attempts.len(), 1, "attempts: {attempts:#?}");
    let session = realtime[0];
    assert_eq!(session["model"], "gpt-realtime-1.5");
    assert_eq!(session["provider"], "openai");
    assert_eq!(session["retry_of"], serde_json::Value::Null);
    let started = session["dispatched_at_ms"].as_i64().expect("start time");
    assert!(started >= started_after_ms, "{session:#?}");
    assert_eq!(observations.len(), 1, "observations: {observations:#?}");
    let patch = &observations[0]["patch"];
    let ended = patch["session_ended_at_ms"].as_i64().expect("end time");
    assert!(ended >= started, "{patch:#?}");
    // The end states nothing else: every token count stays unknown.
    assert_eq!(
        patch.as_object().map(serde_json::Map::len),
        Some(1),
        "{patch:#?}"
    );
}

/// #824 (Travis, 2026-10-15, option a): a realtime session over WebSocket is
/// recorded as one request - route, model, provider, start and end times -
/// with its tokens unknown, not zero. Before #824 it was not recorded at all.
#[tokio::test]
async fn pf_824_websocket_realtime_session_is_one_request_with_tokens_unknown() -> anyhow::Result<()>
{
    let endpoint = realtime_stand_in(StandInEnd::ClientCloses).await?;
    let home = tempfile::tempdir()?;
    let (owner, db) = realtime_ledger_session(
        home.path(),
        &endpoint,
        crate::config::AccountingMode::DirectOpenAiResponses {
            scope: uuid::Uuid::new_v4(),
            approved_endpoint: endpoint.clone(),
        },
    )
    .await?;
    let before = chrono::Utc::now().timestamp_millis();
    super::handle_start(&owner, "fixture-sub".into(), realtime_websocket_params()).await?;
    assert!(
        owner.conversation.running_state().await.is_some(),
        "the session started"
    );
    super::handle_close(&owner, "fixture-sub".into()).await;

    let (attempts, observations) = realtime_ledger_settled(&db).await?;
    assert_one_unknown_session(&attempts, &observations, before);
    Ok(())
}

/// #824: a session the provider drops (an abnormal close) is recorded the
/// same way, with the time it ended.
#[tokio::test]
async fn pf_824_websocket_realtime_session_dropped_by_the_server_is_recorded() -> anyhow::Result<()>
{
    let endpoint = realtime_stand_in(StandInEnd::ServerDrops).await?;
    let home = tempfile::tempdir()?;
    // The provider-bound mode a developer-accounting build uses.
    let (owner, db) = realtime_ledger_session(
        home.path(),
        &endpoint,
        crate::config::AccountingMode::Provider {
            scope: uuid::Uuid::new_v4(),
            provider_id: "openai".into(),
            wire_api: codex_model_provider_info::WireApi::Responses,
            approved_endpoint: endpoint.clone(),
            approved_query: None,
            pricing: crate::config::PriceAuthority::Unavailable,
            basis_source: Default::default(),
            account: None,
        },
    )
    .await?;
    let before = chrono::Utc::now().timestamp_millis();
    super::handle_start(&owner, "fixture-sub".into(), realtime_websocket_params()).await?;

    let (attempts, observations) = realtime_ledger_settled(&db).await?;
    assert_one_unknown_session(&attempts, &observations, before);
    super::handle_close(&owner, "fixture-sub".into()).await;
    Ok(())
}

/// #824: with collection off (any build without developer accounting), the
/// session runs exactly as before and nothing is recorded.
#[tokio::test]
async fn pf_824_websocket_realtime_session_is_not_recorded_when_collection_is_off()
-> anyhow::Result<()> {
    let endpoint = realtime_stand_in(StandInEnd::ClientCloses).await?;
    let home = tempfile::tempdir()?;
    let (owner, db) = realtime_ledger_session(
        home.path(),
        &endpoint,
        crate::config::AccountingMode::Disabled,
    )
    .await?;
    super::handle_start(&owner, "fixture-sub".into(), realtime_websocket_params()).await?;
    assert!(
        owner.conversation.running_state().await.is_some(),
        "the session started"
    );
    super::handle_close(&owner, "fixture-sub".into()).await;
    let (attempts, observations) = realtime_ledger(&db).await?;
    assert_eq!(attempts, Vec::<serde_json::Value>::new());
    assert_eq!(observations, Vec::<serde_json::Value>::new());
    Ok(())
}
