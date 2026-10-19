//! #937: shell arithmetic evaluates a variable's value as an expression, and
//! an array subscript in that value runs its command substitution
//! (`v=$(cat f); echo $((v))` runs `a[$(cmd)0]` held in `f`). Like `eval`,
//! arithmetic that uses a variable filled from command output or `read` in
//! the same script runs code the classifier cannot see (owner decision D33,
//! option B). Variables set from literals (loop counters) are not affected.
//!
//! This over-approximates on purpose: quotes are not honoured, names are
//! tracked for the whole line regardless of order, and any value that
//! mentions a filled name passes it on.

use super::indirect;
use super::shell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Range;

/// `[[` operators that evaluate both operands as arithmetic (bash, zsh and
/// ksh; ksh's `test`/`[` too), and `-v`, whose operand is a name that may
/// carry a subscript (bash and zsh `test` too).
const TEST_OPERATORS: &[&str] = &["-eq", "-ne", "-lt", "-le", "-gt", "-ge", "-v"];
/// Builtins whose operands are numbers evaluated as arithmetic (zsh and ksh
/// evaluate them; bash does not, which only over-matches).
const NUMERIC_BUILTINS: &[&str] = &[
    "let", "exit", "return", "shift", "break", "continue", "repeat", "ulimit", "limit", "unlimit",
];
/// Builtins whose operands name variables, and whose names may hold a
/// subscript (`declare "$x"=1` with `x='a[$(cmd)0]'`).
const NAMING_BUILTINS: &[&str] = &[
    "declare",
    "typeset",
    "local",
    "export",
    "readonly",
    "integer",
    "float",
    "private",
    "unset",
    "read",
    "vared",
    "mapfile",
    "readarray",
    "getopts",
];
/// Builtins that fill their named variables from their input.
const FILLING_BUILTINS: &[&str] = &["read", "vared", "mapfile", "readarray", "getopts"];
/// Builtins that declare variables (and may give them the integer attribute).
const DECLARING_BUILTINS: &[&str] = &[
    "declare", "typeset", "local", "export", "readonly", "integer", "float", "private",
];
/// `read` options whose value is a number (evaluated as arithmetic in ksh).
const READ_NUMERIC_OPTIONS: &[char] = &['t', 'n', 'N', 'k', 'u'];
/// Positional parameters, which `set --` or a function call can fill.
const POSITIONAL: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8", "9", "@", "*"];

/// What one command line does with variables, gathered command by command.
#[derive(Default)]
pub(super) struct Arithmetic {
    /// Functions the line defines (their arguments become `$1`...).
    functions: HashSet<String>,
    /// The line mentions ksh, whose `test`/`[` evaluates `-eq` operands.
    ksh: bool,
    /// Aliases the line defines, by the command they stand for.
    aliases: HashMap<String, String>,
    /// Variables filled from input (`read x`, `mapfile x`).
    filled: HashSet<String>,
    /// `NAME=value` (as lexed), `for NAME in value...`, `printf -v NAME`.
    assigned: Vec<(String, String)>,
    /// Variables given the integer attribute: assigning them evaluates.
    integers: HashSet<String>,
    /// Words evaluated as arithmetic (`let` operands, numeric operands).
    evaluated: Vec<String>,
    /// Words naming variables (`read "$x"`, `declare "$x"=1`).
    named: Vec<String>,
}

impl Arithmetic {
    /// Start a command line: `text` is its argv, scanned for function
    /// definitions.
    pub(super) fn new(text: &[String]) -> Self {
        Self {
            functions: text.iter().flat_map(|arg| functions(arg)).collect(),
            ksh: text.iter().any(|arg| arg.contains("ksh")),
            ..Self::default()
        }
    }

    /// Note one simple command: `raw` are its words as lexed and `words`
    /// the same words after variable substitution, which name the command
    /// (`c=read; $c x` reads into `x`). Values are judged as lexed, so a
    /// copy of a counter (`j=$i`) is not mistaken for command output.
    pub(super) fn note(&mut self, raw: &[String], words: &[String]) {
        let mut start = 0;
        // `eval let v`, zsh `noglob let v`: the rest is a command too.
        while start < raw.len().min(words.len()) {
            let raw = &raw[start..];
            let command = indirect::command_word(&words[start..]);
            self.note_one(raw, command.as_ref());
            match command {
                Some((index, name)) if matches!(name.as_str(), "eval" | "noglob" | "nocorrect") => {
                    start += index + 1;
                }
                _ => break,
            }
        }
    }

