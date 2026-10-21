//! #1268: git run by a launcher that adds words from its input: `xargs`,
//! `parallel` (also its `:::` arguments), `find -exec`/`-execdir`/`-ok`/
//! `-okdir`, `fd -x`/`-X`. Those words are unseen, and may be git's own
//! options (`printf -- '-c core.pager=sh log' | xargs git`) or what `git
//! config` writes (`printf 'filter.a.clean /tmp/x' | xargs git config`).
//! So is a positional parameter (`xargs sh -c 'git config "$@"' _`).
//!
//! Words added at the end of a line whose subcommand git has already read
//! are that subcommand's, so `xargs git log --oneline` stays quiet. Fails
//! closed otherwise: git with no subcommand yet, or an option first (the
//! word lexer may split quoted text, so where the subcommand starts is
//! certain only when it is the first word), or a first word input may fill
//! (`xargs -I{} git {} log`), asks as code the classifier cannot see; `git
//! config` given input words asks as a write unless its visible words
//! already make it a read (`xargs git config --get user.name`).

use super::ProtectedActionKind;
use super::outputs;
use super::shell::basename;
use super::work;

/// Replacement strings (`xargs -I R`) remembered; past this many, any word
/// may be one.
const MAX_REPLACEMENTS: usize = 8;

/// The launchers met so far in one simple command's words, and what the
/// git checks need of the words, worked out once.
pub(super) struct LaunchedGit<'a> {
    /// The command's words, as written and lowercase.
    words: &'a [String],
    lower: &'a [String],
    /// The line as written holds a brace, so a word may have held a
    /// placeholder (`{}`, `{.}`) the word lexer dropped.
    braces: bool,
    /// A launcher that adds words from its input runs what follows.
    fed: bool,
    /// A `find` or `fd`, whose exec options start such a command.
    finder: bool,
    /// The next word may be a replacement string (`xargs -I R`).
    replacement_next: bool,
    replacements: Vec<String>,
    /// More replacement strings than remembered.
    any_replacement: bool,
    /// For each word (and the end): whether, as the words after `config`,
    /// they read, and whether one of them is a positional parameter.
    seen: Option<(Vec<bool>, Vec<bool>)>,
}

impl<'a> LaunchedGit<'a> {
    pub(super) fn new(words: &'a [String], lower: &'a [String], braces: bool) -> Self {
        Self {
            words,
            lower,
            braces,
            fed: false,
            finder: false,
            replacement_next: false,
            replacements: Vec::new(),
            any_replacement: false,
            seen: None,
        }
    }

    /// Takes in `words[index]` after it is judged.
    pub(super) fn step(&mut self, index: usize) {
        let word = self.words[index].as_str();
        let name = basename(&self.lower[index]);
        if std::mem::take(&mut self.replacement_next) {
            self.replace_with(word);
        }
        if self.fed {
            self.replacement_option(word);
        }
        if matches!(name, "xargs" | "parallel")
            || (self.finder
                && matches!(
                    word,
                    "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-x"
                        | "-X"
                        | "--exec"
                        | "--exec-batch"
                ))
        {
            self.fed = true;
        }
        self.finder |= matches!(name, "find" | "fd" | "fdfind");
    }

    /// A launcher's replacement-string option: `xargs -I R`, `-IR`, `-i`,
    /// `-iR`, `--replace[=R]` (or a prefix of it), BSD `-J R`, `parallel -I
    /// R`, in a group of short options too (`-0I%`). Read loosely: a
    /// string taken for one that is not only makes more words input.
    fn replacement_option(&mut self, word: &str) {
        if let Some(long) = word.strip_prefix("--") {
            let (option, value) = long.split_once('=').unwrap_or((long, ""));
            if !option.is_empty() && "replace".starts_with(option) && option.starts_with('r') {
                self.replace_with(value);
                self.replacement_next = value.is_empty();
            }
        } else if let Some(group) = word.strip_prefix('-')
            && let Some(at) = group.find(['I', 'J', 'i'])
        {
            let value = &group[at + 1..];
            self.replace_with(value);
            self.replacement_next = value.is_empty();
        }
    }

    fn replace_with(&mut self, replacement: &str) {
        if replacement.is_empty() {
            return;
        }
        if self.replacements.len() == MAX_REPLACEMENTS {
            self.any_replacement = true;
        } else {
            self.replacements.push(replacement.to_string());
        }
    }

    /// Whether input may fill `word` (a placeholder, which the word lexer
    /// drops, a replacement string, `parallel`'s `:::` arguments).
    fn input_fills(&self, word: &str) -> bool {
        self.braces
            || self.any_replacement
            || word.starts_with(":::")
            || self
                .replacements
                .iter()
                .any(|replacement| word.contains(replacement.as_str()))
    }

    /// The kind `words[index]` asks as when it may run git (`helper`: what
    /// [`super::git_helpers::git_name`] made of it).
    pub(super) fn judge(
        &mut self,
        index: usize,
        helper: Option<&Option<String>>,
    ) -> Option<ProtectedActionKind> {
        let name = basename(&self.lower[index]);
        if helper.is_none() && name != "git" {
            // A dashed helper whose name lost a placeholder (`git-{}`).
            let dropped = matches!(name, "git-" | "git-.exe");
            return (dropped && self.fed && self.braces).then_some(ProtectedActionKind::UnseenCode);
        }
        let config = |this: &mut Self, from: usize| {
            let fed = this.fed;
            let (reads, positional) = this.seen();
            ((fed || positional[from]) && !reads[from]).then_some(ProtectedActionKind::Persistence)
        };
        if let Some(Some(subcommand)) = helper {
            // A dashed helper: its name gives the subcommand.
            if self.fed && self.input_fills(&self.words[index]) {
                return Some(ProtectedActionKind::UnseenCode);
            }
            return if subcommand == "config" {
                config(self, index + 1)
            } else {
                None
            };
        }
        match self.words.get(index + 1) {
            None => self.fed.then_some(ProtectedActionKind::UnseenCode),
            Some(first) if self.fed && (first.starts_with('-') || self.input_fills(first)) => {
                Some(ProtectedActionKind::UnseenCode)
            }
            Some(_) if self.lower[index + 1] == "config" => config(self, index + 2),
            Some(_) => None,
        }
    }

    fn seen(&mut self) -> &(Vec<bool>, Vec<bool>) {
        let words = self.lower;
        self.seen.get_or_insert_with(|| {
            let reads = outputs::git_config_reads_each(words);
            let mut positional = vec![false; words.len() + 1];
            for (at, word) in words.iter().enumerate().rev() {
                work::step();
                positional[at] = positional[at + 1] || holds_positional(word);
            }
            (reads, positional)
        })
    }
}

/// Whether `word` holds a positional parameter (`$1`, `"$@"`, `${*}`),
/// which the classifier never sees (the substitution step keeps them as
/// written).
fn holds_positional(word: &str) -> bool {
    word.match_indices('$').any(|(at, _)| {
        let after = &word[at + 1..];
        let after = after.strip_prefix('{').unwrap_or(after);
        after
            .chars()
            .next()
            .is_some_and(|ch| ch == '@' || ch == '*' || ch.is_ascii_digit())
    })
}
