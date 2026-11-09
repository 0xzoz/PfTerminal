//! #1912: a glob may match a file whose name is an option. `tar cf a.tar *`
//! in a folder holding files named `--checkpoint=1` and
//! `--checkpoint-action=exec=sh x` runs `sh x`; so does `rg data *` with
//! `--pre=sh`, `sed -i s/a/b/ *` with `--expression=1e sh x`, and `sort`'s
//! and `rsync`'s program options. The GNU tools read options after their
//! operands; BSD `tar` and `sed` read them up to the first operand, which a
//! name starting with `-` usually sorts before. So a glob before `--` that
//! may make such a word asks, as one the line does not show. `./*`, `src/*`, `"$PWD"/*` and `--exclude=*.log`
//! cannot start with an option name; `*`, `*.txt`, `-*`, `?*` and `[-]*`
//! can (for `rg` and `sort`, only a glob that ends in its pattern: see
//! [`PROGRAM_RUNNERS`]). Git's transports are judged in `rerun`.
//!
//! D484 (b): a value the line does not show asks only in two forms. A loop
//! variable taken from such a glob (`for f in *; do tar rf a.tar "$f";
//! done`; see [`GlobLoops`]), and `xargs` fed by `ls` or by `find` without
//! `-print0` and `xargs -0` (`ls | xargs tar cf a.tar`; see [`Feed`]),
//! whose input may hold such a name, or one split at a blank. Other unseen
//! operands are not judged here.

use super::indirect;
use super::may_be_built;
use super::quoted::Command;
use super::quoted::Word;
use super::work;

/// Commands with an option whose value they run as a command line, or as
/// a program with arguments (see `rerun`'s `OPERAND_RUNNERS`): `tar`'s
/// `--checkpoint-action=exec=`, `--to-command`, `-I`; `sed`'s `e`
/// command (`--expression=1e sh x`); `rsync -e`. Any text after the
/// pattern becomes an argument (`--to-command=sh x.txt` matches `*.txt`).
/// macOS's `tar` is `bsdtar` (`--use-compress-program`).
const LINE_RUNNERS: &[&str] = &["tar", "gtar", "bsdtar", "sed", "gsed", "rsync"];

/// Commands with an option whose value is a program name, looked up on
/// `PATH` (`rg --pre`, `sort --compress-program`): a glob with literal
/// text after its pattern (`*.rs`) can only name a program ending in it,
/// which a file name cannot give a folder for (a `PATH` holding `.` or an
/// empty entry would; that is not judged here).
const PROGRAM_RUNNERS: &[&str] = &["rg", "sort", "gsort"];

/// Characters at which a word's text stops being literal for this check:
/// glob characters, braces and extended-glob groups.
const PATTERN: &[char] = &['*', '?', '[', '^', '{', '(', '@', '!', '+', '#'];

/// Characters a pattern may end with (`*`, `x?`, `[ab]`, `@(a|b)`,
/// `{a,*}`).
const PATTERN_END: &[char] = &['*', '?', ']', ')', '}'];

/// Whether a glob's matches may end in any text: it ends in its pattern,
/// or it may negate or exclude (zsh `^a.txt`, `*~x.rs`, `#`; bash
/// `!(a).rs`), which a literal ending does not bound.
fn ends_open(text: &str) -> bool {
    text.ends_with(PATTERN_END) || text.contains(['^', '~', '!', '#'])
}

/// Old-style `tar` letters that take the next word as their value
/// (`tar cf -- *`: `--` is the archive): GNU tar's (as in `rerun`) and
/// bsdtar's `s` and `W`. An extra letter only reads `--` as a value, which
/// asks.
const TAR_VALUE_LETTERS: &str = "bCfFgHIKLNTVXsW";

/// Whether the command whose command word is at `at` (lowercase basename
/// `name`) is a [`LINE_RUNNERS`] or [`PROGRAM_RUNNERS`] command given a
/// word that may name one of its options: a glob that may match a file
/// named so, a [`GlobLoops`] variable holding one, or, when `xargs` runs
/// it, input from `feed`. `words` is the quote-aware reading of the
/// command, `texts` their texts.
pub(super) fn runs_option_glob(
    name: &str,
    at: usize,
    texts: &[String],
    words: &[Word],
    loops: &GlobLoops,
    feed: Feed,
) -> bool {
    let program_only = if LINE_RUNNERS.contains(&name) {
        false
    } else if PROGRAM_RUNNERS.contains(&name) {
        true
    } else {
        return false;
    };
    // `xargs`' input: added at the end, or put where its replacement
    // string is (`-I{}`), when it may name an option.
    let fed = fed_input(texts, at, feed);
    let tar = matches!(name, "tar" | "gtar" | "bsdtar");
    // Old-style tar's first word holds letters whose values follow it.
    let old_end = match texts.get(at + 1) {
        Some(letters) if tar && !letters.starts_with('-') => {
            at + 2
                + letters
                    .chars()
                    .filter(|letter| {
                        work::step();
                        TAR_VALUE_LETTERS.contains(*letter)
                    })
                    .count()
        }
        _ => 0,
    };
    for index in at + 1..texts.len().min(words.len()) {
        work::step();
        // Options end at `--`, unless it is an option's value (`tar -cf
        // --`, `tar cf --`).
        if texts[index] == "--" && !texts[index - 1].starts_with('-') && index >= old_end {
            return false;
        }
        let word = &words[index];
        let open = if may_match_option(word) {
            Some(ends_open(&word.text))
        } else if let Some(Some(replace)) = &fed
            && word.text.starts_with(replace.as_str())
        {
            Some(true)
        } else {
            loops.option(word)
        };
        if open.is_some_and(|open| open || !program_only) {
            return true;
        }
    }
    // Input added after the last word.
    matches!(fed, Some(None))
}