    fn note_one(&mut self, words: &[String], command: Option<&(usize, String)>) {
        let end = command.map_or(words.len(), |(index, _)| *index);
        // Assignments before the command (`IFS= read`, `X=1 cmd`), and all
        // words when there is no command.
        for word in &words[..end] {
            if let Some((name, value)) = assignment(word) {
                self.assigned.push((name.to_string(), value.to_string()));
            }
        }
        let Some((index, name)) = command else {
            return;
        };
        let args = &words[index + 1..];
        // `alias L='let v'`: the definition is judged as the command it
        // stands for, and later `L` commands as that command.
        if name == "alias" {
            for (at, word) in args.iter().enumerate() {
                let Some((alias, value)) = word.split_once('=') else {
                    continue;
                };
                let mut aliased = vec![value.to_string()];
                aliased.extend(
                    args[at + 1..]
                        .iter()
                        .take_while(|word| !word.contains('='))
                        .cloned(),
                );
                if let Some((first, target)) = indirect::command_word(&aliased)
                    && target != "alias"
                {
                    self.note_one(&aliased, Some(&(first, target.clone())));
                    // Command names are compared lowercased.
                    self.aliases.insert(alias.to_lowercase(), target);
                }
            }
        }
        let name = self
            .aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        let name = name.as_str();
        if NUMERIC_BUILTINS.contains(&name) {
            self.evaluated.extend(args.iter().cloned());
        }
        let operator = |word: &String| {
            word == "-v" || (TEST_OPERATORS.contains(&word.as_str()) && (name == "[[" || self.ksh))
        };
        if matches!(name, "test" | "[" | "[[") && args.iter().any(operator) {
            self.evaluated.extend(args.iter().cloned());
        }
        if matches!(name, "printf" | "print") {
            self.printf(args);
        }
        if name == "read" {
            self.read_options(args);
        }
        if NAMING_BUILTINS.contains(&name) {
            self.named.extend(
                args.iter()
                    .filter(|word| !word.starts_with(['-', '+']))
                    .map(|word| word.split_once('=').map_or(word.as_str(), |(name, _)| name))
                    .map(str::to_string),
            );
        }
        if FILLING_BUILTINS.contains(&name) {
            self.filled
                .extend(args.iter().filter(|word| is_identifier(word)).cloned());
        }
        if DECLARING_BUILTINS.contains(&name) {
            let integer = matches!(name, "integer" | "float")
                || args.iter().any(|word| {
                    word.len() > 1
                        && word.starts_with(['-', '+'])
                        && word[1..].contains(['i', 'F', 'E'])
                });
            for word in args {
                let Some((name, value)) = assignment(word) else {
                    if integer && is_identifier(word) {
                        self.integers.insert(word.clone());
                    }
                    continue;
                };
                self.assigned.push((name.to_string(), value.to_string()));
                if integer {
                    self.integers.insert(name.to_string());
                    self.evaluated.push(value.to_string());
                }
            }
        }
        if matches!(name, "for" | "select" | "foreach") {
            // `for a b in ...` (zsh) fills each name from the values.
            let variables: Vec<&String> = args
                .iter()
                .take_while(|word| word.as_str() != "in" && is_identifier(word))
                .collect();
            for value in &args[variables.len()..] {
                for variable in &variables {
                    self.assigned.push(((*variable).clone(), value.clone()));
                }
            }
        }
        if name == "set" || self.functions.contains(name) {
            for value in args {
                for positional in POSITIONAL {
                    self.assigned
                        .push(((*positional).to_string(), value.clone()));
                }
            }
        }
    }

