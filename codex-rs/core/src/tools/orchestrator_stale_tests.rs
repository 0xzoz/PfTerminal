//! D50 (#1001): a run approved while a security transition was committed
//! was approved for a stale prompt. Nothing runs on that answer: the run is
//! decided again, the person asked again (the prompt says why), and refused
//! when that keeps happening, or at once when the transition switched
//! Aggressive on. A retry outside the sandbox approved (or due) before a
//! transition is not run.
//!
//! The transitions here move the session's policy marker; the live level the
//! model client reports stays Permissive unless a test sets it.

use super::*;
use crate::session::session::Session;
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

fn initialize_policy(session: &Session) -> crate::security::TrustedSecurityController {
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

/// What happens while the probe is asked or runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transition {
    /// A security transition is committed: the policy marker moves.
    Commit(SecurityLevel),
    /// One that also switches Aggressive on for the session's live level.
    SwitchAggressiveOn,
}

/// A tool that needs approval (or not). Each ask records the prompt's
/// reason, applies the next queued transition while the prompt is open (so
/// before the answer), and the person approves. Each run applies the next
/// queued run transition and may be denied by the sandbox.
struct Probe {
    session: Arc<Session>,
    controller: crate::security::TrustedSecurityController,
    requirement: ExecApprovalRequirement,
    while_asked: Vec<Transition>,
    while_running: Vec<Transition>,
    /// Whether each run, in order, is denied by the sandbox (later ones are
    /// not).
    sandbox_denies: Vec<bool>,
    asked: Vec<Option<String>>,
    runs: usize,
}

