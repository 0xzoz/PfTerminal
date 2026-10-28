//! #1422: aliases an action defines (`alias b=bash`) and later runs (`b
//! run.sh`; bash with `expand_aliases`, zsh and sh expand them). A command
//! named by one is also judged as the text it stands for, so what that
//! text runs is read: a script (`alias b=bash`, `alias s=source`), code
//! (`alias e=eval`, `alias m=mapfile` with `-C`). The word lexer ignores
//! quotes, so a definition's value is its word and the words after it up
//! to the next assignment (`alias m='mapfile -C eval'`).

use super::indirect;
use super::unknown_folder;
use super::work;
use std::collections::HashMap;

/// Alias expansions followed in one command (`alias a=b b=bash; a`).
const MAX_EXPANSIONS: usize = 8;
/// Bytes of alias values one line expands; past it the line asks, so a
/// long value used many times is not copied each time.
const MAX_EXPANDED_BYTES: usize = 256 * 1024;

pub(super) struct Aliases {
    /// Lowercase name to the words of its value and their length.
    values: HashMap<String, (Vec<String>, usize)>,
    /// Bytes of values still to expand.
    left: usize,
}

impl Default for Aliases {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
            left: MAX_EXPANDED_BYTES,
        }
    }
}

/// A command named by an alias, as the shell runs it.
pub(super) struct Expansion {
    /// The command's text with its aliases replaced.
    pub(super) text: String,
    /// It may change the folder (`alias c=cd`), which is not followed.
    pub(super) changes_folder: bool,
    /// Past `MAX_EXPANDED_BYTES`: not expanded, so it asks.
    pub(super) too_long: bool,
}

impl Aliases {
    /// Read `words` (a simple command, values filled in): an `alias`
    /// command defines its names; a command named by one is expanded.
    pub(super) fn read(&mut self, words: &[String]) -> Option<Expansion> {
        let (index, name) = indirect::command_word(words)?;
        if name == "alias" {
            self.define(&words[index + 1..]);
            return None;
        }
        if self.values.is_empty() {
            return None;
        }
        let mut expanded: Vec<String> = words[index..].to_vec();
        let mut used = Vec::new();
        let mut changes_folder = false;
        while used.len() < MAX_EXPANSIONS {
            work::step();
            let Some((at, name)) = indirect::command_word(&expanded) else {
                break;
            };
            if used.contains(&name) {
                break;
            }
            let Some((value, bytes)) = self.values.get(&name) else {
                break;
            };
            if *bytes > self.left {
                self.left = 0;
                return Some(Expansion {
                    text: String::new(),
                    changes_folder: true,
                    too_long: true,
                });
            }
            self.left -= bytes;
            changes_folder |= value.iter().any(|word| {
                work::step();
                unknown_folder::CHANGERS.contains(&super::shell::basename(word))
            });
            expanded.splice(at..=at, value.iter().cloned());
            used.push(name);
        }
        if used.is_empty() {
            return None;
        }
        Some(Expansion {
            text: expanded.join(" "),
            changes_folder,
            too_long: false,
        })
    }

    fn define(&mut self, args: &[String]) {
        for (at, word) in args.iter().enumerate() {
            work::step();
            let Some((name, value)) = word.split_once('=') else {
                continue;
            };
            if name.is_empty() || name.starts_with('-') {
                continue;
            }
            let mut words = vec![value.to_string()];
            words.extend(
                args[at + 1..]
                    .iter()
                    .take_while(|word| !word.contains('='))
                    .cloned(),
            );
            words.retain(|word| !word.is_empty());
            let bytes = words.iter().map(|word| word.len() + 1).sum();
            self.values.insert(name.to_lowercase(), (words, bytes));
        }
    }
}
