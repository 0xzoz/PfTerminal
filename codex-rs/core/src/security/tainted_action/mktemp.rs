//! D191 (owner-approved relaxation): the output of a plain `mktemp` is a
//! known path in the temp folder, so `tmp=$(mktemp); … > "$tmp"` and `trap
//! "rm -f $tmp" EXIT` after untrusted content no longer ask.
//!
//! Bounded on purpose. Only the action's own `sh`/`bash`/`zsh -c` script
//! qualifies, and only variables it sets first, before anything else runs
//! (`tmp=$(mktemp); …`), from `mktemp` with no template or one directly in
//! `/tmp` or `$TMPDIR` (`-d`, `-q`, `-t` allowed; not `-p`, `-u`). Such a
//! variable holds [`TEMP_FILE`] or [`TEMP_DIR`], which every check still
//! reads as unseen output. Only two prompts stop: for the file itself
//! written (`> "$tmp"`, `cp f "$tmp"`; not `"$tmp.x"` or a folder's member,
//! which a failed `mktemp` or an extracted link may put elsewhere), and for
//! a trap that only removes the path. They are held back until the whole
//! line has been read, and the line keeps them when it may give the
//! variable another value: it names the variable other than as `$tmp`
//! (`read tmp`, `tmp=x`, `${tmp:=x}`, `export tmp`), uses a builtin that
//! sets or evaluates variables (`read`, `printf -v`, `declare`, `eval`,
//! `source`), names a command at run time (`$c`), or assigns a variable it
//! names at run time (`(( $n = 1 ))`); or when it gives the file's path to
//! a command that may put a link in its place or pass the path on (`mv
//! ./link "$tmp"`, `ln`, `echo "$tmp" | xargs …`): only a few commands that
//! do neither may take it as an operand, by name, on a line that defines
//! no function and does not change `PATH`. A program that finds the file
//! on its own may still replace it, as it may any literal temp path.

use super::ProtectedActionKind;
use super::indirect;
use super::shell::SimpleCommand;
use super::shell::UNSEEN_OUTPUT;
use super::work;
use std::collections::HashMap;

/// What a qualifying variable set from `mktemp` holds: unseen output (every
/// other check treats it as such) marked as a fresh temp file. Text never
/// holds `\u{1}`, and the placeholders only hold it around `unseen-output`,
/// so nothing but [`Scope::value`] makes this.
pub(super) const TEMP_FILE: &str = "\u{1}unseen-output\u{1}\u{1}mktemp\u{1}";
/// The same for `mktemp -d`.
pub(super) const TEMP_DIR: &str = "\u{1}unseen-output\u{1}\u{1}mktemp-d\u{1}";

/// Builtins that set variables they are given or evaluate text, and words
/// that change how the shell reads a line: a line using any of them keeps
/// its prompts.
const SETS_VARIABLES: &[&str] = &[
    "read",
    "mapfile",
    "readarray",
    "getopts",
    "declare",
    "typeset",
    "local",
    "export",
    "readonly",
    "integer",
    "float",
    "private",
    "unset",
    "let",
    "eval",
    "source",
    "alias",
    "unalias",
    "vared",
    "sysread",
    "zparseopts",
    "wait",
    "coproc",
    "select",
    "enable",
    "disable",
    "setopt",
    "unsetopt",
    "emulate",
    "shopt",
    "zmodload",
    "autoload",
    "bind",
    "complete",
    "compgen",
    "compctl",
    "zle",
    "fc",
    "getln",
    "zstyle",
    "zformat",
    "zregexparse",
    "pcre_match",
    "zstat",
    "strftime",
    "zselect",
    "ztcp",
    "zsocket",
    "zpty",
    "zsystem",
    "zcurses",
    "hash",
];

/// Words before a command that leave it in this shell, and keywords.
const PREFIXES: &[&str] = &[
    "!",
    "if",
    "then",
    "elif",
    "else",
    "do",
    "while",
    "until",
    "time",
    "noglob",
    "nocorrect",
    "-",
    "builtin",
    "command",
    "exec",
    "{",
    "(",
];

