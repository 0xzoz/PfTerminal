//! #747 (Travis, 2026-10-14, decision D3): under security level Moderate no
//! command runs outside the OS sandbox, through the orchestrator and the real
//! sandbox (Seatbelt on macOS, bubblewrap on Linux): not a request to run
//! outside it that a person approves, not the "retry without sandbox?" after
//! a sandbox denial, and not a remembered exec-policy rule (D6: it still
//! skips the prompt). The prompt and the model's instructions say so.
//! Permissive is unchanged.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::Path;
use std::sync::Arc;

use codex_core::config::Constrained;
use codex_features::Feature;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::NetworkSandboxPolicy;
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
use serde_json::json;
use tempfile::TempDir;

/// Shown on the prompt of a request to run outside the sandbox under
/// Moderate.
const STAYS: &str = "still runs inside the sandbox";
/// In the model's permissions instructions under Moderate.
const INSTRUCTED: &str = "commands always run inside the sandbox";

/// How the command asks to leave the sandbox, if it does.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// `sandbox_permissions: require_escalated`, approved by the person.
    Escalated,
    /// An ordinary command under `untrusted`, approved by the person; after
    /// the sandbox denies it, the retry without the sandbox needs no new
    /// answer.
    Retry,
    /// An exec-policy allow rule saved earlier ("don't ask again").
    Rule,
}

struct Run {
    /// Approval prompts shown.
    asked: usize,
    /// The prompts' reasons.
    reasons: Vec<String>,
    /// Whether the command wrote outside the writable folder.
    wrote: bool,
    /// What the model got back.
    output: String,
    /// Whether the model was told that Moderate keeps commands in the sandbox.
    instructed: bool,
}

async fn run(level: SecurityLevel, route: Route) -> anyhow::Result<Run> {
    let outside = TempDir::new()?;
    let target = outside.path().canonicalize()?.join("notes.txt");
    // No redirection: an exec-policy rule applies only to commands it can
    // parse.
    let command = format!("touch '{}'", target.display());
    let mut arguments = json!({ "command": command });
    if matches!(route, Route::Escalated) {
        arguments["sandbox_permissions"] = json!("require_escalated");
        arguments["justification"] = json!("write the notes");
    }
    let home = Arc::new(TempDir::new()?);
    if matches!(route, Route::Rule) {
        std::fs::create_dir_all(home.path().join("rules"))?;
        std::fs::write(
            home.path().join("rules/policy.rules"),
            "prefix_rule(pattern=[\"touch\"], decision=\"allow\")\n",
        )?;
    }
    let server = start_mock_server().await;
    let captured = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("call-0"),
                ev_function_call("call-0", "shell_command", &arguments.to_string()),
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
    let approval = match route {
        Route::Escalated | Route::Rule => AskForApproval::OnRequest,
        Route::Retry => AskForApproval::UnlessTrusted,
    };
    let test = test_codex()
        .with_home(home)
        .with_model("test-gpt-5.1-codex")
        .with_config(move |config| {
            config.security_level = level;
            config.permissions.approval_policy = Constrained::allow_any(approval);
            // Writes only in the current folder (not /tmp or $TMPDIR, where
            // `outside` is); network off.
            let folder = config.cwd.clone();
            config
                .permissions
                .set_permission_profile(PermissionProfile::workspace_write_with(
                    &[folder],
                    NetworkSandboxPolicy::Restricted,
                    /*exclude_tmpdir_env_var*/ true,
                    /*exclude_slash_tmp*/ true,
                ))
                .expect("workspace write");
            // Moderate needs the untrusted-content labels.
            config
                .features
                .enable(Feature::SourceEnvelopes)
                .expect("enable source envelopes");
        })
        // Host-sandbox security test: local executor on purpose, also in the
        // remote lane (#708).
        .build(&server)
        .await?;
    std::mem::forget(server);
    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "write the notes".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    let mut asked = 0;
    let mut reasons = Vec::new();
    let mut errors = Vec::new();
    loop {
        let approval = match wait_for_event(&test.codex, |event| {
            matches!(
                event,
                EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_) | EventMsg::Error(_)
            )
        })
        .await
        {
            EventMsg::ExecApprovalRequest(approval) => approval,
            EventMsg::Error(error) => {
                errors.push(error.message);
                continue;
            }
            _ => break,
        };
        reasons.push(approval.reason.clone().unwrap_or_default());
        asked += 1;
        test.codex
            .submit(Op::ExecApproval {
                id: approval.effective_approval_id(),
                turn_id: None,
                decision: ReviewDecision::Approved,
            })
            .await?;
    }
    let requests = captured.requests();
    anyhow::ensure!(
        !requests.is_empty(),
        "{level:?} {route:?}: the model was never asked: {errors:?}"
    );
    let output = requests
        .last()
        .and_then(|request| request.function_call_output_text("call-0"))
        .unwrap_or_default();
    Ok(Run {
        asked,
        reasons,
        wrote: written(&target),
        output,
        instructed: requests[0].body_contains_text(INSTRUCTED),
    })
}

fn written(target: &Path) -> bool {
    target.exists()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_747_moderate_never_runs_a_command_outside_the_sandbox() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    let mut wrong = Vec::new();
    // (level, route, prompts expected, writes outside)
    for (level, route, asks, writes) in [
        (SecurityLevel::Moderate, Route::Escalated, 1, false),
        // The sandbox denies the write; no retry outside it is offered.
        (SecurityLevel::Moderate, Route::Retry, 1, false),
        // D6: the remembered rule still skips the prompt, inside the sandbox.
        (SecurityLevel::Moderate, Route::Rule, 0, false),
        (SecurityLevel::Permissive, Route::Escalated, 1, true),
        // Approved once, its retry outside the sandbox runs unasked.
        (SecurityLevel::Permissive, Route::Retry, 1, true),
        (SecurityLevel::Permissive, Route::Rule, 0, true),
    ] {
        let run = run(level, route).await?;
        let moderate = level == SecurityLevel::Moderate;
        // Only a request to leave the sandbox gets the note on its prompt.
        let noted = moderate && matches!(route, Route::Escalated);
        if run.asked != asks
            || run.wrote != writes
            || run.reasons.iter().any(|reason| reason.contains(STAYS)) != noted
            || run.instructed != moderate
            || run.output.contains("Protected launch refused")
        {
            wrong.push(format!(
                "{level:?} {route:?}: asked={} wrote={} instructed={} \
                 reasons={:?} output={:?}",
                run.asked, run.wrote, run.instructed, run.reasons, run.output
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
    Ok(())
}