    /// `printf`/`print -f` with a numeric conversion evaluates its operands
    /// (zsh, ksh); `-v NAME` names a variable.
    fn printf(&mut self, args: &[String]) {
        if args.iter().any(|word| numeric_format(word)) {
            self.evaluated.extend(args.iter().cloned());
        }
        for (at, pair) in args.windows(2).enumerate() {
            if pair[0] == "-v" {
                self.named.push(pair[1].clone());
                // The variable holds what it prints.
                if let Some((name, _)) = assignment(&format!("{}=", pair[1])) {
                    for value in &args[at + 2..] {
                        self.assigned.push((name.to_string(), value.clone()));
                    }
                }
            }
        }
    }

    /// `read -t N`, `-n N`, `-k N`, `-u N`: numbers (attached or not).
    fn read_options(&mut self, args: &[String]) {
        let mut take_next = false;
        for word in args {
            if take_next {
                self.evaluated.push(word.clone());
                take_next = false;
                continue;
            }
            let Some(flags) = word.strip_prefix('-') else {
                continue;
            };
            if let Some(at) = flags.find(READ_NUMERIC_OPTIONS) {
                let value = &flags[at + 1..];
                if value.is_empty() {
                    take_next = true;
                } else {
                    self.evaluated.push(value.to_string());
                }
            }
        }
    }

    /// Whether the line evaluates, as arithmetic, a variable filled from
    /// command output or input, or command output itself. `text` is the
    /// line's argv.
    pub(super) fn runs_unseen(&self, text: &[String]) -> bool {
        let tainted = self.tainted();
        let tainted_value = |value: &str| {
            command_output(value) || names(&visible(value)).any(|name| tainted.contains(name))
        };
        if self.integers.iter().any(|name| tainted.contains(name))
            || self.evaluated.iter().any(|word| tainted_value(word))
            || self.named.iter().any(|word| {
                command_output(word)
                    || references(word).any(|name| tainted.contains(name))
                    || word.find('[').is_some_and(|at| tainted_value(&word[at..]))
            })
        {
            return true;
        }
        text.iter().any(|arg| {
            regions(arg).into_iter().any(|range| {
                let region = &arg[range];
                names(region).any(|name| tainted.contains(name)) || unseen_substitution(region)
            })
        })
    }

    /// Variables whose value is unseen: set from command output, filled
    /// from input, or assigned a value that mentions such a variable.
    fn tainted(&self) -> HashSet<String> {
        let mut tainted: HashSet<String> = self.filled.clone();
        // Who receives each name's value: a graph walk keeps this linear.
        let mut receivers: HashMap<String, Vec<&str>> = HashMap::new();
        for (name, value) in &self.assigned {
            if command_output(value) {
                tainted.insert(name.clone());
            }
            for source in names(&visible(value)) {
                receivers.entry(source.to_string()).or_default().push(name);
            }
        }
        let mut queue: Vec<String> = tainted.iter().cloned().collect();
        while let Some(source) = queue.pop() {
            for receiver in receivers.get(&source).into_iter().flatten() {
                if tainted.insert((*receiver).to_string()) {
                    queue.push((*receiver).to_string());
                }
            }
        }
        tainted
    }
}

/// Whether lexed text holds the output of a command substitution the
/// classifier cannot see (an arithmetic expansion's number does not count).
fn command_output(text: &str) -> bool {
    text.replace(shell::ARITHMETIC_OUTPUT, "")
        .contains(shell::UNSEEN_OUTPUT)
}

/// Lexed text without its placeholders, so their letters are not names.
fn visible(text: &str) -> String {
    text.replace(shell::ARITHMETIC_OUTPUT, " ")
        .replace(shell::UNSEEN_OUTPUT, " ")
}

/// Whether arithmetic text runs a command substitution whose output the
/// classifier cannot see (`$(( $(cat f) + 1 ))`, `a[`cat f`]`). Nested
/// arithmetic is read as grouping, not as a substitution.
fn unseen_substitution(region: &str) -> bool {
    if !region.contains("$(") && !region.contains('`') {
        return false;
    }
    let text = region.replace("$((", "((").replace("$[", "[");
    shell::simple_commands(&[text])
        .commands
        .iter()
        .any(|command| command.words.iter().any(|word| command_output(word)))
}