/// Variables a qualifying name may not be: set by the classifier itself or
/// read by the shell.
const RESERVED: &[&str] = &[
    "PWD",
    "OLDPWD",
    "HOME",
    "CODEX_HOME",
    "CORBANU_HOME",
    "PFTERMINAL_HOME",
    "TMPDIR",
    "IFS",
    "PATH",
];

/// The variables of the line being read that hold a temp path, and the
/// prompts held back for them.
#[derive(Debug)]
pub(super) struct Scope {
    /// The variables, and whether each holds a folder.
    names: HashMap<String, bool>,
    held: Option<ProtectedActionKind>,
    /// The line may give one of `names` another value, or put a link in a
    /// temp file's place.
    spoiled: bool,
}

impl Scope {
    /// The scope of the action's own command line (`command`, lexed as
    /// `lexed`), if its script sets a qualifying variable first and no word
    /// of the line may set it again.
    pub(super) fn of(command: &[String], lexed: &[SimpleCommand]) -> Option<Self> {
        let (shell, script) = codex_shell_command::bash::extract_bash_command(command)?;
        let system_shell = !shell.contains('/')
            || [
                "/bin/",
                "/usr/bin/",
                "/usr/local/bin/",
                "/opt/homebrew/bin/",
            ]
            .iter()
            .any(|dir| {
                shell
                    .strip_prefix(dir)
                    .is_some_and(|rest| !rest.contains('/'))
            });
        // Text the word lexer may read differently from the shell.
        let plain = script
            .bytes()
            .all(|byte| byte == b'\n' || byte == b'\t' || (0x20..0x7f).contains(&byte))
            && !script.contains("\\\n")
            && !script.contains("$'")
            && !script.contains("$\"")
            // bash 5.3 and ksh function substitutions run in this shell.
            && !script.contains("${ ")
            && !script.contains("${\t")
            && !script.contains("${\n")
            && !script.contains("${|")
            // Word splitting, where commands are found, and functions
            // (`cp() { …; }`) change what a later word does.
            && !script.contains("IFS")
            && !script.contains("PATH")
            && !script.contains("function")
            && !defines_function(script);
        if !system_shell || !plain {
            return None;
        }
        let names: HashMap<String, bool> = leading_definitions(script)
            .into_iter()
            .filter(|(name, _)| !RESERVED.contains(name))
            .map(|(name, folder)| (name.to_string(), folder))
            .collect();
        if names.is_empty() {
            return None;
        }
        let scope = Self {
            names,
            held: None,
            spoiled: false,
        };
        let mut definitions = 0;
        for simple in lexed {
            work::step();
            if simple.words.len() == 1 && scope.definition(&simple.words[0]) {
                definitions += 1;
                continue;
            }
            if !scope.allows(&simple.words, /*substituted*/ false) {
                return None;
            }
        }
        (definitions == scope.names.len()).then_some(scope)
    }

