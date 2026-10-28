//! #1422: aliases an action defines (`alias b=bash`) and runs (`b run.sh`;
//! bash with `expand_aliases`, zsh and sh expand them). Neither reading
//! expands them: the word lexer ignores the quotes of a definition (`alias
//! e='true;eval'`) and of a use, and the quote-aware reading's checks go by
//! command name. So a word naming an alias the action defines anywhere
//! asks, wherever it is (`eval 'b run.sh'`, `f() { b; }; alias b=bash; f`),
//! as does a change to the alias tables the readings cannot name (`aliases=(e
//! eval)`, zsh suffix aliases).

use super::shell::SimpleCommand;
use super::shell::basename;
use super::work;
use std::collections::HashSet;

/// The parameters that hold aliases: bash's, and zsh's regular, global and
/// suffix ones.
const TABLES: &[&str] = &["BASH_ALIASES", "aliases", "galiases", "saliases"];

#[derive(Default)]
pub(super) struct Aliases {
    /// Lowercase names the action defines.
    names: HashSet<String>,
    /// It changes an alias table in a way the readings cannot name.
    opaque: bool,
}

impl Aliases {
    /// The aliases `commands` (a line's, as written) define. Any `alias`
    /// word starts definitions (`x='a b' alias e=eval`, `eval alias
    /// e=eval`): each later `name=value` word defines `name`; `alias -s`
    /// (zsh suffix aliases) is opaque. `BASH_ALIASES[e]=`, zsh
    /// `aliases[e]=` and `galiases[e]=` define `e`; any other assignment to
    /// a table, or a reference to one (`declare -n r=BASH_ALIASES`), is
    /// opaque.
    pub(super) fn of(commands: &[SimpleCommand]) -> Self {
        let mut aliases = Self::default();
        for command in commands {
            work::step();
            let mut defining = false;
            for word in &command.words {
                work::step();
                aliases.table_word(word);
                if basename(word).eq_ignore_ascii_case("alias") {
                    defining = true;
                } else if defining && word.starts_with('-') {
                    aliases.opaque |= word.contains('s');
                } else if defining
                    && let Some((name, _)) = word.split_once('=')
                    && !name.is_empty()
                {
                    aliases.names.insert(name.to_lowercase());
                }
            }
        }
        aliases
    }

    /// Whether `words` (a simple command's, values filled in) name an
    /// alias the action defines, or the action changes a table opaquely.
    pub(super) fn used(&self, words: &[String]) -> bool {
        self.opaque
            || (!self.names.is_empty()
                && words.iter().any(|word| {
                    work::step();
                    self.names.contains(&basename(word).to_lowercase())
                }))
    }

    fn table_word(&mut self, word: &str) {
        // A reference to a table (`declare -n r=aliases`).
        if let Some((_, value)) = word.split_once('=')
            && TABLES.contains(&value)
        {
            self.opaque = true;
        }
        let Some(table) = TABLES.iter().find(|table| word.starts_with(**table)) else {
            return;
        };
        let rest = &word[table.len()..];
        if let Some(subscript) = rest.strip_prefix('[') {
            let Some((key, after)) = subscript.split_once(']') else {
                return;
            };
            if !(after.starts_with('=') || after.starts_with("+=")) {
                return;
            }
            if *table == "saliases" || key.is_empty() {
                self.opaque = true;
            } else {
                self.names.insert(key.to_lowercase());
            }
        } else if rest.starts_with('=') || rest.starts_with("+=") {
            self.opaque = true;
        }
    }
}
