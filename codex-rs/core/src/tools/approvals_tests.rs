use super::*;
use codex_protocol::approvals::NetworkPolicyAmendment;
use pretty_assertions::assert_eq;

#[test]
fn approval_resolution_rejects_denied_network_policy_amendment() {
    let resolution = ApprovalResolution {
        decision: ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment: NetworkPolicyAmendment {
                host: "denied.example.com".to_string(),
                action: NetworkPolicyRuleAction::Deny,
            },
        },
        source: ApprovalResolutionSource::User,
    };
    assert!(matches!(
        resolution.into_tool_result(),
        Err(ToolError::Rejected(rejection)) if rejection == "rejected by user"
    ));
}

#[test]
fn guardian_cwd_preserves_drive_shaped_local_posix_path() {
    let native_cwd = AbsolutePathBuf::try_from(std::path::PathBuf::from("/C:/workspace"))
        .expect("drive-shaped POSIX path should be absolute");
    let cwd = PathUri::from_abs_path(&native_cwd);

    assert_eq!(
        guardian_cwd(codex_exec_server::LOCAL_ENVIRONMENT_ID, cwd)
            .expect("local cwd should retain the host path convention"),
        native_cwd
    );
}

#[test]
fn guardian_cwd_rejects_foreign_remote_path() {
    let cwd = PathUri::parse("file:///C:/workspace").expect("valid Windows path URI");

    assert!(guardian_cwd(codex_exec_server::REMOTE_ENVIRONMENT_ID, cwd).is_err());
}

/// #437: what the inspector lists for each refused approval.
#[test]
fn sec_437_refused_approvals_are_described_for_the_inspector() {
    let resolution = |decision, source| ApprovalResolution { decision, source };
    for (decision, source, expected) in [
        (
            ReviewDecision::Abort,
            ApprovalResolutionSource::User,
            Some("declined by you"),
        ),
        (
            ReviewDecision::denied("no"),
            ApprovalResolutionSource::Hook,
            Some("declined by a permission hook"),
        ),
        (
            ReviewDecision::denied("no"),
            ApprovalResolutionSource::Guardian,
            Some("declined by the automatic reviewer"),
        ),
        (
            ReviewDecision::TimedOut,
            ApprovalResolutionSource::Guardian,
            Some("not answered in time"),
        ),
        (ReviewDecision::Approved, ApprovalResolutionSource::User, None),
    ] {
        assert_eq!(decline_outcome(&resolution(decision, source)), expected);
    }
    assert_eq!(approval_kind("exec_command"), "command approval");
    assert_eq!(approval_kind("apply_patch"), "file edit approval");
    assert_eq!(approval_kind("mcp__x__y"), "tool approval");
}
