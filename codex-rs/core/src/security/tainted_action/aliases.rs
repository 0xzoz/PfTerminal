//! #1422: aliases an action defines (`alias b=bash`) and runs (`b run.sh`;
//! bash with `expand_aliases`, zsh and sh expand them). Neither reading
//! expands them: the word lexer ignores the quotes of a definition (`alias
//! e='true;eval'`) and of a use, and the quote-aware reading's checks go by
//! command name. So a word naming an alias the line defines asks, wherever
//! it is (`eval 'b run.sh'`), even read before the definition it runs
//! after (`f() { b; }; alias b=bash; f`, a trap). A definition that does
//! not name its alias asks (`BASH_ALIASES[$k]=bash`, `aliases=(b bash)`,
//! `declare -n r=BASH_ALIASES`, zsh suffix aliases). An alias defined but
//! not used stays quiet (`alias gs='git status'`).

use super::indirect;
use super::shell::UNSEEN_OUTPUT;
use super::shell::basename;
use super::work;
use std::collections::HashSet;

/// The parameters that hold aliases: bash's, and zsh's regular, global and
/// suffix ones.
const TABLES: &[&str] = &["BASH_ALIASES", "aliases", "galiases", "saliases"];

#[derive(Default)]
pub(super) struct Aliases {
    /// Lowercase names the line defines.
    names: HashSet<String>,
    /// A definition the readings cannot name.
    opaque: bool,
}

impl Aliases {
    /// The aliases `commands` (a line's simple commands, as written)
    /// define, so a use read before its definition counts.
    pub(super) fn of(commands: &[super::shell::SimpleCommand]) -> Self {
        let mut aliases = Self::default();
        for command in commands {
            work::step();
            aliases.define(&command.words, /*as_written*/ true);
        }
        aliases
    }

    /// Read `words`, a simple command's with values filled in (`a=alias;
    /// $a b=bash`, `k=b; BASH_ALIASES[$k]=bash`): whether a word names an
    /// alias the line defines, or a definition cannot be named.
    pub(super) fn read(&mut self, words: &[String]) -> bool {
        self.define(words, /*as_written*/ false);
        self.opaque
            || (!self.names.is_empty()
                && words.iter().any(|word| {
                    work::step();
                    self.names.contains(&basename(word).to_lowercase())
                }))
    }

    /// The definitions in one command's `words`: after an `alias` word (or,
    /// as written, a command word holding a value, which may be `alias`:
    /// `$a`, `al$x`), each `name=value` word. Where the definer is the
    /// command (past `eval`, `builtin`, `command`, or after an assignment,
    /// which the lexer may misread: `x='a b' alias`), a value that may
    /// hold a definition (`alias $1`, `alias "$@"`) asks, as does `-s`
    /// (zsh suffix aliases) before the definitions. Any alias table word
    /// counts too.
    fn define(&mut self, words: &[String], as_written: bool) {
        if self.opaque {
            return;
        }
        let command = indirect::command_word(words).map(|(at, _)| at);
        let valued = command.filter(|at| as_written && unknown(&words[*at]));
        let assigned_before = command.is_some_and(|at| {
            words[..at].iter().any(|word| {
                work::step();
                word.contains('=')
            })
        });
        let mut defining = false;
        let mut commanding = false;
        let mut named = false;
        for (at, word) in words.iter().enumerate() {
            work::step();
            self.table(word);
            if basename(word).eq_ignore_ascii_case("alias") || valued == Some(at) {
                defining = true;
                named = false;
                // A command named by a value defines only `name=value`
                // words (`$PYTHON "$f"` is not `alias "$f"`).
                commanding = valued != Some(at)
                    && (assigned_before
                        || command.is_some_and(|command| {
                            command <= at
                                && words[command..at].iter().all(|word| {
                                    work::step();
                                    matches!(word.as_str(), "eval" | "builtin" | "command")
                                })
                        }));
                continue;
            }
            if !defining {
                continue;
            }
            if word.starts_with('-') {
                // `alias -s sh=bash`: suffix aliases run for file names.
                self.opaque |=
                    commanding && !named && !word.starts_with("--") && word.contains('s');
            } else if let Some((name, _)) = word.split_once('=')
                && !name.is_empty()
            {
                named = true;
                if unknown(name) {
                    self.opaque = true;
                } else {
                    self.names.insert(name.to_lowercase());
                }
            } else if commanding && unknown(word) {
                self.opaque = true;
            }
        }
    }

    /// An alias table in `word`: a subscript anywhere (`BASH_ALIASES[b]`;
    /// `read`, `printf -v` and `${…:=}` write one too) defines its key; a
    /// key not known, a suffix alias, an assignment to the whole table
    /// (`aliases=(…)`) or a reference to one (`r=BASH_ALIASES`) cannot be
    /// named.
    fn table(&mut self, word: &str) {
        for table in TABLES {
            work::step();
            for (at, _) in word.match_indices(table) {
                work::step();
                let rest = &word[at + table.len()..];
                let before = word[..at].chars().next_back();
                // A parameter's name starts the word or follows `${`, `$`,
                // `${!`, `${#` or `=` (not `jq '.aliases[0]'`).
                if !matches!(before, None | Some('{' | '$' | '!' | '#' | '=')) {
                    continue;
                }
                if let Some(subscript) = rest.strip_prefix('[') {
                    // Up to its `]`; one inside another is not read.
                    let key = match subscript.find([']', '[']) {
                        Some(end) if subscript[end..].starts_with(']') => &subscript[..end],
                        _ => {
                            self.opaque = true;
                            return;
                        }
                    };
                    if *table == "saliases" || key.is_empty() || unknown(key) {
                        self.opaque = true;
                    } else {
                        self.names.insert(key.to_lowercase());
                    }
                } else if (at == 0 && (rest.starts_with('=') || rest.starts_with("+=")))
                    || (before == Some('=') && rest.is_empty())
                {
                    self.opaque = true;
                }
            }
        }
    }
}

/// Whether `text` holds a value the readings do not know.
fn unknown(text: &str) -> bool {
    text.contains(['$', '`']) || text.contains(UNSEEN_OUTPUT)
}
