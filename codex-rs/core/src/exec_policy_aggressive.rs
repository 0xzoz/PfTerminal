//! #626: remembered command approvals under security level Aggressive.
//!
//! A remembered approval ("Yes, and don't ask again for commands that start
//! with …") is an `allow` prefix rule. Rules carried no level, so while
//! Aggressive is in force no `allow` prefix rule approves a command unless a
//! person saved it while Aggressive was in force (Travis, 2026-10-13): such a
//! rule is written to the Corbanu home's `rules/default.rules` with
//! [`APPROVED_UNDER_AGGRESSIVE`] as its justification, and only that file may
//! say so (a project's `.codex/rules` cannot). Every other `allow` prefix rule
//! is suspended, not deleted: the person is asked as usual, the prompt says
//! which remembered approval was not applied, and under Permissive the rules
//! apply as before. `prompt` and `forbidden` rules always apply.

use std::path::Path;

use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::ConfigLayerStackOrdering;
use codex_execpolicy::Decision;
use codex_execpolicy::Evaluation;
use codex_execpolicy::PatternToken;
use codex_execpolicy::Policy;
use codex_execpolicy::PolicyParser;
use codex_execpolicy::PrefixRule;
use codex_execpolicy::RuleMatch;

/// The justification of a rule a person saved while Aggressive was active.
pub(crate) const APPROVED_UNDER_AGGRESSIVE: &str =
    "Remembered while security level Aggressive was active.";

/// The prefixes the Corbanu home's `rules/default.rules` records as approved
/// while Aggressive was active (the only file trusted to say so). A file that
/// is missing or does not parse records none.
pub(crate) async fn load_approved(config_stack: &ConfigLayerStack) -> Vec<Vec<String>> {
    if config_stack.ignore_user_and_project_exec_policy_rules() {
        return Vec::new();
    }
    let Some(folder) = config_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ false,
        )
        .into_iter()
        .filter(|layer| matches!(layer.name, ConfigLayerSource::User { .. }))
        .find_map(codex_config::ConfigLayerEntry::config_folder)
    else {
        return Vec::new();
    };
    let path = crate::exec_policy::default_policy_path(folder.as_path());
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => approved_in(&path, &contents),
        Err(_) => Vec::new(),
    }
}

/// The prefixes of `contents`' `allow` prefix rules marked
/// [`APPROVED_UNDER_AGGRESSIVE`].
pub(crate) fn approved_in(path: &Path, contents: &str) -> Vec<Vec<String>> {
    let mut parser = PolicyParser::new();
    if parser
        .parse(&path.to_string_lossy(), contents)
        .inspect_err(|err| tracing::warn!("rules approved under Aggressive not read: {err}"))
        .is_err()
    {
        return Vec::new();
    }
    let policy = parser.build();
    let mut approved: Vec<Vec<String>> = policy
        .rules()
        .iter_all()
        .flat_map(|(_, rules)| rules)
        .filter_map(|rule| rule.as_any().downcast_ref::<PrefixRule>())
        .filter(|rule| marked(rule))
        .filter_map(prefix_of)
        .collect();
    approved.sort();
    approved.dedup();
    approved
}

fn marked(rule: &PrefixRule) -> bool {
    rule.decision == Decision::Allow
        && rule.justification.as_deref() == Some(APPROVED_UNDER_AGGRESSIVE)
}

/// The words of a rule with one word per position (as saved approvals are).
fn prefix_of(rule: &PrefixRule) -> Option<Vec<String>> {
    std::iter::once(Some(rule.pattern.first.to_string()))
        .chain(rule.pattern.rest.iter().map(|token| match token {
            PatternToken::Single(word) => Some(word.clone()),
            PatternToken::Alts(_) => None,
        }))
        .collect()
}

/// `policy` without its `allow` prefix rules, except those marked
/// [`APPROVED_UNDER_AGGRESSIVE`] whose prefix is in `approved`.
pub(crate) fn suspend_unapproved(policy: &Policy, approved: &[Vec<String>]) -> Policy {
    let mut rules = policy.rules().clone();
    rules.retain(|_, rule| match rule.as_any().downcast_ref::<PrefixRule>() {
        Some(rule) if rule.decision == Decision::Allow => {
            marked(rule) && prefix_of(rule).is_some_and(|prefix| approved.contains(&prefix))
        }
        _ => true,
    });
    Policy::from_parts(
        rules,
        policy.network_rules().to_vec(),
        policy.host_executables().clone(),
    )
}

/// The suspended remembered approvals in `full` (the evaluation without
/// suspension) when they would have approved the command unasked, as the
/// prompt names them; those in `approved` applied and are not named.
pub(crate) fn suspended_prefixes(full: &Evaluation, approved: &[Vec<String>]) -> Vec<Vec<String>> {
    if full.decision != Decision::Allow {
        return Vec::new();
    }
    let mut prefixes: Vec<Vec<String>> = full
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Allow,
                justification,
                ..
            } if !(justification.as_deref() == Some(APPROVED_UNDER_AGGRESSIVE)
                && approved.contains(matched_prefix)) =>
            {
                Some(matched_prefix.clone())
            }
            _ => None,
        })
        .collect();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

/// What the approval prompt says about `prefixes` (see [`suspended_prefixes`]).
pub(crate) fn suspended_note(prefixes: &[Vec<String>]) -> Option<String> {
    let shown = prefixes
        .iter()
        .map(|prefix| {
            let words = shlex::try_join(prefix.iter().map(String::as_str))
                .unwrap_or_else(|_| prefix.join(" "));
            format!("`{}`", crate::security::protected_reads::shown_path(&words))
        })
        .collect::<Vec<_>>();
    if shown.is_empty() {
        return None;
    }
    let (what, were) = if shown.len() == 1 {
        ("A remembered approval", "was")
    } else {
        ("Remembered approvals", "were")
    };
    Some(format!(
        "{what} (don't ask again for commands that start with {}) {were} not applied because \
         security level Aggressive is on: approvals and allow rules saved outside Aggressive are \
         suspended while it is active, so you are asked instead. They are kept and apply again \
         when you return to a less strict level.",
        shown.join(", ")
    ))
}

#[cfg(test)]
#[path = "exec_policy_aggressive_tests.rs"]
mod tests;
