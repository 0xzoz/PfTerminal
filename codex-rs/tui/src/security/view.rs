//! Read-only profile copy. Configuration intent is never proof of protection.

use codex_protocol::security::SecurityLevel;

pub(crate) const PROFILES: [SecurityLevel; 3] = [
    SecurityLevel::Permissive,
    SecurityLevel::Moderate,
    SecurityLevel::Aggressive,
];

pub(crate) fn profile_name(level: SecurityLevel) -> &'static str {
    match level {
        SecurityLevel::Permissive => "Permissive",
        SecurityLevel::Moderate => "Moderate",
        SecurityLevel::Aggressive => "Aggressive",
    }
}

pub(crate) fn profile_summary(level: SecurityLevel) -> &'static str {
    match level {
        SecurityLevel::Permissive => {
            "No additional security controls. Existing approvals, sandbox, vault, wallet, tool, network and agent policies remain unchanged."
        }
        SecurityLevel::Moderate => {
            "Protection around untrusted content and secrets while normal work continues: commands stay in the sandbox, credential and Corbanu files are protected, and protected actions after untrusted content need a person's fresh approval. Chosen in /security with the security_levels feature."
        }
        SecurityLevel::Aggressive => {
            "Sensitive access denied by default: writes only in the current folder, approval for every command, no network, and no vault access for agents. Chosen in /security with the security_levels feature."
        }
    }
}

pub(crate) fn requested_summary(requested: Option<SecurityLevel>) -> String {
    match requested {
        Some(level) => format!("{} (configuration only)", profile_name(level)),
        None => "Unknown — cannot verify configuration".to_string(),
    }
}

pub(crate) fn status_summary(requested: SecurityLevel) -> String {
    if let Some(context) = super::level::context().filter(|context| context.picker_enabled) {
        let line =
            super::level::status_line(context.active, &super::level::load(&context.codex_home));
        return match &context.boundary {
            Some(boundary) => format!("{line}; {}", boundary.summary()),
            None => line,
        };
    }
    format!(
        "Requested {}; effective protection unverified (/security)",
        profile_name(requested)
    )
}