/// What a pipeline feeds a command's input, by its earlier stages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Feed {
    /// Nothing judged here.
    #[default]
    Other,
    /// `find … -print0` (without `-printf`): its names start with a
    /// start point and are split only at NULs, read so by `xargs -0`.
    FindNul,
    /// `ls`, or `find` otherwise: bare names (`ls`), or names `xargs`
    /// splits at blanks (`./a --checkpoint=1`).
    Listing,
}

/// Commands that list file names.
const LISTERS: &[&str] = &["ls", "gls"];
const FINDERS: &[&str] = &["find", "gfind"];

/// What each of `commands` gets from the pipeline stages before it: the
/// most open [`Feed`] of any of them.
pub(super) fn feeds(commands: &[Command]) -> Vec<Feed> {
    let mut feeds = vec![Feed::Other; commands.len()];
    for (index, command) in commands.iter().enumerate() {
        work::step();
        let Some(to) = command.pipe_to.filter(|to| *to < commands.len()) else {
            continue;
        };
        let texts: Vec<String> = command.words.iter().map(|word| word.text.clone()).collect();
        let own = match indirect::command_word(&texts) {
            Some((_, name)) if LISTERS.contains(&name.as_str()) => Feed::Listing,
            Some((at, name)) if FINDERS.contains(&name.as_str()) => {
                let nul = texts[at + 1..].iter().any(|text| text == "-print0")
                    && !texts[at + 1..].iter().any(|text| {
                        work::step();
                        text.contains("printf")
                    });
                if nul { Feed::FindNul } else { Feed::Listing }
            }
            _ => Feed::Other,
        };
        feeds[to] = feeds[to].max(feeds[index]).max(own);
    }
    feeds
}

