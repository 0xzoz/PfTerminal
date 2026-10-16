//! #885: a quote-aware reading of shell text, beside the word lexer in
//! `shell.rs` (which removes quotes on purpose). It keeps each quoted word
//! whole, so the script a nested shell runs (`bash -c "sh -c './x'"`) and
//! the program a sed command gets are judged as the shell passes them on.
//! Substitutions are read as commands of their own; here-documents and
//! here-strings are kept as the text fed to their command.

use super::shell::UNSEEN_OUTPUT;
use super::shell::ansi_c;

#[derive(Clone, Debug, Default)]
pub(super) struct Word {
    /// The text the command receives. A substitution's output is
    /// `UNSEEN_OUTPUT`; a parameter expansion is kept as written.
    pub(super) text: String,
    /// Some of it was quoted or escaped, so the word lexer may split it.
    pub(super) quoted: bool,
    /// It holds an expansion whose value the classifier does not know.
    pub(super) dynamic: bool,
    /// An expansion outside double quotes: it may split into several words.
    pub(super) splits: bool,
}

#[derive(Debug, Default)]
pub(super) struct Command {
    pub(super) words: Vec<Word>,
    /// Words its output is redirected to (`> f`, `>> f`, `&> f`).
    pub(super) writes_to: Vec<Word>,
    /// Its output feeds a pipe: the next command of the pipeline, when it
    /// is in the same list of commands.
    pub(super) piped: bool,
    pub(super) pipe_to: Option<usize>,
    /// Its output goes to a descriptor other than 1 or 2 (`>&3`), which
    /// may be any file.
    pub(super) writes_descriptor: bool,
    /// Here-strings and here-document bodies fed to its input.
    pub(super) input: Vec<Word>,
    /// It runs inside a substitution, in a shell of its own.
    pub(super) substituted: bool,
}

impl Command {
    fn is_empty(&self) -> bool {
        self.words.is_empty() && self.writes_to.is_empty() && self.input.is_empty() && !self.piped
    }
}

/// Substitutions nested deeper than this are not read (fail closed).
const MAX_NESTING: usize = 32;

/// The simple commands of some shell text.
pub(super) struct Reading {
    /// Substitutions included. A command inside a group (`( ... )`,
    /// `{ ...; }`) is not linked to the group's pipe or redirections; the
    /// group's own (empty) command holds them.
    pub(super) commands: Vec<Command>,
    /// Substitutions nested too deeply to read: the rest was not read.
    pub(super) too_deep: bool,
    /// A quote, substitution or `${...}` was not closed: a shell would
    /// refuse the line, and this reading may not match how it splits.
    pub(super) unterminated: bool,
}

/// Read `text`, substitutions included.
pub(super) fn commands(text: &str) -> Reading {
    read(text, /*nesting*/ 0, /*body*/ false)
}