    /// What `name=value` stores: [`TEMP_FILE`] or [`TEMP_DIR`] for a
    /// qualifying variable's first value (its `mktemp` output), else
    /// nothing.
    pub(super) fn value(&self, name: &str, value: &str, first: bool) -> Option<&'static str> {
        let folder = self
            .names
            .get(name)
            .filter(|_| first && value == UNSEEN_OUTPUT)?;
        Some(if *folder { TEMP_DIR } else { TEMP_FILE })
    }

    /// Read a command's words as substituted (`$a$b` may spell a name):
    /// the line keeps its prompts when they may set a variable, or when the
    /// command may put a link in a temp file's place or pass its path on.
    pub(super) fn read(&mut self, simple: &SimpleCommand, words: &[String]) {
        if words.len() == 1 && self.definition(&words[0]) {
            return;
        }
        if !self.allows(words, /*substituted*/ true) || !file_operands_ok(simple, words) {
            self.spoiled = true;
        }
    }

    /// Whether a trap's operands (as substituted) only remove temp paths.
    pub(super) fn removes_temp(&self, args: &[String]) -> bool {
        self.removes(args, /*substituted*/ true)
    }

    /// Whether a command (as substituted) is such a trap.
    pub(super) fn removal_trap(&self, words: &[String]) -> bool {
        words.first().is_some_and(|word| word == "trap") && self.removes_temp(&words[1..])
    }

    /// Hold back a prompt that only a temp path raised.
    pub(super) fn hold(&mut self, kind: ProtectedActionKind) {
        if self.held.is_none_or(|held| held.strongest(kind) != held) {
            self.held = Some(kind);
        }
    }

    /// The prompts the line keeps: those held back, unless the variables
    /// kept their temp paths. `assigns_unseen_name`: arithmetic may set a
    /// variable named at run time.
    pub(super) fn finish(self, assigns_unseen_name: bool) -> Option<ProtectedActionKind> {
        if self.spoiled || assigns_unseen_name {
            self.held
        } else {
            None
        }
    }

    /// `name=` and the unseen output of its `mktemp`.
    fn definition(&self, word: &str) -> bool {
        word.split_once('=')
            .is_some_and(|(name, value)| self.names.contains_key(name) && value == UNSEEN_OUTPUT)
    }

    /// Whether a command's words leave every qualifying variable alone.
    fn allows(&self, words: &[String], substituted: bool) -> bool {
        let mut at_command = true;
        let mut after_wrapper = false;
        for (index, word) in words.iter().enumerate() {
            work::step();
            // zsh `set -A name`.
            if SETS_VARIABLES.contains(&word.as_str())
                || word
                    .strip_prefix(['-', '+'])
                    .is_some_and(|flags| flags.starts_with('A'))
                || !self.names_ok(word)
            {
                return false;
            }
            if word == "trap" && !self.removes(&words[index + 1..], substituted) {
                return false;
            }
            // `printf -v`, `print -v`.
            if matches!(word.as_str(), "printf" | "print")
                && words[index + 1..].iter().any(|word| {
                    word.strip_prefix('-')
                        .is_some_and(|flags| flags.contains('v'))
                })
            {
                return false;
            }
            if at_command {
                let assignment = word
                    .split_once('=')
                    .is_some_and(|(name, _)| !name.is_empty() && name.bytes().all(is_name_byte));
                if assignment
                    || PREFIXES.contains(&word.as_str())
                    || (after_wrapper && word.starts_with('-'))
                {
                    after_wrapper |=
                        matches!(word.as_str(), "builtin" | "command" | "exec" | "time");
                    continue;
                }
                // `. file` and a command named at run time (`$c x`).
                if word == "."
                    || (!substituted && (word.contains(['$', '`']) || word.contains(UNSEEN_OUTPUT)))
                {
                    return false;
                }
                at_command = false;
            }
        }
        true
    }

    /// Whether every name in `word` that a qualifying variable has is only
    /// read (`$tmp`, `${tmp}`) or part of a path or option (`/tmp`, `.tmp`,
    /// `-tmp`), not set (`tmp=x`, `read tmp`, `${tmp:=x}`, `${!tmp}`).
    fn names_ok(&self, word: &str) -> bool {
        let text = word
            .replace(TEMP_FILE, " ")
            .replace(TEMP_DIR, " ")
            .replace(UNSEEN_OUTPUT, " ");
        let bytes = text.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            work::step();
            if !is_name_byte(bytes[start]) {
                start += 1;
                continue;
            }
            let end = bytes[start..]
                .iter()
                .position(|byte| !is_name_byte(*byte))
                .map_or(bytes.len(), |len| start + len);
            if self.names.contains_key(&text[start..end]) {
                let before = start.checked_sub(1).map(|at| bytes[at]);
                let after = bytes.get(end).copied();
                let read =
                    before == Some(b'$') && !matches!(after, Some(b'=' | b':' | b'[' | b'('));
                if !read && !matches!(before, Some(b'/' | b'.' | b'-')) {
                    return false;
                }
            }
            start = end;
        }
        true
    }

    /// Whether a trap's words (after `trap`) only remove qualifying
    /// variables' paths: `[--] rm [-f…] [--] "$tmp"… SIGNAL…`.
    fn removes(&self, words: &[String], substituted: bool) -> bool {
        let mut words = words.iter().map(String::as_str).peekable();
        words.next_if_eq(&"--");
        if words.next() != Some("rm") {
            return false;
        }
        while words.next_if(|word| rm_flags(word)).is_some() {
            work::step();
        }
        words.next_if_eq(&"--");
        let mut removed = 0;
        while words
            .next_if(|word| {
                if substituted {
                    *word == TEMP_FILE || *word == TEMP_DIR
                } else {
                    word.strip_prefix('$')
                        .is_some_and(|name| self.names.contains_key(name))
                }
            })
            .is_some()
        {
            work::step();
            removed += 1;
        }
        let mut signals = 0;
        for word in words {
            work::step();
            if word.is_empty()
                || !word
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            {
                return false;
            }
            signals += 1;
        }
        removed > 0 && signals > 0
    }
}

