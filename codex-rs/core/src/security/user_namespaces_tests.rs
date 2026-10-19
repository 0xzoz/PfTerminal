use super::*;
use pretty_assertions::assert_eq;

const REFUSED: &str = "unshare: unshare failed: Operation not permitted";

/// The rule's levels: Moderate and Aggressive, also where only the launch
/// enforces Aggressive; never Permissive alone. Only Linux enforces it.
#[test]
fn issue_826_the_rule_follows_the_protected_levels() {
    let cases = [
        (None, false, None),
        (Some(SecurityLevel::Permissive), false, None),
        (Some(SecurityLevel::Moderate), false, Some("Moderate")),
        (Some(SecurityLevel::Aggressive), false, Some("Aggressive")),
        (None, true, Some("Aggressive")),
        (Some(SecurityLevel::Permissive), true, Some("Aggressive")),
        (Some(SecurityLevel::Moderate), true, Some("Aggressive")),
    ];
    for (level, launch, named) in cases {
        assert_eq!(denying_level(level, launch), named, "{level:?} {launch}");
        assert_eq!(
            rule_applies(level, launch),
            cfg!(target_os = "linux") && named.is_some(),
            "{level:?} {launch}"
        );
    }
}

/// A failed command whose output reports the refusal is explained, naming
/// the level; a command that succeeded, one whose failure is not about a
/// namespace, and any command where the rule does not apply are not.
#[test]
fn issue_826_only_a_refused_namespace_under_the_rule_is_explained() {
    let explanation = explained(Some("Moderate"), Some(1), REFUSED).unwrap();
    assert!(
        explanation.starts_with("Security level Moderate blocks creating user namespaces"),
        "{explanation}"
    );
    assert_eq!(explained(Some("Aggressive"), Some(0), REFUSED), None);
    assert_eq!(
        explained(Some("Aggressive"), /*exit_code*/ None, REFUSED),
        None
    );
    assert_eq!(
        explained(
            Some("Aggressive"),
            Some(1),
            "cat: /home/someone/.ssh/id_ed25519: Operation not permitted"
        ),
        None
    );
    assert_eq!(explained(/*level*/ None, Some(1), REFUSED), None);
}

/// D27 (a): a run a person let create user namespaces is remembered by
/// thread and call only, so a refusal its host reports isn't put down to the
/// rule; another call, or the same call in another thread, still is.
#[test]
fn d27_allowed_runs_are_known_by_thread_and_call() {
    let thread = ThreadId::new();
    let other = ThreadId::new();
    note_allowed_run(thread, "call-d27-allowed");
    assert!(run_was_allowed(thread, "call-d27-allowed"));
    assert!(!run_was_allowed(thread, "call-d27-other"));
    assert!(!run_was_allowed(other, "call-d27-allowed"));
    // Bounded: older runs are forgotten once enough newer ones are noted.
    for n in 0..ALLOWED_RUNS_KEPT {
        note_allowed_run(other, &format!("call-d27-{n}"));
    }
    assert!(!run_was_allowed(thread, "call-d27-allowed"));
    assert!(run_was_allowed(other, "call-d27-0"));
}