/// When `xargs` runs the command at `at` and `feed` may name an option
/// through it: `Some(Some(replace))` puts the input where `replace` is
/// (`-I{}`, `-i`, `--replace`, BSD `-J`), `Some(None)` adds it at the end.
fn fed_input(texts: &[String], at: usize, feed: Feed) -> Option<Option<String>> {
    if feed == Feed::Other {
        return None;
    }
    let start = indirect::programs(texts)
        .into_iter()
        .rev()
        .find(|&index| index < at && basename_lower(&texts[index]) == "xargs")?;
    let (mut replace, mut nul) = (None, false);
    let mut index = start + 1;
    while index < at {
        work::step();
        let word = texts[index].as_str();
        index += 1;
        if word == "--null" {
            nul = true;
        } else if word == "--replace" {
            replace = Some("{}".to_string());
        } else if let Some(value) = word.strip_prefix("--replace=") {
            replace = Some(value.to_string());
        } else if let Some(letters) = word.strip_prefix('-').filter(|rest| !rest.starts_with('-')) {
            for (offset, letter) in letters.char_indices() {
                work::step();
                let rest = &letters[offset + letter.len_utf8()..];
                match letter {
                    '0' => nul = true,
                    'i' => {
                        replace = Some(if rest.is_empty() { "{}" } else { rest }.to_string());
                        break;
                    }
                    'I' | 'J' => {
                        let value = if rest.is_empty() {
                            index += 1;
                            texts.get(index - 1).map_or("", String::as_str)
                        } else {
                            rest
                        };
                        replace = Some(value.to_string());
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    if feed == Feed::FindNul && nul {
        return None;
    }
    // An empty replacement string would match every word: read it as
    // added at the end, which asks as well.
    Some(replace.filter(|replace| !replace.is_empty()))
}

fn basename_lower(word: &str) -> String {
    word.rsplit('/').next().unwrap_or(word).to_lowercase()
}

/// A loop variable taken from a glob, and what that glob may give it.
struct GlobLoop {
    name: String,
    /// A value starting with `-` (`*`, `*.txt`; not `./*`)...
    dash: bool,
    /// ...and ending in any text (`*`; not `*.txt`).
    open: bool,
}

/// D484 (b): the variables of `for`/`select`/`foreach` loops over a glob
/// met so far in a script, read on to its end. Past `MAX_GLOB_LOOPS`, any
/// variable may be one.
#[derive(Default)]
pub(super) struct GlobLoops {
    loops: Vec<GlobLoop>,
    any: bool,
}

const MAX_GLOB_LOOPS: usize = 8;

impl GlobLoops {
    /// Takes in a simple command (`for f in *`). zsh's `for f (*)` and
    /// `foreach f (*)`, which this reading cuts at `(`, count as `*`.
    pub(super) fn read(&mut self, command: &Command) {
        let words = &command.words;
        let keyword = words.first().is_some_and(|word| {
            matches!(
                word.text.to_lowercase().as_str(),
                "for" | "select" | "foreach"
            )
        });
        let Some(name) = words.get(1).filter(|_| keyword).map(|word| &word.text) else {
            return;
        };
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return;
        }
        let (mut glob, mut dash, mut open) = (false, false, false);
        if words.get(2).is_some_and(|word| word.text == "in") {
            for word in words[3..].iter().take_while(|word| word.text != "do") {
                work::step();
                glob |= word.globs;
                if may_match_option(word) {
                    dash = true;
                    open |= ends_open(&word.text);
                }
            }
        } else if words.len() == 2 && command.cut {
            (glob, dash, open) = (true, true, true);
        }
        if !glob {
            return;
        }
        if let Some(known) = self.loops.iter_mut().find(|known| known.name == *name) {
            known.dash |= dash;
            known.open |= open;
        } else if self.loops.len() == MAX_GLOB_LOOPS {
            self.any = true;
        } else {
            self.loops.push(GlobLoop {
                name: name.clone(),
                dash,
                open,
            });
        }
    }

    /// Whether `word` may name an option through a loop variable: it starts
    /// with one whose glob may start with `-` (`"$f"`, `${f%.txt}`), it
    /// changes one's start (`${f#./}`, `${f##*/}`), or the shell may split
    /// it (`$f` unquoted, `./a --x` from `./*`). `Some(open)`: whether
    /// its value may end in any text.
    pub(super) fn option(&self, word: &Word) -> Option<bool> {
        if !word.dynamic || (self.loops.is_empty() && !self.any) {
            return None;
        }
        let text = word.text.as_str();
        let mut found: Option<bool> = None;
        for (at, _) in text.match_indices('$') {
            work::step();
            let after = &text[at + 1..];
            let braced = after.starts_with('{');
            let after = after.strip_prefix('{').unwrap_or(after);
            let end = after
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .unwrap_or(after.len());
            let (name, rest) = after.split_at(end);
            if name.is_empty() {
                continue;
            }
            let known = self.loops.iter().find(|known| known.name == name);
            let (dash, open) = match known {
                Some(known) => (known.dash, known.open),
                None if self.any => (true, true),
                None => continue,
            };
            // `${f}`, `${f%x}`, `${f:-x}`: the value's start is kept.
            let kept = !braced
                || rest.starts_with(['}', '%', '-', '=', '?', '+'])
                || (rest.starts_with(':') && rest[1..].starts_with(['-', '=', '?', '+']));
            let option = if word.splits {
                // Split into words, one of which may start anywhere in it.
                Some(true)
            } else if at > 0 {
                None
            } else if kept {
                dash.then_some(open)
            } else {
                Some(true)
            };
            found = found.max(option);
        }
        found
    }
}

/// Whether the shell may glob `word` into one that starts with `-` and
/// names an option: its literal text before the first pattern character
/// (less a character zsh's `#` makes optional) is empty, an option name
/// not ending its value at `=` (`-*`, `--pre*`, `-ne=*`: a short option's
/// value may be the `=`), or a value the line does not show (`$d*`;
/// `"$d"/*` holds such a value as a folder, like `"$d"` alone, which is not
/// judged here).
fn may_match_option(word: &Word) -> bool {
    if !word.globs {
        return false;
    }
    let text = word.text.as_str();
    let end = text.find(PATTERN).unwrap_or(text.len());
    let mut prefix = &text[..end];
    // zsh's `x#`, `x##` and `x(#c0,1)` make the `x` before them optional.
    if text[end..].starts_with('#') || text[end..].starts_with("(#") {
        prefix = prefix
            .char_indices()
            .last()
            .map_or(prefix, |(last, _)| &prefix[..last]);
    }
    prefix.is_empty()
        || (prefix.starts_with('-') && !(prefix.starts_with("--") && prefix.contains('=')))
        || (may_be_built(prefix) && !prefix.contains('/'))
}
