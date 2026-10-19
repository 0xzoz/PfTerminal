//! #826 follow-up (#876, #891): on Linux, agent commands under security
//! levels Moderate and Aggressive (and contained Claude panes under them)
//! cannot create user namespaces. A command that needed one fails with a
//! bare "Operation not permitted"; this names the rule instead.
//!
//! Text only: nothing here decides what runs. A match adds an explanation to
//! a command that already failed under the rule.
//!
//! D27 (a) (Travis, 2026-10-19, security sign-off): under Moderate a person
//! may allow it for one command, by approving that command's request to run
//! outside the sandbox before untrusted content enters the session
//! ([`moderate_one_run_warning`] is what they are told). Aggressive, and a
//! launch that enforces Aggressive, never allow it.

/// Tools that need a user namespace, as the explanations list them.
const TOOLS: &str =
    "rootless Podman, bwrap, unshare, Flatpak, Nix's build sandbox and Chrome's sandbox";

/// Programs whose own messages (`unshare: …`, `bwrap: …`) are about the
/// namespaces they make, so a refusal they report counts.
const NAMESPACE_PROGRAM_PREFIXES: [&str; 2] = ["unshare:", "bwrap:"];

/// Lowercase message shapes that name a namespace, or the clone that makes
/// one, as what failed: bubblewrap's and Chrome's "new namespace", Corbanu's
/// own "user namespaces", rootless Podman's `cannot clone:`, Nix's `cloning
/// builder process`, a Go program re-executing itself into new namespaces
/// (rootless Docker, rootlesskit), bubblewrap's uid map setup and the
/// setuid map helpers. Shapes, not bare words: a path such as
/// `namespaces/prod.yaml` next to "Permission denied" is not one.
const NAMESPACE_SHAPES: [&str; 10] = [
    "new namespace",
    "user namespace",
    "cannot clone:",
    "cloning builder process",
    "fork/exec /proc/self/exe",
    "setting up uid map",
    "/proc/self/uid_map",
    "/proc/self/gid_map",
    "newuidmap",
    "newgidmap",
];

/// Lowercase fragments of a line that reports a refusal. Bubblewrap says
/// "No permissions to create(ing) new namespace".
const REFUSAL_FRAGMENTS: [&str; 3] = [
    "operation not permitted",
    "permission denied",
    "no permissions to creat",
];

/// Whether a line of `output` reports that creating a user namespace (or
/// the clone that makes one) was refused: a refusal on a line that
/// `unshare` or `bubblewrap` printed, or that has a [`NAMESPACE_SHAPES`]
/// shape.
pub fn reports_user_namespace_denial(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        let named = NAMESPACE_PROGRAM_PREFIXES
            .iter()
            .any(|prefix| line.trim_start().starts_with(prefix))
            || NAMESPACE_SHAPES.iter().any(|shape| line.contains(shape));
        named
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

/// How a person can still run what needs a user namespace at security
/// level `level`: under Moderate, for one command (D27); otherwise only by
/// choosing Permissive.
fn way_out(level: &str) -> &'static str {
    if level == "Moderate" {
        "Under Moderate a person can allow it for one command: when the agent asks to run that \
         command outside the sandbox (before untrusted content enters the session) and the \
         person approves it when asked, that one run may create user namespaces; nothing \
         remembers it. Otherwise choose Permissive in /security (it applies after a restart)."
    } else {
        "No approval or grant lifts this. To use them, a person can choose Permissive in \
         /security (it applies after a restart)."
    }
}

/// Where a command failed under the rule, at security level `level`
/// ("Moderate" or "Aggressive").
pub fn denial_explanation(level: &str) -> String {
    format!(
        "Security level {level} blocks creating user namespaces for agent commands on Linux, \
         and this command's output reports that one was refused: tools such as {TOOLS} need \
         one, so under Moderate and Aggressive they fail with \"Operation not permitted\". {}",
        way_out(level)
    )
}

/// Before a person approves a command that runs a [`NAMESPACE_PROGRAMS`]
/// program at security level `level`, where approving does not lift the
/// rule (an ordinary prompt; under Moderate not a request to run outside the
/// sandbox, which gets [`moderate_one_run_warning`] instead).
pub fn approval_note(level: &str) -> String {
    let lifted = if level == "Moderate" {
        "approving this prompt doesn't lift it. Under Moderate it is allowed only for a command \
         that asks to run outside the sandbox before untrusted content enters the session, \
         when you approve that request. Otherwise a person can choose Permissive in /security \
         (it applies after a restart)."
    } else {
        "no approval or grant lifts this. To use them, a person can choose Permissive in \
         /security (it applies after a restart)."
    };
    format!(
        "On Linux, security level {level} blocks creating user namespaces for agent commands: \
         if you approve, the command runs, but what needs a user namespace ({TOOLS}) fails with \
         \"Operation not permitted\", and {lifted}"
    )
}

/// D27 (a): the warning in Moderate's prompt for a request to run outside
/// the sandbox on Linux, whose approval also lets that one run create user
/// namespaces.
pub fn moderate_one_run_warning() -> String {
    format!(
        "Warning (Linux): approving also lets this one run create user namespaces, which {TOOLS} \
         need. Inside one, this command can get past the file protections that still apply to \
         it (it is root over its own files there and can mount over paths), and a Corbanu it \
         starts may not run at Moderate. Approve only a command you trust; this is for this one \
         run and is never remembered."
    )
}

/// After a contained Claude pane turn in which a command reported the
/// refusal, at security level `level`.
pub fn claude_pane_note(level: &str) -> String {
    format!(
        "Corbanu Terminal: a command in this turn reported a refused user namespace. Security \
         level {level} blocks creating user namespaces in contained Claude panes on Linux, so \
         Claude Code's own Bash sandbox (bubblewrap) and tools such as {TOOLS} cannot start here \
         and fail with \"Operation not permitted\" (bubblewrap says \"No permissions to create \
         new namespace\"); Corbanu's sandbox still applies to every command. No approval lifts \
         this. To use them, choose Permissive in /security (it applies after a restart)."
    )
}

#[cfg(test)]
#[path = "user_namespaces_tests.rs"]
mod tests;