/// `NAME=value`, `NAME+=value` or `NAME[sub]=value`: the name and value.
fn assignment(word: &str) -> Option<(&str, &str)> {
    let (left, value) = word.split_once('=')?;
    let left = left.strip_suffix('+').unwrap_or(left);
    let name = left.split_once('[').map_or(left, |(name, _)| name);
    is_identifier(name).then_some((name, value))
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// The variable names text mentions as arithmetic would read it: bare
/// identifiers and `$NAME`/`${NAME}` references, including positional `$1`
/// and `$@`. Not a number's digits (`0x1f`, `16#ff`) or a length (`${#x}`).
fn names(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < bytes.len() {
            let start = index;
            let ch = bytes[index] as char;
            let previous = start.checked_sub(1).map(|at| bytes[at] as char);
            let after_dollar = previous == Some('$')
                || (previous == Some('{') && start >= 2 && bytes[start - 2] == b'$');
            if after_dollar && (ch.is_ascii_digit() || ch == '@' || ch == '*') {
                index += 1;
                return Some(&text[start..index]);
            }
            if ch.is_ascii_alphabetic() || ch == '_' {
                while index < bytes.len() && is_name_char(bytes[index] as char) {
                    index += 1;
                }
                let number_part = previous.is_some_and(|ch| ch.is_ascii_digit() || ch == '#');
                if !number_part {
                    return Some(&text[start..index]);
                }
                continue;
            }
            index += 1;
        }
        None
    })
}

/// The `$NAME`/`${NAME` references in a word.
fn references(word: &str) -> impl Iterator<Item = &str> {
    word.match_indices('$').filter_map(|(at, _)| {
        let rest = &word[at + 1..];
        let rest = rest.strip_prefix('{').unwrap_or(rest);
        let len = rest
            .find(|ch: char| !is_name_char(ch))
            .unwrap_or(rest.len());
        (len > 0).then(|| &rest[..len])
    })
}

/// Whether a `printf` format holds a numeric conversion (`%d`, `%5.2f`) or
/// a width or precision taken from an operand (`%*s`).
fn numeric_format(word: &str) -> bool {
    word.split('%').skip(1).any(|conversion| {
        let spec =
            conversion.trim_start_matches(|ch: char| "-+ #0123456789.".contains(ch) || ch == '*');
        let flags = &conversion[..conversion.len() - spec.len()];
        flags.contains('*')
            || spec
                .chars()
                .next()
                .is_some_and(|ch| "diouxXeEfFgGaA".contains(ch))
    })
}

/// Functions defined in the text: `name()`, `name ()` or `function name`.
fn functions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = text.as_bytes();
    for (at, _) in text.match_indices('(') {
        let after = text[at + 1..].trim_start_matches([' ', '\t']);
        if !after.starts_with(')') {
            continue;
        }
        let before = text[..at].trim_end_matches([' ', '\t']);
        let start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !(is_name_char(*ch) || matches!(ch, '-' | '.' | ':')))
            .map_or(0, |(at, ch)| at + ch.len_utf8());
        if start < before.len() && bytes[start] != b'$' {
            found.push(before[start..].to_string());
        }
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    for pair in words.windows(2) {
        if pair[0] == "function" {
            let name = pair[1].trim_end_matches(['(', ')', '{']);
            if !name.is_empty() {
                found.push(name.to_string());
            }
        }
    }
    found
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// `$((`, `((`: closes at `))`.
    Arithmetic,
    /// `$[`, a subscript `x[`, or any `[` inside arithmetic.
    Bracket,
    /// `(` inside arithmetic (or `$(` there).
    Paren,
    /// `${...}`; arithmetic when it holds an offset or indirection.
    Brace { arithmetic: bool },
}

impl Frame {
    fn arithmetic(self) -> bool {
        !matches!(self, Frame::Brace { arithmetic: false })
    }
}