/// Whether a written word (or a piece of one) is a qualifying variable's
/// temp file itself.
pub(super) fn temp_place(word: &str) -> bool {
    word == TEMP_FILE
}

/// Whether a command takes a temp file's path, other than as a file it
/// redirects, only whole and only as an operand of a command that neither
/// puts a link in its place nor prints or passes the path on: `rm`, `cat`,
/// `cp` without options, `mv` as the file moved, `tee`, `chmod`, `touch`,
/// `sort`, `uniq`, `sed`, a test and a removing trap (judged on its own).
fn file_operands_ok(simple: &SimpleCommand, words: &[String]) -> bool {
    let redirected = |at: usize| {
        simple.writes_to.contains(&at)
            || simple.stdin_from == Some(at)
            || simple.here_string == Some(at)
    };
    let mut used = false;
    for (at, word) in words.iter().enumerate() {
        work::step();
        if word.contains(TEMP_FILE) && !redirected(at) {
            if word != TEMP_FILE {
                return false;
            }
            used = true;
        }
    }
    if !used {
        return true;
    }
    // The command itself as written, not behind a wrapper that may add
    // words (`xargs cp …`) or a program named by a path (`./cp`).
    let Some((index, name)) = indirect::command_word(words) else {
        return false;
    };
    if index != 0 || words[0] != name {
        return false;
    }
    let operands: Vec<&str> = words[index + 1..]
        .iter()
        .enumerate()
        .filter(|(at, _)| !redirected(index + 1 + at))
        .map(|(_, word)| word.as_str())
        .collect();
    match name.as_str() {
        "rm" | "cat" | "tee" | "chmod" | "touch" | "sort" | "uniq" | "sed" | "test" | "["
        | "[[" | "trap" => true,
        "cp" => !operands.iter().any(|word| word.starts_with('-')),
        // The file moved, not where it goes (`mv ./link "$tmp"`).
        "mv" => {
            let files: Vec<&str> = operands
                .iter()
                .copied()
                .filter(|word| *word != "-f")
                .collect();
            matches!(files.as_slice(), [from, to]
                if *from == TEMP_FILE && *to != TEMP_FILE && !to.starts_with('-'))
        }
        _ => false,
    }
}

/// Whether text may define a function: `(` and `)` with only blanks
/// between them (`f() {`, `f ( ) {`).
fn defines_function(text: &str) -> bool {
    text.match_indices('(').any(|(at, _)| {
        work::step();
        text[at + 1..]
            .trim_start_matches([' ', '\t'])
            .starts_with(')')
    })
}

