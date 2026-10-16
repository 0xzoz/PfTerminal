//! #826 follow-up (#876, #891): on Linux, agent commands under security
//! levels Moderate and Aggressive (and contained Claude panes under them)
//! cannot create user namespaces. A command that needed one fails with a
//! bare "Operation not permitted"; this names the rule instead.
//!
//! Text only: nothing here decides what runs. A match adds an explanation to
//! a command that already failed under the rule.

/// Tools that need a user namespace, as the explanations list them.
const TOOLS: &str =
    "rootless Podman, bwrap, unshare, Flatpak, Nix's build sandbox and Chrome's sandbox";

/// Lowercase fragments of a line that reports a namespace (or the clone
/// that makes one) as what failed: `unshare failed`, bubblewrap's "creating
/// new namespace", Chrome's "move to new namespace", rootless Podman's
/// `cannot clone`, Nix's `cloning builder process`, a uid map write.
const NAMESPACE_FRAGMENTS: [&str; 9] = [
    "unshare",
    "namespace",
    "cannot clone",
    "clone3",
    "cloning builder process",
    "uid map",
    "uid_map",
    "newuidmap",
    "userns",
];

/// Lowercase fragments of a line that reports a refusal. Bubblewrap says
/// "No permissions to create(ing) new namespace".
const REFUSAL_FRAGMENTS: [&str; 3] = [
    "operation not permitted",
    "permission denied",
    "no permissions to creat",
];

/// Whether a line of `output` reports that creating a user namespace (or
/// the clone that makes one) was refused.
pub fn reports_user_namespace_denial(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        NAMESPACE_FRAGMENTS
            .iter()
            .any(|fragment| line.contains(fragment))
            && REFUSAL_FRAGMENTS
                .iter()
                .any(|fragment| line.contains(fragment))
    })
}

/// Programs that always need a user namespace, by file name.
const NAMESPACE_PROGRAMS: [&str; 5] = ["unshare", "bwrap", "podman", "buildah", "flatpak"];

/// Whether `program` (a path or a file name) is one of
/// [`NAMESPACE_PROGRAMS`].
pub fn needs_user_namespace(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    NAMESPACE_PROGRAMS.contains(&name)
}

/// Where a command failed under the rule, at security level `level`
/// ("Moderate" or "Aggressive").
pub fn denial_explanation(level: &str) -> String {
    format!(
        "Security level {level} blocked creating a user namespace: on Linux, agent commands \
         cannot create user namespaces under Moderate and Aggressive, and tools such as {TOOLS} \
         need one, so they fail with \"Operation not permitted\". No approval or grant lifts \
         this. To use them, a person can choose Permissive in /security (it applies after a \
         restart)."
    )
}

/// Before a person approves a command that runs a [`NAMESPACE_PROGRAMS`]
/// program at security level `level`.
pub fn approval_note(level: &str) -> String {
    format!(
        "On Linux, security level {level} blocks creating user namespaces for agent commands: \
         if you approve, the command runs, but what needs a user namespace ({TOOLS}) fails with \
         \"Operation not permitted\", and no approval or grant lifts this. To use them, a person \
         can choose Permissive in /security (it applies after a restart)."
    )
}

/// After a contained Claude pane turn in which a command reported the
/// refusal, at security level `level`.
pub fn claude_pane_note(level: &str) -> String {
    format!(
        "Corbanu Terminal: a command in this turn needed a user namespace, which security level \
         {level} blocks in contained Claude panes on Linux. Claude Code's own Bash sandbox \
         (bubblewrap) and tools such as {TOOLS} cannot start here and fail with \"Operation not \
         permitted\" (bubblewrap says \"No permissions to create new namespace\"); Corbanu's \
         sandbox still applies to every command. No approval lifts this. To use them, choose \
         Permissive in /security (it applies after a restart)."
    )
}

#[cfg(test)]
#[path = "user_namespaces_tests.rs"]
mod tests;
