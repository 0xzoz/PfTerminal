//! D50 (#1001): a run approved while a security transition was committed
//! was approved for a stale prompt. Nothing runs on that answer: the run is
//! decided again under the new level, the person asked again (the prompt
//! says why), and refused when that keeps happening.

use super::*;
use crate::session::tests::make_session_and_context_with_rx;
use crate::tools::approvals::ASKED_AGAIN_NOTE;
use crate::tools::sandboxing::Approvable;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_security_policy::PolicyPrincipal;
use codex_security_policy::PrincipalKind;
use codex_security_policy::RevocationState;
use codex_security_policy::SecurityLevel;
use codex_security_policy::SecuritySettings;
use pretty_assertions::assert_eq;
use std::sync::Arc;

fn initialize_policy(
    session: &crate::session::session::Session,
) -> crate::security::TrustedSecurityController {
    crate::security::TrustedSecurityController::initialize(
        &session.services.agent_control.effective_security_policy(),
        crate::security::PersistedHumanSecurityState::new(
            SecuritySettings::new(SecurityLevel::Permissive),
            PolicyPrincipal::new(PrincipalKind::Human, "human:issue-1001").unwrap(),
            RevocationState::new(),
        )
        .unwrap(),
        session.thread_id,
        codex_protocol::SessionId::from(session.thread_id),
        crate::security::EffectivePolicyInitialization::Root,
    )
    .expect("initialize the session's policy tree")
}

/// A security transition is committed: the policy marker moves.
fn change_level(controller: &crate::security::TrustedSecurityController, level: SecurityLevel) {
    let change = controller
        .confirm_level_change(level, RevocationState::new())
        .unwrap();
    controller.apply_confirmed_change(change).unwrap();
}

/// A tool that needs approval. Each ask records the prompt's reason, then
/// commits the next queued transition while the prompt is open (before the
/// answer), and the person approves.
struct Probe {
    controller: crate::security::TrustedSecurityController,
    transitions_while_asked: Vec<SecurityLevel>,
    requirement: ExecApprovalRequirement,
    /// Whether each run, in order, is denied by the sandbox (later ones are
    /// not).
    sandbox_denies: Vec<bool>,
    asked: Vec<Option<String>>,
    runs: usize,
}

impl Approvable<()> for Probe {
    type ApprovalKey = String;

    fn additional_permissions<'a>(
        &self,
        _req: &'a (),
    ) -> Option<&'a codex_protocol::models::AdditionalPermissionProfile> {
        None
    }

    fn approval_keys(&self, _req: &()) -> Vec<Self::ApprovalKey> {
        vec!["issue-1001-probe".to_string()]
    }

    fn exec_approval_requirement(&self, _req: &()) -> Option<ExecApprovalRequirement> {
        Some(self.requirement.clone())
    }

    fn wants_no_sandbox_approval(&self, _policy: AskForApproval) -> bool {
        true
    }

    fn start_approval_async<'a>(
        &'a mut self,
        _req: &'a (),
        ctx: ApprovalCtx<'a>,
    ) -> futures::future::BoxFuture<'a, ReviewDecision> {
        self.asked.push(ctx.retry_reason);
        if !self.transitions_while_asked.is_empty() {
            let level = self.transitions_while_asked.remove(0);
            change_level(&self.controller, level);
        }
        Box::pin(async { ReviewDecision::Approved })
    }

    fn approval_action(&self, _req: &(), ctx: &ApprovalCtx<'_>) -> std::io::Result<ApprovalAction> {
        Ok(ApprovalAction::Shell {
            id: ctx.call_id.to_string(),
            environment_id: codex_exec_server::LOCAL_ENVIRONMENT_ID.to_string(),
            command: vec!["issue-1001-probe".to_string()],
            #[allow(deprecated)]
            cwd: PathUri::from_abs_path(&ctx.turn.cwd),
            sandbox_permissions: crate::sandboxing::SandboxPermissions::UseDefault,
            additional_permissions: None,
            justification: Some("the model's reason".to_string()),
        })
    }
}

impl crate::tools::sandboxing::Sandboxable for Probe {
    fn sandbox_preference(&self) -> codex_sandboxing::SandboxablePreference {
        codex_sandboxing::SandboxablePreference::Auto
    }
}