fn rm_flags(word: &str) -> bool {
    word.strip_prefix('-')
        .is_some_and(|flags| !flags.is_empty() && flags.bytes().all(|byte| b"rRfv".contains(&byte)))
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The variables a script sets first, each from a plain `mktemp`, one
/// after another (`a=$(mktemp); b="$(mktemp -d)" && …`), and whether each
/// holds a folder.
fn leading_definitions(script: &str) -> Vec<(&str, bool)> {
    let mut names = Vec::new();
    let mut rest = script.trim_start_matches([' ', '\t', '\n']);
    while let Some((name, folder, after)) = definition(rest) {
        work::step();
        let after = after.trim_start_matches([' ', '\t']);
        if after.is_empty() || after.starts_with("&&") || after.starts_with("||") {
            names.push((name, folder));
            break;
        }
        match after.strip_prefix([';', '\n']) {
            // `;;`, `;&` and `;|` end a case item.
            Some(next) if !next.starts_with([';', '&', '|']) => {
                names.push((name, folder));
                rest = next.trim_start_matches([' ', '\t', '\n']);
            }
            _ => break,
        }
    }
    names
}

/// `NAME=$(mktemp …)` or `NAME="$(mktemp …)"` at the start of `text`,
/// whether it makes a folder, and the text after it.
fn definition(text: &str) -> Option<(&str, bool, &str)> {
    let name_len = text.bytes().position(|byte| !is_name_byte(byte))?;
    let name = &text[..name_len];
    if name.is_empty() || name.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let value = text[name_len..].strip_prefix('=')?;
    let (quoted, value) = match value.strip_prefix('"') {
        Some(value) => (true, value),
        None => (false, value),
    };
    let body = value.strip_prefix("$(")?;
    let close = body.find(')')?;
    // `${TMPDIR:-/tmp}` holds no `)`; any other `(` ends it.
    if body[..close].contains('(') {
        return None;
    }
    let folder = plain_mktemp(&body[..close])?;
    let after = &body[close + 1..];
    let after = if quoted {
        after.strip_prefix('"')?
    } else {
        after
    };
    // The value ends at the word's end.
    if after
        .bytes()
        .next()
        .is_some_and(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b';' | b'&' | b'|'))
    {
        return None;
    }
    Some((name, folder, after))
}

/// `mktemp` with options `-d`, `-q`, `-t` (before any operand) and at most
/// one operand: with `-t` a plain name (BSD's prefix, GNU's template, both
/// in `$TMPDIR`), else a template directly in `/tmp` or `$TMPDIR` ending in
/// the `X`s `mktemp` replaces. Whether it makes a folder.
fn plain_mktemp(text: &str) -> Option<bool> {
    let mut words = text.split([' ', '\t']).filter(|word| !word.is_empty());
    if words.next() != Some("mktemp") {
        return None;
    }
    let mut folder = false;
    let mut in_tmpdir = false;
    let mut operand: Option<&str> = None;
    for word in words {
        work::step();
        if operand.is_some() {
            return None;
        }
        if let Some(flags) = word.strip_prefix('-') {
            let short = !flags.is_empty()
                && !flags.starts_with('-')
                && flags.bytes().all(|byte| b"dqt".contains(&byte));
            if !(short || matches!(flags, "-directory" | "-quiet")) {
                return None;
            }
            in_tmpdir |= short && flags.contains('t');
            folder |= (short && flags.contains('d')) || flags == "-directory";
            continue;
        }
        operand = Some(unquoted(word)?);
    }
    let Some(operand) = operand else {
        return Some(folder);
    };
    let in_temp = if in_tmpdir {
        plain_name(operand)
    } else {
        ["/tmp/", "$TMPDIR/", "${TMPDIR}/", "${TMPDIR:-/tmp}/"]
            .iter()
            .any(|root| {
                operand
                    .strip_prefix(root)
                    .is_some_and(|name| plain_name(name) && name.ends_with("XXX"))
            })
    };
    in_temp.then_some(folder)
}

/// A word without its surrounding quotes, if it holds no other quote and
/// no `$` but a leading `$TMPDIR` (single-quoted: none).
fn unquoted(word: &str) -> Option<&str> {
    let (inner, expands) = if let Some(inner) = word
        .strip_prefix('\'')
        .and_then(|word| word.strip_suffix('\''))
    {
        (inner, false)
    } else if let Some(inner) = word
        .strip_prefix('"')
        .and_then(|word| word.strip_suffix('"'))
    {
        (inner, true)
    } else {
        (word, true)
    };
    if inner.contains(['\'', '"']) {
        return None;
    }
    let tmpdir = ["$TMPDIR/", "${TMPDIR}/", "${TMPDIR:-/tmp}/"]
        .iter()
        .find_map(|root| inner.strip_prefix(root));
    match tmpdir {
        Some(rest) if expands && !rest.contains(['$', '{', '}']) => Some(inner),
        _ if !inner.contains(['$', '{', '}']) => Some(inner),
        _ => None,
    }
}

/// A file name of letters, digits, `.`, `_` and `-`, not starting with `.`.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}
