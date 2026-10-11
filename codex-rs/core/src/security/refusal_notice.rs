//! #439: a command that security level Aggressive stopped says so, to the
//! person (a warning in the transcript) and to the model (a note on the
//! command's output), with the way forward. Without it the person saw only
//! "Operation not permitted" or a connection error.
//!
//! Only the sandbox that Corbanu Terminal's Aggressive launch selected counts
//! ([`crate::config::Permissions::launch_enforces_aggressive`]); a sandbox the
//! person configured themselves is not attributed to the level.

use crate::config::Config;

/// Added to the permissions instructions under Aggressive, so the model
/// names the level when it cannot do something (AGG-13: asked for a web
/// search, it tried `curl` instead).
pub(crate) const AGGRESSIVE_INSTRUCTIONS: &str = "The person chose security level Aggressive in Corbanu Terminal's /security. Under it, agent commands stay in the sandbox even when the person approves them: they can write only inside the workspace, have no network access, and can never read protected files (such as .env and keychains), not even with a grant; web search is off, and agents cannot use the Corbanu vault. When a request needs something this blocks, do not try another way around it (for example, do not fetch web pages with curl when web search is off): tell the person that security level Aggressive blocks it and that they can choose another level with /security.";

/// What the Aggressive sandbox stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blocked {
    /// A write outside the project or a read of a protected file.
    Files,
    /// Network access.
    Network,
}

/// Lowercase fragments of common "no network" failures (curl, git, Python,
/// Node, pip). A refused connection is left out: it also means "nothing
/// listens there".
const NETWORK_FAILURES: [&str; 8] = [
    "could not resolve host",
    "couldn't resolve host",
    "failed to connect to",
    "network is unreachable",
    "temporary failure in name resolution",
    "nodename nor servname",
    "name or service not known",
    "getaddrinfo",
];

/// What the Aggressive sandbox stopped in a finished command, if anything.
/// `sandbox_denied` is the sandbox-denial verdict for the command.
pub(crate) fn blocked(
    config: &Config,
    sandbox_denied: bool,
    exit_code: Option<i32>,
    output: &str,
) -> Option<Blocked> {
    if !config.permissions.launch_enforces_aggressive() || exit_code.is_none_or(|code| code == 0) {
        return None;
    }
    let lower = output.to_lowercase();
    if NETWORK_FAILURES
        .iter()
        .any(|failure| lower.contains(failure))
    {
        Some(Blocked::Network)
    } else if sandbox_denied {
        Some(Blocked::Files)
    } else {
        None
    }
}

impl Blocked {
    /// Shown to the person in the transcript.
    pub(crate) fn notice(self) -> &'static str {
        match self {
            Self::Files => {
                "Security level Aggressive blocked this command: agent commands can write only inside this project, even when you approve them, and can never read protected files (such as .env and keychains), not even once. To lift these limits, choose another level with /security."
            }
            Self::Network => {
                "This command looks blocked by security level Aggressive: agent commands have no network access, even when you approve them, and web search is off. To use the network, choose another level with /security."
            }
        }
    }

    /// Added to the command's output for the model.
    pub(crate) fn model_note(self) -> String {
        format!(
            "\n[Corbanu Terminal: {} Tell the person this instead of trying another way around it.]",
            self.notice()
        )
    }
}

#[cfg(test)]
#[path = "refusal_notice_tests.rs"]
mod tests;
