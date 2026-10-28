use super::*;

const CALL_ID: &str = "slow-to-record-call";

/// Runs one direct tool call the way a regular turn does, but is slow to record
/// the call's output, like a loaded runtime that misses the interrupt grace
/// period (#1467).
struct SlowToRecordToolTask {
    tool_started: Arc<tokio::sync::Notify>,
}

impl SessionTask for SlowToRecordToolTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.slow_to_record_tool"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        _input: Vec<TurnInput>,
        cancellation_token: CancellationToken,
    ) -> SessionTaskResult {
        let session = session.clone_session();
        let item = ResponseItem::FunctionCall {
            id: None,
            name: "test_sync_tool".to_string(),
            namespace: None,
            arguments: json!({ "sleep_after_ms": 60_000 }).to_string(),
            call_id: CALL_ID.to_string(),
            encrypted_function_args: None,
            internal_chat_message_metadata_passthrough: None,
        };
        session
            .record_conversation_items(&ctx, std::slice::from_ref(&item))
            .await;
        let call = ToolRouter::build_tool_call(item)
            .expect("test_sync_tool call should parse")
            .expect("test_sync_tool item should build a tool call");
        let tool = test_tool_runtime(Arc::clone(&session), Arc::clone(&ctx))
            .handle_tool_call(call, cancellation_token.child_token());
        self.tool_started.notify_one();
        let output = tool
            .await
            .expect("cancelled tool should return its abort output");
        // The interrupt grace period runs out before the output is recorded.
        sleep(Duration::from_secs(60)).await;
        session
            .record_conversation_items(&ctx, &[output.into()])
            .await;
        Ok(None)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hard_aborted_turn_records_tool_abort_output() {
    let (sess, mut tc) = make_session_and_context().await;
    tc.model_info
        .experimental_supported_tools
        .push("test_sync_tool".to_string());
    let sess = Arc::new(sess);
    let tool_started = Arc::new(tokio::sync::Notify::new());
    sess.spawn_task(
        Arc::new(tc),
        Vec::new(),
        SlowToRecordToolTask {
            tool_started: Arc::clone(&tool_started),
        },
    )
    .await;
    timeout(Duration::from_secs(10), tool_started.notified())
        .await
        .expect("tool call should start");

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let history = sess.clone_history().await;
    let outputs: Vec<_> = history
        .raw_items()
        .iter()
        .filter_map(|item| match item {
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } if call_id == CALL_ID => Some(output.body.to_text()),
            _ => None,
        })
        .collect();
    let [Some(output)] = outputs.as_slice() else {
        panic!("expected one text abort output for the call, got {outputs:?}");
    };
    assert!(
        output.starts_with("aborted by user after ") && output.ends_with('s'),
        "expected the tool's timed abort output, got {output:?}"
    );
}