/// Byte ranges of the text that the shell evaluates as arithmetic:
/// `$(( ))`, `(( ))`, `$[ ]`, subscripts, `${x:offset:length}`, `${!x}`,
/// zsh `${(flags)x}`, and `[[ ]]` with an arithmetic operator. Quotes are
/// not honoured; an unclosed region runs to the end of the text. Linear in
/// the text.
fn regions(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    // Where the outermost open arithmetic frame starts.
    let mut open: Option<usize> = None;
    let mut index = 0;
    let at = |offset: usize| bytes.get(offset).copied().map(char::from);
    // The colon of the innermost `${x:offset}`, where arithmetic starts.
    let mut offset: Option<usize> = None;
    while index < bytes.len() {
        if offset == Some(index) && stack.last() == Some(&Frame::Brace { arithmetic: false }) {
            stack.pop();
            if open.is_none() {
                open = Some(index);
            }
            stack.push(Frame::Brace { arithmetic: true });
        }
        let ch = bytes[index] as char;
        let next = at(index + 1);
        let inside = stack.iter().any(|frame| frame.arithmetic());
        let push = |frame: Frame, stack: &mut Vec<Frame>, open: &mut Option<usize>| {
            if frame.arithmetic() && open.is_none() {
                *open = Some(index);
            }
            stack.push(frame);
        };
        let mut step = 1;
        match ch {
            '$' if next == Some('(') && at(index + 2) == Some('(') => {
                push(Frame::Arithmetic, &mut stack, &mut open);
                step = 3;
            }
            '(' if next == Some('(') => {
                push(Frame::Arithmetic, &mut stack, &mut open);
                step = 2;
            }
            '$' if next == Some('[') => {
                push(Frame::Bracket, &mut stack, &mut open);
                step = 2;
            }
            '$' if next == Some('{') => {
                // `${!x}` and zsh `${(flags)x}` evaluate names and numbers;
                // `${x:offset:length}` from its colon.
                let arithmetic = matches!(at(index + 2), Some('!' | '('));
                offset = brace_offset(&text[index + 2..]).map(|colon| index + 2 + colon);
                push(Frame::Brace { arithmetic }, &mut stack, &mut open);
                step = 2;
            }
            '[' if inside
                || index
                    .checked_sub(1)
                    .is_some_and(|before| is_name_char(bytes[before] as char)) =>
            {
                push(Frame::Bracket, &mut stack, &mut open);
            }
            '(' if inside => push(Frame::Paren, &mut stack, &mut open),
            '{' if !stack.is_empty() => {
                push(Frame::Brace { arithmetic: false }, &mut stack, &mut open)
            }
            ')' => match stack.last() {
                Some(Frame::Paren) => {
                    stack.pop();
                }
                Some(Frame::Arithmetic) if next == Some(')') => {
                    stack.pop();
                    step = 2;
                }
                _ => {}
            },
            ']' if stack.last() == Some(&Frame::Bracket) => {
                stack.pop();
            }
            '}' if matches!(stack.last(), Some(Frame::Brace { .. })) => {
                stack.pop();
            }
            _ => {}
        }
        index += step;
        if let Some(start) = open
            && !stack.iter().any(|frame| frame.arithmetic())
        {
            ranges.push(start..index);
            open = None;
        }
    }
    if let Some(start) = open {
        ranges.push(start..bytes.len());
    }
    ranges.extend(conditions(text));
    ranges
}

/// Where a `${` body (after the `${`) takes an offset: the colon of
/// `NAME:` when not followed by `-`, `=`, `?` or `+`.
fn brace_offset(body: &str) -> Option<usize> {
    let len = body
        .find(|ch: char| !is_name_char(ch))
        .unwrap_or(body.len());
    let rest = &body[len..];
    (len > 0 && rest.starts_with(':') && !rest[1..].starts_with(['-', '=', '?', '+']))
        .then_some(len)
}

/// `[[ ... ]]` spans holding an arithmetic operator.
fn conditions(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("[[").map(|at| from + at) {
        let close = text[open + 2..]
            .find("]]")
            .map_or(text.len(), |at| open + 2 + at + 2);
        let span = &text[open..close];
        if span
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | '!'))
            .any(|word| TEST_OPERATORS.contains(&word))
        {
            found.push(open..close);
        }
        from = close.max(open + 2);
    }
    found
}

#[cfg(test)]
#[path = "arithmetic_tests.rs"]
mod tests;
