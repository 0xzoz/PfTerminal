//! #1422: aliases an action defines (`alias b=bash`) and later runs (`b
//! run.sh`; bash with `expand_aliases`, zsh and sh expand them). Neither
//! reading expands them: the word lexer ignores the quotes of a definition
//! (`alias e='true;eval'`) and of a use, and the quote-aware reading's
//! checks go by command name. So a word naming an alias the action defined
//! asks, wherever it is (`eval 'b run.sh'`), as does a definition the
//! readings cannot name (`BASH_ALIASES[e]=eval` names `e`; `BASH_ALIASES=(…)`
//! asks).

use super::shell::basename;
use super::work;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Aliases {
    /// Lowercase names defined so far.
    names: HashSet<String>,
}

impl Aliases {
    /// Read `words`, a simple command's (values filled in): whether it uses
    /// an alias defined before or defines aliases the readings cannot name.
    /// Any `alias` word starts definitions (`x='a b' alias e=eval`, `eval
    /// alias e=eval`): each later `name=value` word defines `name`.
    pub(super) fn read(&mut self, words: &[String]) -> bool {
        let mut asks = false;
        let mut defining = false;
        let mut defined = Vec::new();
        for word in words {
            work::step();
            let lower = word.to_lowercase();
            if !self.names.is_empty() && self.names.contains(basename(&lower)) {
                asks = true;
            }
            if let Some(rest) = word
                .strip_prefix("BASH_ALIASES")
                .or_else(|| word.strip_prefix("aliases["))
            {
                // `BASH_ALIASES[e]=eval`, zsh `aliases[e]=eval`.
                match rest
                    .strip_prefix('[')
                    .unwrap_or(rest)
                    .split_once(']')
                    .filter(|(name, _)| !name.is_empty())
                {
                    Some((name, _)) if word.starts_with("aliases[") || rest.starts_with('[') => {
                        defined.push(name.to_lowercase());
                    }
                    _ => asks = true,
                }
            }
            if basename(&lower) == "alias" {
                defining = true;
            } else if defining
                && let Some((name, _)) = word.split_once('=')
                && !name.is_empty()
                && !name.starts_with('-')
            {
                defined.push(name.to_lowercase());
            }
        }
        self.names.extend(defined);
        asks
    }
}