impl Probe {
    fn apply(&self, transition: Transition) {
        let level = match transition {
            Transition::Commit(level) => level,
            Transition::SwitchAggressiveOn => {
                let client = (*self.session.services.model_client())
                    .clone()
                    .with_ingress_level(SecurityLevel::Aggressive);
                self.session.services.replace_model_client(client);
                SecurityLevel::Aggressive
            }
        };
        let change = self
            .controller
            .confirm_level_change(level, RevocationState::new())
            .unwrap();
        self.controller.apply_confirmed_change(change).unwrap();
    }
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
        // A person is shown the prompt (not a remembered answer).
        crate::tools::sandboxing::note_person_asked();
        self.asked.push(ctx.retry_reason);
        if !self.while_asked.is_empty() {
            let transition = self.while_asked.remove(0);
            self.apply(transition);
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
        if !self.while_running.is_empty() {
            let transition = self.while_running.remove(0);
            self.apply(transition);
        }
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

struct Case {
    requirement: ExecApprovalRequirement,
    while_asked: Vec<Transition>,
    while_running: Vec<Transition>,
    sandbox_denies: Vec<bool>,
    approval_policy: AskForApproval,
}

impl Default for Case {
    fn default() -> Self {
        Self {
            requirement: needs_approval(),
            while_asked: Vec::new(),
            while_running: Vec::new(),
            sandbox_denies: Vec::new(),
            approval_policy: AskForApproval::OnRequest,
        }
    }
}

/// Runs the probe; the error is its message, or "sandbox denied".
async fn run_probe(case: Case) -> (Probe, Result<(), String>) {
    let (session, turn, _events) = make_session_and_context_with_rx().await;
    let controller = initialize_policy(&session);
    let mut probe = Probe {
        session: Arc::clone(&session),
        controller,
        requirement: case.requirement,
        while_asked: case.while_asked,
        while_running: case.while_running,
        sandbox_denies: case.sandbox_denies,
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
        .run(
            &mut probe,
            &(),
            &tool_ctx,
            turn.as_ref(),
            case.approval_policy,
        )
        .await
        .map(|_| ())
        .map_err(|err| match err {
            ToolError::Rejected(message) | ToolError::Declined(message) => message,
            ToolError::Codex(err) => match err.details() {
                CodexErrorDetails::Sandbox(SandboxErr::Denied { .. }) => {
                    "sandbox denied".to_string()
                }
                _ => err.to_string(),
            },
        });
    (probe, result)
}

#[tokio::test]
async fn issue_1001_approval_before_a_transition_is_asked_again() {
    let (probe, result) = run_probe(Case {
        while_asked: vec![Transition::Commit(SecurityLevel::Moderate)],
        ..Case::default()
    })
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(
        probe.asked,
        vec![None, Some(format!("{ASKED_AGAIN_NOTE} the model's reason"))]
    );
    assert_eq!(probe.runs, 1, "ran once, on the second answer");
}

#[tokio::test]
async fn issue_1001_approval_without_a_transition_runs_as_before() {
    let (probe, result) = run_probe(Case::default()).await;
    assert_eq!(result, Ok(()));
    assert_eq!(probe.asked, vec![None]);
    assert_eq!(probe.runs, 1);
}

#[tokio::test]
async fn issue_1001_approval_stale_every_time_is_refused() {
    let (probe, result) = run_probe(Case {
        while_asked: vec![
            Transition::Commit(SecurityLevel::Moderate),
            Transition::Commit(SecurityLevel::Aggressive),
            Transition::Commit(SecurityLevel::Moderate),
        ],
        ..Case::default()
    })
    .await;
    assert_eq!(result, Err(STALE_APPROVAL_REFUSAL.to_string()));
    assert_eq!(probe.asked.len(), MAX_STALE_APPROVAL_ROUNDS);
    assert_eq!(probe.runs, 0, "nothing ran");
}

/// The exec-policy decision a run carries was made for the level when it
/// started (under Aggressive, allow rules saved outside it are suspended and
/// vault commands forbidden), so an approval made stale by switching
/// Aggressive on is refused instead of asked again.
#[tokio::test]
async fn issue_1001_approval_before_aggressive_is_switched_on_is_refused() {
    let (probe, result) = run_probe(Case {
        while_asked: vec![Transition::SwitchAggressiveOn],
        ..Case::default()
    })
    .await;
    assert_eq!(result, Err(STALE_AGGRESSIVE_REFUSAL.to_string()));
    assert_eq!(probe.asked.len(), 1);
    assert_eq!(probe.runs, 0, "nothing ran");
}

/// The retry outside the sandbox, approved after a transition: not run, and
/// the run is not decided again (its sandboxed attempt already ran). The
/// sandbox denial stands.
#[tokio::test]
async fn issue_1001_retry_approved_before_a_transition_is_not_run() {
    let (probe, result) = run_probe(Case {
        requirement: ExecApprovalRequirement::Skip {
            bypass_sandbox: false,
            proposed_execpolicy_amendment: None,
        },
        while_asked: vec![Transition::Commit(SecurityLevel::Moderate)],
        sandbox_denies: vec![true],
        approval_policy: AskForApproval::UnlessTrusted,
        ..Case::default()
    })
    .await;
    assert_eq!(result, Err("sandbox denied".to_string()));
    assert_eq!(probe.asked.len(), 1, "the retry was asked about once");
    assert_eq!(probe.runs, 1, "no retry, no second sandboxed attempt");
}

/// A transition while the sandboxed attempt runs: the retry that the first
/// approval would have let run unasked is not run on that approval.
#[tokio::test]
async fn issue_1001_unprompted_retry_after_a_transition_is_not_run() {
    let (probe, result) = run_probe(Case {
        while_running: vec![Transition::Commit(SecurityLevel::Moderate)],
        sandbox_denies: vec![true],
        approval_policy: AskForApproval::UnlessTrusted,
        ..Case::default()
    })
    .await;
    assert_eq!(result, Err("sandbox denied".to_string()));
    assert_eq!(probe.asked, vec![None], "approved once, before it ran");
    assert_eq!(probe.runs, 1, "not retried");

    // Without the transition the same retry runs unasked, as before.
    let (probe, result) = run_probe(Case {
        sandbox_denies: vec![true],
        approval_policy: AskForApproval::UnlessTrusted,
        ..Case::default()
    })
    .await;
    assert_eq!(result, Ok(()));
    assert_eq!(probe.asked, vec![None]);
    assert_eq!(probe.runs, 2);
}
