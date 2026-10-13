//! Issue #505: confirming Aggressive in `/security` requires approval for
//! every command that is not known read-only, decided by a person, from each
//! thread's next turn, before the restart that loads the Aggressive rules.
#![cfg(unix)]

use codex_core::config::Constrained;
use codex_core::security_level_change::LevelBasis;
use codex_core::security_level_change::LevelChangeKind;
use codex_core::security_level_change::Probes;
use codex_core::security_level_change::commit_human_level_change;
use codex_features::Feature;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::models::PermissionProfile;
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
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::json;

/// What happened to a command that writes a file, in a Permissive session
/// with approvals off and the automatic reviewer chosen; with `confirm`, the
/// person confirms Aggressive before the turn, as `/security` does.
struct Run {
    /// Whether a person was asked (they decline).
    asked: bool,
    /// Whether the command ran.
    wrote: bool,
}

async fn run(confirm: bool) -> anyhow::Result<Run> {
    let server = start_mock_server().await;
    let command = "printf x > issue505.txt";
    mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("call-0"),
                ev_function_call(
                    "call-0",
                    "shell_command",
                    &json!({ "command": command }).to_string(),
                ),
                ev_completed("call-0"),
            ]),
            sse(vec![
                ev_response_created("done"),
                ev_assistant_message("msg-done", "done"),
                ev_completed("done"),
            ]),
        ],
    )
    .await;
    let test = test_codex()
        .with_model("test-gpt-5.1-codex")
        .with_config(|config| {
            config.security_level = SecurityLevel::Permissive;
            config.permissions.approval_policy = Constrained::allow_any(AskForApproval::Never);
            config.approvals_reviewer = ApprovalsReviewer::AutoReview;
            config
                .permissions
                .set_permission_profile(PermissionProfile::workspace_write())
                .expect("workspace write");
            config
                .features
                .enable(Feature::SourceEnvelopes)
                .expect("enable source envelopes");
        })
        .build_with_auto_env(&server)
        .await?;
    if confirm {
        let home = test.codex_home_path();
        let thread = Some(test.session_configured.thread_id);
        let basis = LevelBasis::read(home, SecurityLevel::Permissive, thread);
        assert!(basis.live, "the session's policy tree runs in this process");
        let report = commit_human_level_change(
            home,
            SecurityLevel::Permissive,
            thread,
            &basis,
            SecurityLevel::Aggressive,
            Probes::Passed,
            /*now_unix_seconds*/ 1_000,
        )?;
        assert_eq!(report.kind, LevelChangeKind::Stricter);
        assert!(report.live, "Aggressive applies in this session now");
    }
    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "write the file".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    let mut asked = false;
    while let EventMsg::ExecApprovalRequest(approval) = wait_for_event(&test.codex, |event| {
        matches!(
            event,
            EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_)
        )
    })
    .await
    {
        asked = true;
        test.codex
            .submit(Op::ExecApproval {
                id: approval.effective_approval_id(),
                turn_id: None,
                decision: ReviewDecision::denied("not now"),
            })
            .await?;
    }
    Ok(Run {
        asked,
        wrote: test.cwd_path().join("issue505.txt").exists(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_505_confirming_aggressive_requires_a_persons_approval_at_once() -> anyhow::Result<()>
{
    skip_if_no_network!(Ok(()));
    let confirmed = run(/*confirm*/ true).await?;
    assert!(confirmed.asked, "a person is asked before the command runs");
    assert!(!confirmed.wrote, "declined: the command did not run");
    Ok(())
}

/// Control: Permissive with approvals off, unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_505_permissive_sessions_keep_their_approval_policy() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    let permissive = run(/*confirm*/ false).await?;
    assert!(!permissive.asked);
    assert!(permissive.wrote);
    Ok(())
}