/// Read `text` (`body`: the body of an unquoted here-document, where only
/// substitutions run) inside `nesting` substitutions.
fn read(text: &str, nesting: usize, body: bool) -> Reading {
    let chars: Vec<char> = text.chars().collect();
    let mut reader = Reader {
        chars: &chars,
        out: Vec::new(),
        nesting,
        too_deep: false,
        unterminated: false,
    };
    if body {
        reader.substitutions_only();
    } else {
        let mut index = 0;
        while index < chars.len() {
            index = reader.list(index, /*nested*/ false);
        }
    }
    // Drop empty commands, keeping pipe links.
    let mut kept = Vec::with_capacity(reader.out.len());
    let mut next = 0;
    for command in &reader.out {
        kept.push((!command.is_empty()).then(|| {
            next += 1;
            next - 1
        }));
    }
    let mut out: Vec<Command> = Vec::with_capacity(next);
    for mut command in reader.out {
        if command.is_empty() {
            continue;
        }
        command.pipe_to = command
            .pipe_to
            .and_then(|to| kept.get(to).copied().flatten());
        out.push(command);
    }
    Reading {
        commands: out,
        too_deep: reader.too_deep,
        unterminated: reader.unterminated,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Redirect {
    None,
    Stdin,
    Stdout,
    /// Output to a descriptor other than 1 or 2 (`>&3`).
    Descriptor,
    HereString,
    /// `<<` or `<<-` (strip leading tabs).
    HereDocument(bool),
}

struct Reader<'a> {
    chars: &'a [char],
    out: Vec<Command>,
    /// Substitutions open around the text being read.
    nesting: usize,
    /// They nested past `MAX_NESTING`; reading stopped.
    too_deep: bool,
    /// Something was left open at the end.
    unterminated: bool,
}

/// One list of commands being read.
struct List {
    command: usize,
    word: Word,
    has_word: bool,
    redirect: Redirect,
    /// Here-documents whose bodies start after the next newline.
    pending: Vec<HereDocument>,
    /// `case` commands open in this list: their patterns end with `)`.
    cases: usize,
}

struct HereDocument {
    delimiter: String,
    /// `<<-`: leading tabs are stripped.
    strip: bool,
    /// The delimiter was quoted, so the body is not expanded.
    quoted: bool,
    command: usize,
}

impl Reader<'_> {
    fn new_command(&mut self) -> usize {
        self.out.push(Command {
            substituted: self.nesting > 0,
            ..Command::default()
        });
        self.out.len() - 1
    }

    fn finish_word(&mut self, list: &mut List) {
        if !std::mem::take(&mut list.has_word) {
            return;
        }
        let word = std::mem::take(&mut list.word);
        let command = &mut self.out[list.command];
        if list.redirect == Redirect::None && command.words.is_empty() && !word.quoted {
            match word.text.as_str() {
                "case" => list.cases += 1,
                "esac" => list.cases = list.cases.saturating_sub(1),
                _ => {}
            }
        }
        match std::mem::replace(&mut list.redirect, Redirect::None) {
            Redirect::None => command.words.push(word),
            Redirect::Stdin | Redirect::Descriptor => {}
            Redirect::Stdout => command.writes_to.push(word),
            Redirect::HereString => command.input.push(word),
            Redirect::HereDocument(strip) => list.pending.push(HereDocument {
                quoted: word.quoted,
                delimiter: word.text,
                strip,
                command: list.command,
            }),
        }
    }

    fn finish_command(&mut self, list: &mut List) {
        self.finish_word(list);
        if !self.out[list.command].is_empty() {
            list.command = self.new_command();
        }
    }

    /// Read commands from `index` to the end, or (`nested`) to the `)` that
    /// closes a substitution; returns the index after it.
    fn list(&mut self, index: usize, nested: bool) -> usize {
        if !self.enter(nested) {
            return self.chars.len();
        }
        let end = self.read_list(index, nested);
        self.nesting -= usize::from(nested);
        end
    }

    /// Open one more substitution (when `nested`); `false` past the limit.
    fn enter(&mut self, nested: bool) -> bool {
        if self.too_deep || (nested && self.nesting >= MAX_NESTING) {
            self.too_deep = true;
            return false;
        }
        self.nesting += usize::from(nested);
        true
    }

    fn read_list(&mut self, mut index: usize, nested: bool) -> usize {
        let chars = self.chars;
        let mut list = List {
            command: self.new_command(),
            word: Word::default(),
            has_word: false,
            redirect: Redirect::None,
            pending: Vec::new(),
            cases: 0,
        };
        let mut depth = 0usize;
        while index < chars.len() {
            let ch = chars[index];
            let next = chars.get(index + 1).copied();
            let previous = index.checked_sub(1).map(|at| chars[at]);
            match ch {
                // A `case` pattern's `)` does not close the substitution
                // (`esac)` closes the case first).
                ')' if nested && depth == 0 && {
                    self.finish_word(&mut list);
                    list.cases == 0
                } =>
                {
                    return index + 1;
                }
                ' ' | '\t' => self.finish_word(&mut list),
                '\n' => {
                    self.finish_command(&mut list);
                    index = self.here_documents(index + 1, &mut list.pending);
                    continue;
                }
                '&' if next == Some('>') => {
                    self.finish_word(&mut list);
                    list.redirect = Redirect::Stdout;
                    index += if chars.get(index + 2) == Some(&'>') {
                        3
                    } else {
                        2
                    };
                    continue;
                }
                ';' | '&' => self.finish_command(&mut list),
                '|' => {
                    self.finish_word(&mut list);
                    let from = list.command;
                    let pipe = next != Some('|') && previous != Some('|');
                    self.finish_command(&mut list);
                    if pipe {
                        // After a group (`( ... ) | tee f`) the piped
                        // command is the group's empty one.
                        if from == list.command {
                            list.command = self.new_command();
                        }
                        self.out[from].piped = true;
                        self.out[from].pipe_to = Some(list.command);
                    }
                }
                '(' | ')' => {
                    if nested {
                        depth = if ch == '(' {
                            depth + 1
                        } else {
                            depth.saturating_sub(1)
                        };
                    }
                    self.finish_command(&mut list);
                }
                '{' if !list.has_word && next.is_none_or(char::is_whitespace) => {
                    self.finish_command(&mut list);
                }
                '}' if !list.has_word
                    && next.is_none_or(|next| next.is_whitespace() || ";|&)".contains(next)) =>
                {
                    self.finish_command(&mut list);
                }
                '#' if !list.has_word => {
                    while index < chars.len() && chars[index] != '\n' {
                        index += 1;
                    }
                    continue;
                }
                '\'' => {
                    list.has_word = true;
                    list.word.quoted = true;
                    index += 1;
                    while index < chars.len() && chars[index] != '\'' {
                        list.word.text.push(chars[index]);
                        index += 1;
                    }
                    self.unterminated |= index >= chars.len();
                }
                '"' => {
                    list.has_word = true;
                    list.word.quoted = true;
                    index = self.double_quoted(index + 1, &mut list.word);
                    continue;
                }
                '\\' => {
                    match next {
                        // A line continuation.
                        Some('\n') => {}
                        Some(escaped) => {
                            list.has_word = true;
                            list.word.quoted = true;
                            list.word.text.push(escaped);
                        }
                        None => {}
                    }
                    index += 2;
                    continue;
                }
                '$' if next == Some('\'') => {
                    list.has_word = true;
                    list.word.quoted = true;
                    index = ansi_c(chars, index + 2, &mut list.word.text);
                    continue;
                }
                '$' | '`' => {
                    list.has_word = true;
                    index = self.expansion(index, &mut list.word, /*in_double*/ false);
                    continue;
                }
                '<' | '>' if next == Some('(') => {
                    // Process substitution: a file name the classifier
                    // cannot see, and commands of its own.
                    list.has_word = true;
                    list.word.text.push_str(UNSEEN_OUTPUT);
                    list.word.dynamic = true;
                    index = self.list(index + 2, /*nested*/ true);
                    continue;
                }
                '<' | '>' => {
                    // A descriptor number (`2>`) is not a word.
                    if list.has_word
                        && !list.word.quoted
                        && list.word.text.chars().all(|ch| ch.is_ascii_digit())
                    {
                        list.has_word = false;
                        list.word = Word::default();
                    }
                    self.finish_word(&mut list);
                    let (redirect, used) = redirection(&chars[index..]);
                    if redirect == Redirect::Descriptor {
                        self.out[list.command].writes_descriptor = true;
                        list.redirect = Redirect::None;
                    } else {
                        list.redirect = redirect;
                    }
                    index += used;
                    continue;
                }
                ch => {
                    list.has_word = true;
                    list.word.text.push(ch);
                }
            }
            index += 1;
        }
        self.finish_word(&mut list);
        self.unterminated |= nested;
        index
    }

    /// Text after `"` up to the closing quote; returns the index after it.
    fn double_quoted(&mut self, mut index: usize, word: &mut Word) -> usize {
        let chars = self.chars;
        while index < chars.len() {
            match chars[index] {
                '"' => return index + 1,
                '\\' => match chars.get(index + 1) {
                    Some('\n') => index += 2,
                    Some(escaped @ ('$' | '`' | '"' | '\\')) => {
                        word.text.push(*escaped);
                        index += 2;
                    }
                    _ => {
                        word.text.push('\\');
                        index += 1;
                    }
                },
                '$' | '`' => index = self.expansion(index, word, /*in_double*/ true),
                ch => {
                    word.text.push(ch);
                    index += 1;
                }
            }
        }
        self.unterminated = true;
        index
    }

    /// `$name`, `${...}`, `$(...)`, `$((...))` or a backtick substitution
    /// at `index`; returns the index after it.
    fn expansion(&mut self, index: usize, word: &mut Word, in_double: bool) -> usize {
        let chars = self.chars;
        let next = chars.get(index + 1).copied();
        let dynamic = |word: &mut Word| {
            word.dynamic = true;
            word.splits |= !in_double;
        };
        if chars[index] == '`' {
            let mut inner = String::new();
            let mut at = index + 1;
            while at < chars.len() && chars[at] != '`' {
                if chars[at] == '\\'
                    && let Some(escaped @ ('`' | '\\' | '$')) = chars.get(at + 1)
                {
                    inner.push(*escaped);
                    at += 2;
                    continue;
                }
                inner.push(chars[at]);
                at += 1;
            }
            self.unterminated |= at >= chars.len();
            self.nested_text(&inner, /*body*/ false);
            word.text.push_str(UNSEEN_OUTPUT);
            dynamic(word);
            return at + 1;
        }
        match next {
            Some('(') => {
                word.text.push_str(UNSEEN_OUTPUT);
                dynamic(word);
                self.list(index + 2, /*nested*/ true)
            }
            Some('{') => {
                // `${x:-$(...)}`: substitutions inside run too.
                if !self.enter(/*nested*/ true) {
                    return chars.len();
                }
                // Quotes inside it are honoured, as bash does.
                let mut scratch = Word::default();
                let mut at = index + 2;
                let mut open = 1usize;
                while at < chars.len() {
                    match chars[at] {
                        '$' if !in_double && chars.get(at + 1) == Some(&'\'') => {
                            at = ansi_c(chars, at + 2, &mut scratch.text);
                            continue;
                        }
                        '$' | '`' => {
                            at = self.expansion(at, &mut scratch, in_double);
                            continue;
                        }
                        '"' => {
                            at = self.double_quoted(at + 1, &mut scratch);
                            continue;
                        }
                        // Even inside double quotes (`"${x:-'}'}"`).
                        '\'' => {
                            at += 1;
                            while at < chars.len() && chars[at] != '\'' {
                                at += 1;
                            }
                        }
                        '\\' => at += 1,
                        '{' => open += 1,
                        '}' => {
                            open -= 1;
                            if open == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    at += 1;
                }
                self.unterminated |= at >= chars.len();
                self.nesting -= 1;
                let end = (at + 1).min(chars.len());
                word.text.extend(&chars[index.min(end)..end]);
                dynamic(word);
                end
            }
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
                let mut at = index + 1;
                while chars
                    .get(at)
                    .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                {
                    at += 1;
                }
                word.text.extend(&chars[index..at]);
                dynamic(word);
                at
            }
            Some(ch) if ch.is_ascii_digit() || "@*#?$!-".contains(ch) => {
                word.text.extend(&chars[index..index + 2]);
                dynamic(word);
                index + 2
            }
            // `$"..."` is a double-quoted string.
            Some('"') if !in_double => index + 1,
            _ => {
                word.text.push('$');
                index + 1
            }
        }
    }

    /// Commands of text inside a substitution (a backtick's text, or an
    /// unquoted here-document `body`, whose substitutions run).
    fn nested_text(&mut self, text: &str, body: bool) {
        if !self.enter(/*nested*/ true) {
            return;
        }
        let reading = read(text, self.nesting, body);
        self.nesting -= 1;
        self.too_deep |= reading.too_deep;
        self.unterminated |= reading.unterminated;
        let offset = self.out.len();
        self.out
            .extend(reading.commands.into_iter().map(|command| Command {
                substituted: true,
                pipe_to: command.pipe_to.map(|to| to + offset),
                ..command
            }));
    }

    /// The substitutions in a here-document body.
    fn substitutions_only(&mut self) {
        let chars = self.chars;
        let mut scratch = Word::default();
        let mut index = 0;
        while index < chars.len() {
            match chars[index] {
                '\\' => index += 2,
                '$' | '`' => index = self.expansion(index, &mut scratch, /*in_double*/ true),
                _ => index += 1,
            }
        }
    }

    /// Bodies of pending here-documents, from `index` (a line start);
    /// returns the index after the last delimiter line.
    fn here_documents(&mut self, mut index: usize, pending: &mut Vec<HereDocument>) -> usize {
        let chars = self.chars;
        for HereDocument {
            delimiter,
            strip,
            quoted,
            command,
        } in std::mem::take(pending)
        {
            let mut body = String::new();
            while index < chars.len() {
                let end = chars[index..]
                    .iter()
                    .position(|ch| *ch == '\n')
                    .map_or(chars.len(), |at| index + at);
                let line: String = chars[index..end].iter().collect();
                index = (end + 1).min(chars.len());
                let compared = if strip {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if compared == delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            if !quoted {
                self.nested_text(&body, /*body*/ true);
            }
            self.out[command].input.push(Word {
                text: body,
                quoted: true,
                dynamic: false,
                splits: false,
            });
        }
        index
    }
}

/// The redirection at the start of `chars` and its length.
fn redirection(chars: &[char]) -> (Redirect, usize) {
    match chars {
        ['<', '<', '<', ..] => (Redirect::HereString, 3),
        ['<', '<', '-', ..] => (Redirect::HereDocument(true), 3),
        ['<', '<', ..] => (Redirect::HereDocument(false), 2),
        // `<>` opens the file for writing too.
        ['<', '>', ..] => (Redirect::Stdout, 2),
        ['<', '&', ..] => (Redirect::Stdin, 2),
        ['<', ..] => (Redirect::Stdin, 1),
        // `>&2`, `2>&1`, `>&-`: standard output or error, or closed.
        ['>', '&', '1' | '2', after, ..] if after.is_ascii_digit() => (Redirect::Descriptor, 3),
        ['>', '&', '1' | '2' | '-', ..] => (Redirect::None, 3),
        // `>&3`: whatever file descriptor 3 is.
        ['>', '&', after, ..] if after.is_ascii_digit() => (Redirect::Descriptor, 3),
        ['>', '>' | '|' | '&', ..] => (Redirect::Stdout, 2),
        _ => (Redirect::Stdout, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn all(text: &str) -> Vec<Command> {
        let reading = commands(text);
        assert!(!reading.too_deep && !reading.unterminated, "{text:?}");
        reading.commands
    }

    fn texts(text: &str) -> Vec<Vec<String>> {
        all(text)
            .into_iter()
            .map(|command| command.words.into_iter().map(|word| word.text).collect())
            .collect()
    }

    #[test]
    fn quoted_words_stay_whole() {
        assert_eq!(
            texts("bash -c \"sh -c './missing.sh'\"; sed -n '1,5p' f | sort"),
            vec![
                vec!["bash", "-c", "sh -c './missing.sh'"],
                vec!["sed", "-n", "1,5p", "f"],
                vec!["sort"],
            ]
        );
        assert_eq!(
            texts("s\\ed $'\\x31e x' \"a\\\"b\" # c\necho `sed e` $(gsed e) x"),
            vec![
                vec!["sed", "1e x", "a\"b"],
                vec!["echo", UNSEEN_OUTPUT, UNSEEN_OUTPUT, "x"],
                vec!["sed", "e"],
                vec!["gsed", "e"],
            ]
        );
        let read = all("sed p f > r.sh 2>&1 | tee x; cat <<-'E' <<< w\n\tsed e\n\tE\nls");
        assert_eq!(read[0].writes_to[0].text, "r.sh");
        assert!(read[0].piped);
        assert_eq!(read[2].input[0].text, "w");
        assert_eq!(read[2].input[1].text, "\tsed e\n");
        assert_eq!(read[3].words[0].text, "ls");
        let substituted: Vec<bool> = all("cd a; echo $(cd b) `cd c` <(cd d)")
            .iter()
            .map(|command| command.substituted)
            .collect();
        assert_eq!(substituted, vec![false, false, true, true, true]);
        let dynamic = &all("sed \"$P\" '$p' ${Q}")[0].words;
        assert_eq!(
            dynamic
                .iter()
                .map(|word| (word.dynamic, word.splits))
                .collect::<Vec<_>>(),
            vec![(false, false), (true, false), (false, false), (true, true)]
        );
    }

    #[test]
    fn substitutions_redirections_and_nesting() {
        // Commands in `${...}` and in an unquoted here-document run; a
        // quoted one's body is only text.
        assert_eq!(
            texts("echo ${x:-$(sh -c a)}; cat <<E\n$(sh -c b)\nE\ncat <<'Q'\n$(sh -c c)\nQ"),
            vec![
                vec!["echo", "${x:-$(sh -c a)}"],
                vec!["sh", "-c", "a"],
                vec!["cat"],
                vec!["cat"],
                vec!["sh", "-c", "b"],
            ]
        );
        let read = all("sed p f | sort | tee r.sh; sed p f 1<>w.sh >&3");
        assert_eq!(read[0].pipe_to, Some(1));
        assert_eq!(read[1].pipe_to, Some(2));
        assert_eq!(read[2].pipe_to, None);
        assert_eq!(read[3].writes_to[0].text, "w.sh");
        assert!(read[3].writes_descriptor);
        assert!(!all("ls 2>&1 >&2")[0].writes_descriptor);
        // Too deep to read: refused, never a stack overflow.
        let deep = format!("{}x{}", "$(".repeat(100_000), ")".repeat(100_000));
        assert!(commands(&deep).too_deep);
        assert!(!commands(&"`".repeat(10_000)).too_deep);
        let backticks = format!("{}x", "\\`echo ".repeat(200));
        let _ = commands(&backticks);
        let braces = format!("{}x{}", "${a:-".repeat(100_000), "}".repeat(100_000));
        assert!(commands(&braces).too_deep);
        // Quotes in `${...}` and a `case` in a substitution keep the reading
        // in step with bash; what is left open is reported.
        assert_eq!(
            texts("echo ${x:-'}'}; bash -c x; echo \"$(case a in a) echo '\"' ;; esac)\"; sh -c y"),
            vec![
                vec!["echo", "${x:-'}'}"],
                vec!["bash", "-c", "x"],
                vec!["echo", UNSEEN_OUTPUT],
                vec!["case", "a", "in", "a"],
                vec!["echo", "\""],
                vec!["esac"],
                vec!["sh", "-c", "y"],
            ]
        );
        for open in ["echo 'a", "echo \"a", "echo $(a", "echo `a", "echo ${a"] {
            assert!(commands(open).unterminated, "{open:?}");
        }
        let group = all("(sed p f) | tee r.sh");
        assert_eq!(group[1].pipe_to, Some(2));
        assert!(group[1].words.is_empty());
    }
}