impl ToolRuntime<(), ()> for Probe {
    fn workspace_roots<'a>(&self, _req: &'a ()) -> &'a [PathUri] {
        &[]
    }

    async fn run(
        &mut self,
        _req: &(),
        _attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx,
    ) -> Result<(), ToolError> {
        let denied = self.sandbox_denies.get(self.runs).copied().unwrap_or(false);
        self.runs += 1;
        if denied {
            return Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output: Box::new(ExecToolCallOutput {
                    exit_code: 1,
                    ..Default::default()
                }),
                network_policy_decision: None,
            })));
        }
        Ok(())
    }
}

fn needs_approval() -> ExecApprovalRequirement {
    ExecApprovalRequirement::NeedsApproval {
        reason: None,
        proposed_execpolicy_amendment: None,
    }
}

async fn run_probe(
    transitions_while_asked: Vec<SecurityLevel>,
    requirement: ExecApprovalRequirement,
    sandbox_denies: Vec<bool>,
    approval_policy: AskForApproval,
) -> (Probe, Result<(), String>) {
    let (session, turn, _events) = make_session_and_context_with_rx().await;
    let controller = initialize_policy(&session);
    let mut probe = Probe {
        controller,
        transitions_while_asked,
        requirement,
        sandbox_denies,
        asked: Vec::new(),
        runs: 0,
    };
    let tool_ctx = ToolCtx {
        session: Arc::clone(&session),
        turn: Arc::clone(&turn),
        call_id: "call-1001".to_string(),
        tool_name: codex_tools::ToolName::plain("issue_1001_probe"),
    };
    let result = ToolOrchestrator::new()
        .run(&mut probe, &(), &tool_ctx, turn.as_ref(), approval_policy)
        .await
        .map(|_| ())
        .map_err(|err| match err {
            ToolError::Rejected(message) | ToolError::Declined(message) => message,
            ToolError::Codex(err) => err.to_string(),
        });
    (probe, result)
}

#[tokio::test]
async fn issue_1001_approval_before_a_level_change_is_asked_again() {
    // Shown under Permissive; the level rose before the answer.
    let (probe, result) = run_probe(
        vec![SecurityLevel::Moderate],
        needs_approval(),
        Vec::new(),
        AskForApproval::OnRequest,
    )
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(probe.asked.len(), 2, "asked again: {:?}", probe.asked);
    assert_eq!(probe.asked[0], None);
    assert_eq!(
        probe.asked[1],
        Some(format!("{ASKED_AGAIN_NOTE} the model's reason"))
    );
    assert_eq!(probe.runs, 1, "ran once, on the second answer");
}

#[tokio::test]
async fn issue_1001_approval_without_a_level_change_runs_as_before() {
    let (probe, result) = run_probe(
        Vec::new(),
        needs_approval(),
        Vec::new(),
        AskForApproval::OnRequest,
    )
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(probe.asked, vec![None]);
    assert_eq!(probe.runs, 1);
}

#[tokio::test]
async fn issue_1001_approval_stale_every_time_is_refused() {
    let (probe, result) = run_probe(
        vec![
            SecurityLevel::Moderate,
            SecurityLevel::Aggressive,
            SecurityLevel::Moderate,
        ],
        needs_approval(),
        Vec::new(),
        AskForApproval::OnRequest,
    )
    .await;
    assert_eq!(result, Err(STALE_APPROVAL_REFUSAL.to_string()));
    assert_eq!(probe.asked.len(), MAX_STALE_APPROVAL_ROUNDS);
    assert_eq!(probe.runs, 0, "nothing ran");
}

/// The retry outside the sandbox after a denial, approved after a security
/// transition: not run on that answer. The run is decided again under the
/// new level (its sandboxed attempt too), and the retry asked about again.
#[tokio::test]
async fn issue_1001_retry_approved_before_a_level_change_is_asked_again() {
    let (probe, result) = run_probe(
        vec![SecurityLevel::Moderate],
        ExecApprovalRequirement::Skip {
            bypass_sandbox: false,
            proposed_execpolicy_amendment: None,
        },
        vec![true, true],
        AskForApproval::UnlessTrusted,
    )
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(probe.asked.len(), 2, "asked again: {:?}", probe.asked);
    assert!(
        probe.asked[1]
            .as_deref()
            .is_some_and(|reason| reason.starts_with(ASKED_AGAIN_NOTE)),
        "{:?}",
        probe.asked
    );
    // Denied, then (decided again) denied and retried.
    assert_eq!(probe.runs, 3);
}
