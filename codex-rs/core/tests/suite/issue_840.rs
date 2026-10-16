//! #840 (D6, Travis 2026-10-14/15): under Moderate, a "don't ask again" rule
//! saved earlier (as under Permissive) still skips the prompt of a command
//! that asks to run outside the sandbox. The command runs inside the sandbox
//! with Moderate's protected-file rules kept: only a person's fresh answer
//! lifts them (#239). After untrusted content the rule counts for nothing on
//! a protected action: the person is asked afresh (PF-30-S03).
#![cfg(target_os = "macos")]

use std::sync::Arc;

use codex_core::config::Constrained;
use codex_features::Feature;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::user_input::UserInput;
use codex_security_policy::SecurityLevel;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::skip_if_sandbox;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

const CANARY: &str = "ISSUE_840_CANARY";

fn step(call_id: &str, command: &str, escalate: bool) -> Vec<Value> {
    let mut arguments = json!({ "command": command });
    if escalate {
        arguments["sandbox_permissions"] = json!("require_escalated");
        arguments["justification"] = json!("read the notes");
    }
    vec![
        ev_response_created(call_id),
        ev_function_call(call_id, "shell_command", &arguments.to_string()),
        ev_completed(call_id),
    ]
}

/// What happened to the read of a protected file in the Corbanu home.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    asked: usize,
    read: bool,
    denied_by_sandbox: bool,
}

/// Under Moderate, a rule saved for `cat` and a `cat` of a file in the
/// Corbanu home that asks to run outside the sandbox, optionally after
/// untrusted content; every prompt is declined.
async fn read_with_rule(tainted: bool) -> anyhow::Result<Outcome> {
    let home = Arc::new(TempDir::new()?);
    let path = home.path().canonicalize()?.join("issue-840-notes.txt");
    std::fs::write(&path, CANARY)?;
    std::fs::create_dir_all(home.path().join("rules"))?;
    std::fs::write(
        home.path().join("rules/default.rules"),
        "prefix_rule(pattern=[\"cat\"], decision=\"allow\")\n",
    )?;
    let mut steps = Vec::new();
    if tainted {
        // Tool output: untrusted content.
        steps.push(step("call-first", "printf '%s' 'notes'", false));
    }
    steps.push(step(
        "call-read",
        &format!("cat '{}'", path.display()),
        /*escalate*/ true,
    ));
    steps.push(vec![
        ev_response_created("done"),
        ev_assistant_message("msg-done", "done"),
        ev_completed("done"),
    ]);
    let server = start_mock_server().await;
    let captured = mount_sse_sequence(&server, steps.into_iter().map(sse).collect()).await;
    let test = test_codex()
        .with_home(home)
        .with_model("test-gpt-5.1-codex")
        .with_config(|config| {
            config.security_level = SecurityLevel::Moderate;
            config.permissions.approval_policy = Constrained::allow_any(AskForApproval::OnRequest);
            config
                .features
                .enable(Feature::SourceEnvelopes)
                .expect("enable source envelopes");
        })
        // Host-sandbox security test: local executor on purpose (#157).
        .build(&server)
        .await?;
    std::mem::forget(server);
    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "read the notes".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    let mut asked = 0;
    while let EventMsg::ExecApprovalRequest(approval) = wait_for_event(&test.codex, |event| {
        matches!(
            event,
            EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_)
        )
    })
    .await
    {
        asked += 1;
        test.codex
            .submit(Op::ExecApproval {
                id: approval.effective_approval_id(),
                turn_id: None,
                decision: ReviewDecision::denied("no"),
            })
            .await?;
    }
    let requests = captured.requests();
    let output = requests
        .last()
        .and_then(|request| request.function_call_output_text("call-read"))
        .unwrap_or_default();
    Ok(Outcome {
        asked,
        read: output.contains(CANARY),
        denied_by_sandbox: output.contains("Operation not permitted"),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_840_saved_rule_skips_the_prompt_but_keeps_moderates_file_rules() -> anyhow::Result<()>
{
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    // Before untrusted content: no prompt; the sandbox still denies the read.
    assert_eq!(
        read_with_rule(/*tainted*/ false).await?,
        Outcome {
            asked: 0,
            read: false,
            denied_by_sandbox: true,
        }
    );
    // After untrusted content: the rule does not count; the person is asked.
    let tainted = read_with_rule(/*tainted*/ true).await?;
    assert_eq!((tainted.asked, tainted.read), (1, false), "{tainted:?}");
    Ok(())
}
