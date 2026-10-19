//! #945: a "don't ask again" answer to a command prompt shown before a
//! security transition is for a stale prompt: the rule is not saved (it
//! would be marked as approved under the new level, #626), and the answer
//! approves the command once.

use super::*;
use crate::session::tests::make_session_and_context_with_rx;
use crate::state::ActiveTurn;
use codex_security_policy::PolicyPrincipal;
use codex_security_policy::PrincipalKind;
use codex_security_policy::RevocationState;
use codex_security_policy::SecurityLevel;
use codex_security_policy::SecuritySettings;
use pretty_assertions::assert_eq;
use std::time::Duration;

const TOOL: &str = "issue-945-tool";

async fn prompt_shown(events: &async_channel::Receiver<Event>) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("event channel open");
            if let EventMsg::ExecApprovalRequest(prompt) = event.msg {
                return prompt.effective_approval_id();
            }
        }
    })
    .await
    .expect("the prompt was shown")
}

fn saved_rules(codex_home: &std::path::Path) -> String {
    std::fs::read_to_string(crate::exec_policy::default_policy_path(codex_home)).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_945_dont_ask_again_after_a_level_change_is_not_saved() {
    let (session, turn_context, events) = make_session_and_context_with_rx().await;
    let controller = crate::security::TrustedSecurityController::initialize(
        &session.services.agent_control.effective_security_policy(),
        crate::security::PersistedHumanSecurityState::new(
            SecuritySettings::new(SecurityLevel::Permissive),
            PolicyPrincipal::new(PrincipalKind::Human, "human:issue-945").unwrap(),
            RevocationState::new(),
        )
        .unwrap(),
        session.thread_id,
        codex_protocol::SessionId::from(session.thread_id),
        crate::security::EffectivePolicyInitialization::Root,
    )
    .expect("initialize the session's policy tree");
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    let codex_home = session.codex_home().await.to_path_buf();
    // The test home may be gone; the rules file must be writable here.
    std::fs::create_dir_all(&codex_home).unwrap();
    let command = vec![TOOL.to_string(), "run".to_string()];
    let amendment = ExecPolicyAmendment::new(command.clone());
    let ask = |call_id: &'static str| {
        let session = Arc::clone(&session);
        let turn_context = Arc::clone(&turn_context);
        let command = command.clone();
        let amendment = amendment.clone();
        tokio::spawn(async move {
            #[allow(deprecated)]
            let cwd = turn_context.cwd.clone();
            session
                .request_command_approval(
                    turn_context.as_ref(),
                    call_id.to_string(),
                    /*approval_id*/ None,
                    /*environment_id*/ None,
                    command,
                    cwd,
                    /*reason*/ None,
                    /*network_approval_context*/ None,
                    Some(amendment),
                    /*additional_permissions*/ None,
                    /*available_decisions*/ None,
                    /*plugin_attribution_override*/ None,
                )
                .await
        })
    };
    let dont_ask_again = ReviewDecision::ApprovedExecpolicyAmendment {
        proposed_execpolicy_amendment: amendment.clone(),
    };

    // Shown under Permissive, answered after the level rose.
    let approval = ask("call-945-stale");
    let id = prompt_shown(&events).await;
    let change = controller
        .confirm_level_change(SecurityLevel::Aggressive, RevocationState::new())
        .unwrap();
    controller.apply_confirmed_change(change).unwrap();
    handlers::exec_approval(&session, id, None, dont_ask_again.clone()).await;
    assert_eq!(approval.await.unwrap(), ReviewDecision::Approved);
    assert!(
        !saved_rules(&codex_home).contains(TOOL),
        "no rule was saved"
    );
    let warned = std::iter::from_fn(|| events.try_recv().ok()).any(|event| {
        matches!(
            event.msg,
            EventMsg::Warning(WarningEvent { message }) if message.contains("security level changed")
        )
    });
    assert!(warned, "the person is told why");

    // Shown and answered under the level in force: saved as before.
    let approval = ask("call-945-current");
    let id = prompt_shown(&events).await;
    handlers::exec_approval(&session, id, None, dont_ask_again.clone()).await;
    assert_eq!(approval.await.unwrap(), dont_ask_again);
    let warnings: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event.msg {
            EventMsg::Warning(WarningEvent { message }) => Some(message),
            _ => None,
        })
        .collect();
    assert!(saved_rules(&codex_home).contains(TOOL), "{warnings:?}");
}
