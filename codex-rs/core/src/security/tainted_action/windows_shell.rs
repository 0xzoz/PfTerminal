//! #1240: command text as cmd and PowerShell read it, for the classifier's
//! Windows view. The POSIX lexer removes backslashes (they escape in a POSIX
//! shell), so a Windows path in a cmd or PowerShell line (`type
//! C:\Users\me\.corbanu\auth.json`) reaches it as one meaningless word. This
//! pass reads the same text again with `\` kept: words split at blanks and
//! at cmd and PowerShell operators, double quotes (and, in one of two
//! readings, single quotes) grouping, `^` and `` ` `` escapes dropped. Every
//! word may be a path; it is resolved under Windows rules against the
//! working folder and every folder a `cd`/`Set-Location` in the text names.
//! A word after a writing command (`copy`, `Set-Content`, `Out-File`, ...),
//! a redirection (`>`, `>>`, `2>`) or a writing parameter (`-OutFile`) is a
//! write. Where unsure the pass over-approximates; it only adds findings.

use super::CLI_NAMES;
use super::CREDENTIAL_COMMANDS;
use super::Classifier;
use super::FOLDER_FLAGS;
use super::GitConfigWrites;
use super::MAX_FOLDER_BYTES;
use super::ProtectedActionKind;
use super::RECURSIVE_READERS;
use super::WRITERS;
use super::code_variables;
use super::has_flag;
use super::persistence_command;
use super::windows_paths;
use super::work;
use base64::Engine;

/// Folders one command's words resolve against: the working folder, the
/// folder the line has moved to, and folder flags' values. Past it the
/// command fails closed.
const MAX_FOLDERS: usize = 8;
/// cmd and PowerShell commands (and aliases) that write, move or remove the
/// paths after them, besides the POSIX [`WRITERS`].
const WINDOWS_WRITERS: &[&str] = &[
    "copy",
    "xcopy",
    "robocopy",
    "move",
    "ren",
    "rename",
    "mklink",
    "del",
    "erase",
    "rd",
    "md",
    "replace",
    "attrib",
    "icacls",
    "takeown",
    "expand",
    "certutil",
    "bitsadmin",
    "set-content",
    "sc",
    "add-content",
    "ac",
    "out-file",
    "tee-object",
    "copy-item",
    "cpi",
    "move-item",
    "mi",
    "rename-item",
    "rni",
    "remove-item",
    "ri",
    "new-item",
    "ni",
    "clear-content",
    "clc",
    "set-item",
    "si",
    "export-csv",
    "export-clixml",
    "expand-archive",
    "compress-archive",
    "invoke-webrequest",
    "iwr",
    "invoke-restmethod",
    "irm",
    "start-bitstransfer",
    "set-acl",
];
/// .NET methods that write (`[IO.File]::WriteAllText(...)`), by prefix.
const WRITING_METHODS: &[&str] = &[
    "write", "append", "copy", "move", "create", "replace", "delete", "open", "set", "encrypt",
    "decrypt",
];
/// PowerShell parameters whose value is a file written. PowerShell takes
/// any unambiguous prefix of a parameter name.
const WRITING_PARAMETERS: &[&str] = &[
    "-outfile",
    "-destination",
    "-destinationpath",
    "-filepath",
    "-redirectstandardoutput",
    "-redirectstandarderror",
];
/// Commands that copy or archive a whole folder tree.
const WINDOWS_TREE_COPIERS: &[&str] = &["xcopy", "robocopy", "compress-archive"];
/// Commands that do so with a recursive flag.
const WINDOWS_RECURSIVE: &[&str] = &[
    "copy-item",
    "cpi",
    "copy",
    "select-string",
    "sls",
    "findstr",
];
/// Commands that change the folder later commands run in.
const CHANGE_FOLDER: &[&str] = &[
    "cd",
    "chdir",
    "set-location",
    "sl",
    "pushd",
    "push-location",
];

/// One command of cmd or PowerShell text: its words (quotes and escapes
/// removed) and which of them a redirection writes.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Command {
    pub(super) words: Vec<String>,
    pub(super) redirected: Vec<bool>,
    /// Whether a command separator (`&`, `|`, `;`, a line end) or the end
    /// of the text ended it, rather than a brace.
    pub(super) separated: bool,
}

/// How a reading takes quotes: grouping a word, or separating words.
#[derive(Clone, Copy)]
pub(super) struct Quotes {
    /// `'` groups a word (PowerShell); else it separates words (cmd keeps
    /// it as text; reading it as a separator keeps `echo don't > f` from
    /// hiding the redirection).
    pub(super) single: bool,
    /// `"` groups a word; else it separates words, so a command line
    /// quoted whole (`cmd /c "echo x > f"`) is read as one.
    pub(super) double: bool,
}

/// The readings each text gets.
const READINGS: [Quotes; 3] = [
    Quotes {
        single: true,
        double: true,
    },
    Quotes {
        single: false,
        double: true,
    },
    Quotes {
        single: false,
        double: false,
    },
];

/// The commands of `text`, read with `quotes`.
pub(super) fn commands(text: &str, quotes: Quotes) -> Vec<Command> {
    let chars: Vec<char> = text.chars().collect();
    let mut done = Vec::new();
    let mut current = Command::default();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut redirect = false;
    let flush = |word: &mut String, current: &mut Command, redirect: &mut bool| {
        if !word.is_empty() {
            current.words.push(std::mem::take(word));
            current.redirected.push(std::mem::take(redirect));
        }
    };
    let end =
        |done: &mut Vec<Command>, current: &mut Command, redirect: &mut bool, separated: bool| {
            *redirect = false;
            current.separated = separated;
            let command = std::mem::take(current);
            if !command.words.is_empty() {
                done.push(command);
            }
        };
    let mut at = 0;
    while at < chars.len() {
        work::step();
        let ch = chars[at];
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else if ch != '`' {
                word.push(ch);
            }
            at += 1;
            continue;
        }
        match ch {
            '"' if quotes.double => quote = Some('"'),
            '\'' if quotes.single => quote = Some('\''),
            '`' | '^' => {}
            // A POSIX escaped `;` (`find ... -exec cmd {} \;`), not a path.
            '\\' if chars.get(at + 1) == Some(&';') => {
                word.push(';');
                at += 1;
            }
            '>' => {
                // A stream number (`2>`, `*>`) belongs to the operator.
                if word.chars().all(|ch| ch.is_ascii_digit() || ch == '*') {
                    word.clear();
                }
                flush(&mut word, &mut current, &mut redirect);
                if chars.get(at + 1) == Some(&'>') {
                    at += 1;
                }
                // `2>&1`: a stream, not a file.
                if chars.get(at + 1) == Some(&'&')
                    && chars.get(at + 2).is_some_and(char::is_ascii_digit)
                {
                    at += 2;
                } else {
                    redirect = true;
                }
            }
            '|' | '&' | ';' | '\n' | '\r' | '{' | '}' => {
                flush(&mut word, &mut current, &mut redirect);
                let separated = !matches!(ch, '{' | '}');
                end(&mut done, &mut current, &mut redirect, separated);
            }
            ch if ch.is_whitespace() || matches!(ch, ',' | '(' | ')' | '<' | '\'' | '"') => {
                flush(&mut word, &mut current, &mut redirect);
            }
            ch => word.push(ch),
        }
        at += 1;
    }
    flush(&mut word, &mut current, &mut redirect);
    end(
        &mut done,
        &mut current,
        &mut redirect,
        /*separated*/ true,
    );
    done
}

/// A command word's name: lowercase, without its folder or a Windows
/// program extension (`C:\Git\bin\git.exe` is `git`).
fn command_name(word: &str) -> &str {
    let base = word
        .trim_start_matches('@')
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(word);
    [".exe", ".com", ".cmd", ".bat"]
        .iter()
        .find_map(|extension| base.strip_suffix(extension))
        .unwrap_or(base)
}

/// The paths a word may name: itself, an attached value (`-Path:C:\x`,
/// `/OUT:x`, `f=@x`) and a `file:` URL's path.
fn candidates(word: &str) -> Vec<&str> {
    let mut found = vec![word];
    if (word.starts_with('-') || word.starts_with('/'))
        && let Some((_, value)) = word.split_once(':')
    {
        found.push(value);
    }
    found.extend(word.split('=').skip(1));
    if word
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
    {
        found.push(word[5..].trim_start_matches('/'));
    }
    // A PowerShell provider path (`FileSystem::C:\x`,
    // `Microsoft.PowerShell.Core\FileSystem::~\x`).
    if let Some(at) = word.to_ascii_lowercase().find("filesystem::") {
        found.push(&word[at + "filesystem::".len()..]);
    }
    found
        .into_iter()
        .map(|candidate| candidate.trim_start_matches('@'))
        .filter(|candidate| {
            !candidate.is_empty()
                && !(candidate.starts_with('-') && !candidate.contains(['/', '\\']))
        })
        .collect()
}

/// Words a join may take as operands (`Join-Path a b c`).
const MAX_JOINED: usize = 3;
/// Words after `New-Item` its `-Path` and `-Name` are looked for in.
const MAX_PARAMETER_WORDS: usize = 16;

/// Paths a command builds from several words, each with the position of
/// its first word: `Join-Path a b`, `[IO.Path]::Combine(a, b)`, `a + b`, and
/// `New-Item -Path a -Name b`.
fn joined_paths(words: &[String], lower: &[String], names: &[&str]) -> Vec<(usize, String)> {
    let operand = |word: &String| !word.starts_with('-');
    let value_of = |flag: &str, shortest: usize, from: usize| {
        (from..words.len().saturating_sub(1))
            .take(MAX_PARAMETER_WORDS)
            .find(|at| {
                lower[*at].len() >= shortest
                    && lower[*at].starts_with('-')
                    && flag.starts_with(&lower[*at])
            })
            .map(|at| at + 1)
    };
    let mut found = Vec::new();
    for at in 0..words.len() {
        work::step();
        if names[at] == "join-path" || lower[at].ends_with("::combine") {
            let operands: Vec<usize> = (at + 1..words.len())
                .take(2 * MAX_JOINED)
                .filter(|index| operand(&words[*index]))
                .take(MAX_JOINED)
                .collect();
            if let [first, ..] = operands[..]
                && operands.len() > 1
            {
                let parts: Vec<&str> = operands
                    .iter()
                    .map(|index| words[*index].as_str())
                    .collect();
                found.push((first, parts.join("\\")));
            }
        }
        if words[at] == "+" && at > 0 && at + 1 < words.len() {
            found.push((at - 1, format!("{}{}", words[at - 1], words[at + 1])));
        }
        if matches!(names[at], "new-item" | "ni")
            && let Some(name) = value_of("-name", 3, at + 1)
        {
            let path = value_of("-path", 2, at + 1).or_else(|| {
                (at + 1..words.len())
                    .take(MAX_PARAMETER_WORDS)
                    .find(|index| operand(&words[*index]) && *index != name)
            });
            if let Some(path) = path {
                found.push((path, format!("{}\\{}", words[path], words[name])));
            }
        }
    }
    found
}

/// Whether `word` (lowercase) is a PowerShell parameter naming a file
/// written.
fn writing_parameter(word: &str) -> bool {
    word.len() >= 4
        && word.starts_with('-')
        && WRITING_PARAMETERS
            .iter()
            .any(|parameter| parameter.starts_with(word))
}

/// Whether `word` (lowercase) asks for a recursive copy or search.
fn recursive_flag(word: &str) -> bool {
    matches!(
        word,
        "-r" | "--recursive" | "-a" | "--archive" | "/s" | "/e" | "/mir"
    ) || (word.len() >= 3 && "-recurse".starts_with(word))
}

/// The text of PowerShell's `-EncodedCommand` (base64 of UTF-16LE).
fn decoded_command(word: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(word.trim())
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16(&units).ok()
}

/// Whether `word` (lowercase) may be one of PowerShell's encoded
/// parameters (`-EncodedCommand`, `-EncodedArguments` and the prefixes and
/// short names it accepts, also with `/`): any `-e…` parameter but
/// `-ExecutionPolicy`. Its value is decoded and judged, or fails closed.
fn encoded_parameter(word: &str) -> bool {
    let word = word
        .strip_prefix('/')
        .map_or(word.to_string(), |rest| format!("-{rest}"));
    word.starts_with("-e") && !(word.len() >= 3 && "-executionpolicy".starts_with(word.as_str()))
}

/// Whether `word` (lowercase) is a folder-changing command with its
/// folder attached (`cd..`, `cd\Users`, `chdir\x`): the folder.
fn attached_folder(word: &str) -> Option<&str> {
    ["chdir", "cd"].iter().find_map(|name| {
        word.strip_prefix(name)
            .filter(|rest| rest.starts_with(['.', '\\', '/']))
    })
}

/// Whether `word` (as written; `lower` in lower case) names the folder the
/// next word is: a `-C`-style flag, `cd`'s and `start`'s `/d`,
/// PowerShell's `-WorkingDirectory` (`-wd`, or a prefix).
fn folder_flag(word: &str, lower: &str) -> bool {
    FOLDER_FLAGS.contains(&word) || {
        let word = lower;
        word == "/d" || word == "-wd" || (word.len() >= 3 && "-workingdirectory".starts_with(word))
    }
}

/// Words of one command that name programs judged by name (the Corbanu
/// CLI, credential tools, schedulers, git); past it the command fails
/// closed, so judging each with the words after it stays linear.
const MAX_NAMED_PROGRAMS: usize = 8;

/// Whether `expanded` (a word with the known home variables expanded)
/// leaves an unknown variable by `..` (`%APPDATA%\..\..\.bashrc`): where it
/// lands cannot be known.
fn climbs_out_of_a_variable(expanded: &str) -> bool {
    let lower = expanded.to_lowercase();
    let start = [lower.find('%'), lower.find("$env:"), lower.find("${env:")]
        .into_iter()
        .flatten()
        .min();
    start.is_some_and(|start| {
        lower[start..].split(['/', '\\']).any(|segment| {
            segment.trim_end_matches(['.', ' ']).is_empty() && segment.contains("..")
        })
    })
}

/// The drive and the rest of a drive-relative path (`C:x`, `C:`), which is
/// relative to that drive's own current folder.
fn drive_relative(word: &str) -> Option<(char, &str)> {
    let mut chars = word.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    let rest = &word[1..];
    let rest = rest.strip_prefix(':')?;
    (!rest.starts_with(['/', '\\', ':'])).then_some((letter, rest))
}

/// Whether `word` is relative to the current folder of a drive other than
/// one of `bases`' (normalized folders): that folder cannot be known.
fn on_another_drive(word: &str, bases: &[String]) -> bool {
    drive_relative(word).is_some_and(|(letter, _)| {
        bases.iter().any(|base| {
            !base
                .get(1..2)
                .is_some_and(|drive| drive.eq_ignore_ascii_case(&letter.to_string()))
        })
    })
}

impl Classifier {
    /// #1240: judge `command` (an argv, or a script as `sh -c text`) as cmd
    /// and PowerShell text, on its own and as one command line (Windows
    /// passes an argv to a program as one line; `cmd /c` and `powershell
    /// -Command` run the words after them as one). Each reading starts in
    /// the working folder.
    pub(super) fn windows_text(&mut self, command: &[String], cwd: &str, depth: usize) {
        let line = command
            .iter()
            .map(|arg| {
                work::step();
                if (arg.is_empty() || arg.contains(char::is_whitespace)) && !arg.contains('"') {
                    format!("\"{arg}\"")
                } else {
                    arg.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        if windows_paths::hides_a_home(&line) {
            self.note(ProtectedActionKind::UnseenCode);
        }
        let cwd_path = windows_paths::normalize(cwd);
        for text in std::iter::once(&line).chain(command) {
            work::step();
            let text = windows_paths::cmd_spellings(text);
            for quotes in READINGS {
                work::step();
                let mut folder = cwd_path.clone();
                for found in commands(&text, quotes) {
                    work::step();
                    self.windows_command(&found, &cwd_path, &mut folder, depth);
                }
            }
        }
    }

    /// One cmd or PowerShell command. `folder`: where the line has moved
    /// to (`cd`), followed in order; words also resolve against the working
    /// folder `cwd` (a `cd` may not have run).
    fn windows_command(&mut self, command: &Command, cwd: &str, folder: &mut String, depth: usize) {
        let words = &command.words;
        let lower: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
        let names: Vec<&str> = lower.iter().map(|word| command_name(word)).collect();
        // Folders the line moves to (a `cd` anywhere: after `cmd /c`, `if`,
        // `@`), and folder flags: their values are where a command runs,
        // not what it reads.
        let mut folder_values = vec![false; words.len()];
        let mut flagged: Vec<String> = Vec::new();
        for (index, word) in words.iter().enumerate() {
            work::step();
            let changes_folder = |at: usize| at > 0 && CHANGE_FOLDER.contains(&names[at - 1]);
            let folder_parameter =
                |at: usize| matches!(lower[at].as_str(), "/d" | "-path" | "-literalpath" | "-lp");
            // `cd x`, `cd..`: the line moves; a flag: this command runs there.
            let (target, moves) = if let Some(target) = attached_folder(&lower[index]) {
                (Some(target), true)
            } else if (changes_folder(index) && !folder_parameter(index))
                || (index > 1 && changes_folder(index - 1) && folder_parameter(index - 1))
            {
                (Some(word.as_str()), true)
            } else if index > 0 && folder_flag(&words[index - 1], &lower[index - 1]) {
                (Some(word.as_str()), false)
            } else {
                let value = word
                    .split_once('=')
                    .filter(|(flag, _)| FOLDER_FLAGS.contains(flag))
                    .map(|(_, value)| value);
                (value, false)
            };
            // A bare drive (`D:`) moves to that drive's current folder.
            let (target, moves) = match target {
                None if words.len() == 1
                    && command.separated
                    && drive_relative(word).is_some_and(|(_, rest)| rest.is_empty()) =>
                {
                    (Some(word.as_str()), true)
                }
                other => (other, moves),
            };
            let Some(target) = target.and_then(|target| candidates(target).pop()) else {
                continue;
            };
            folder_values[index] = true;
            if on_another_drive(target, std::slice::from_ref(folder)) {
                self.note(ProtectedActionKind::UnseenCode);
            }
            let resolved = self.homes.resolve(target, folder);
            if resolved.len() > MAX_FOLDER_BYTES || (!moves && flagged.len() + 2 >= MAX_FOLDERS) {
                self.note(ProtectedActionKind::UnseenCode);
            } else if moves {
                *folder = resolved;
            } else {
                flagged.push(resolved);
            }
        }
        let mut bases = vec![cwd.to_string()];
        for base in std::iter::once(folder.clone()).chain(flagged) {
            work::step();
            if !bases.contains(&base) {
                bases.push(base);
            }
        }
        let bases = &bases;
        let writer = names.iter().position(|name| {
            WINDOWS_WRITERS.contains(name)
                || WRITERS.contains(name)
                || name.rsplit_once("::").is_some_and(|(_, method)| {
                    WRITING_METHODS
                        .iter()
                        .any(|prefix| method.starts_with(prefix))
                })
        });
        // Flags looked for once per command, not once per command word.
        let recursive_flags = lower.iter().any(|word| recursive_flag(word));
        let readers_flagged: Vec<bool> = RECURSIVE_READERS
            .iter()
            .map(|(_, flags)| {
                work::step();
                flags.is_empty()
                    || lower
                        .iter()
                        .any(|word| flags.iter().any(|flag| has_flag(word, flag)))
            })
            .collect();
        let recursive = names.iter().any(|name| {
            WINDOWS_TREE_COPIERS.contains(name)
                || (recursive_flags && WINDOWS_RECURSIVE.contains(name))
                || RECURSIVE_READERS
                    .iter()
                    .zip(&readers_flagged)
                    .any(|((reader, _), flagged)| *flagged && reader == name)
        });
        let powershell = names
            .iter()
            .any(|name| matches!(*name, "powershell" | "pwsh"));
        let written = |index: usize| {
            command.redirected[index]
                || writer.is_some_and(|at| index > at)
                || (index > 0 && writing_parameter(&lower[index - 1]))
        };
        let mut git_config = GitConfigWrites::new(&lower);
        let mut named_programs = 0;
        for (index, word) in words.iter().enumerate() {
            work::step();
            let name = names[index];
            let args = &lower[index + 1..];
            // The checks the POSIX pass makes by command name, for names
            // only this reading sees (`C:\Tools\corbanu.exe vault list`).
            let named = CLI_NAMES.contains(&name)
                || name == "git"
                || matches!(name, "crontab" | "launchctl" | "systemctl")
                || CREDENTIAL_COMMANDS
                    .iter()
                    .any(|(credential_command, _)| *credential_command == name);
            if named {
                named_programs += 1;
            }
            if named_programs > MAX_NAMED_PROGRAMS {
                self.note(ProtectedActionKind::UnseenCode);
            } else if named {
                let imported = index > 0 && matches!(lower[index - 1].as_str(), "from" | "import");
                if CLI_NAMES.contains(&name) && !imported {
                    self.cli(args);
                }
                if persistence_command(name, args)
                    || (name == "git" && git_config.persists(index + 1, /*subcommand*/ None))
                {
                    self.note(ProtectedActionKind::Persistence);
                }
                if name == "git"
                    && code_variables::git_runs_code(&words[index + 1..], &words[index + 1..])
                {
                    self.note(ProtectedActionKind::UnseenCode);
                }
                for (credential_command, verbs) in CREDENTIAL_COMMANDS {
                    work::step();
                    if name == *credential_command
                        && args.iter().any(|arg| verbs.contains(&arg.as_str()))
                    {
                        self.note(ProtectedActionKind::Credentials);
                    }
                }
            }
            if powershell && index > 0 && encoded_parameter(&lower[index - 1]) {
                match decoded_command(word) {
                    Some(text) => self.script(&text, folder, depth + 1),
                    None => self.note(ProtectedActionKind::UnseenCode),
                }
            }
            for candidate in candidates(word) {
                work::step();
                self.windows_path(
                    candidate,
                    bases,
                    written(index),
                    recursive && !folder_values[index],
                );
            }
        }
        for (index, path) in joined_paths(words, &lower, &names) {
            work::step();
            self.windows_path(&path, bases, written(index), recursive);
        }
    }

    /// One word that may be a path, resolved against each folder. A path
    /// whose place cannot be known fails closed: one on another drive's
    /// current folder (`D:x` from `C:\work`), or one climbing out of an
    /// unknown variable (`%APPDATA%\..\..\.bashrc`).
    fn windows_path(&mut self, candidate: &str, bases: &[String], written: bool, recursive: bool) {
        let path_like =
            candidate.contains(['/', '\\']) || candidate.starts_with(['.', '~', '%', '$']);
        let expanded = windows_paths::expand(
            candidate,
            &windows_paths::Values {
                codex_home: self.homes.codex_home(),
                user_home: self.homes.user_home(),
                cwd: "",
            },
        );
        if climbs_out_of_a_variable(&expanded) {
            self.note(ProtectedActionKind::UnseenCode);
        }
        // Written, or spelled as a path (`D:.netrc`, `D:x\y`); not any
        // `x:y` word (`e:true:`, `a::b`).
        let spelled_as_path = drive_relative(&expanded).is_some_and(|(_, rest)| {
            written || rest.starts_with('.') || rest.contains(['/', '\\'])
        });
        if spelled_as_path && on_another_drive(&expanded, bases) {
            self.note(ProtectedActionKind::UnseenCode);
        }
        for base in bases {
            work::step();
            let resolved = self.homes.resolve(candidate, base);
            if path_like {
                self.path(&resolved);
            } else if let Some(kind) = self.homes.classify_path(&resolved) {
                self.note(kind);
            }
            if written && self.homes.is_persistence(&resolved) {
                self.note(ProtectedActionKind::Persistence);
            }
            if recursive && self.homes.holds_a_home(&resolved) {
                self.note(ProtectedActionKind::Credentials);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn words(text: &str, single_quotes: bool) -> Vec<(Vec<&'static str>, Vec<bool>)> {
        commands(
            text,
            Quotes {
                single: single_quotes,
                double: true,
            },
        )
        .into_iter()
        .map(|command| {
            (
                command
                    .words
                    .into_iter()
                    .map(|word| &*Box::leak(word.into_boxed_str()))
                    .collect(),
                command.redirected,
            )
        })
        .collect()
    }

    #[test]
    fn issue_1240_windows_commands_split_as_cmd_and_powershell_do() {
        assert_eq!(
            words(
                r"type C:\Users\me\.corbanu\auth.json",
                /*single_quotes*/ true
            ),
            vec![(
                vec!["type", r"C:\Users\me\.corbanu\auth.json"],
                vec![false, false]
            )]
        );
        assert_eq!(
            words(
                r#"echo x >> "C:\Users\John Smith\.bashrc" 2>&1 & dir"#,
                /*single_quotes*/ true
            ),
            vec![
                (
                    vec!["echo", "x", r"C:\Users\John Smith\.bashrc"],
                    vec![false, false, true]
                ),
                (vec!["dir"], vec![false]),
            ]
        );
        assert_eq!(
            words(
                r"Set-Content -Path 'C:\a b\.zshrc' -Value x; [IO.File]::ReadAllText('C:\k')",
                /*single_quotes*/ true
            ),
            vec![
                (
                    vec!["Set-Content", "-Path", r"C:\a b\.zshrc", "-Value", "x"],
                    vec![false; 5]
                ),
                (vec!["[IO.File]::ReadAllText", r"C:\k"], vec![false, false]),
            ]
        );
        // cmd keeps `'` as text: read as a separator it cannot hide `>`.
        assert_eq!(
            words(r"echo don't > C:\x\.bashrc", /*single_quotes*/ false),
            vec![(
                vec!["echo", "don", "t", r"C:\x\.bashrc"],
                vec![false, false, false, true]
            )]
        );
        assert_eq!(
            words(
                r"ty^pe C:\x\.cor`banu\auth.json",
                /*single_quotes*/ true
            ),
            vec![(vec!["type", r"C:\x\.corbanu\auth.json"], vec![false, false])]
        );
        assert_eq!(command_name(r"c:\program files\git\bin\git.exe"), "git");
        assert_eq!(command_name("corbanu.cmd"), "corbanu");
        assert_eq!(candidates(r"-Path:C:\x"), vec![r"-Path:C:\x", r"C:\x"]);
        assert_eq!(candidates("f=@C:/k"), vec!["f=@C:/k", "C:/k"]);
        assert_eq!(
            candidates("file:///C:/Users/x/.ssh/id_rsa"),
            vec!["file:///C:/Users/x/.ssh/id_rsa", "C:/Users/x/.ssh/id_rsa"]
        );
        assert!(candidates("-Recurse").is_empty());
        assert!(writing_parameter("-outf"));
        assert!(!writing_parameter("-o"));
        assert!(recursive_flag("-rec"));
        assert!(encoded_parameter("-enc"));
        assert!(encoded_parameter("/ec"));
        assert!(encoded_parameter("-encodedarguments"));
        assert!(!encoded_parameter("-executionpolicy"));
        assert!(!encoded_parameter("-ex"));
        assert_eq!(attached_folder(r"cd\users"), Some(r"\users"));
        assert_eq!(attached_folder("cd.."), Some(".."));
        assert_eq!(attached_folder("cdrom"), None);
        assert!(climbs_out_of_a_variable(r"%APPDATA%\..\..\.bashrc"));
        assert!(climbs_out_of_a_variable(r"$env:TEMP\..\.bashrc"));
        assert!(!climbs_out_of_a_variable(r"%TEMP%\out.txt"));
        assert!(!climbs_out_of_a_variable(r"..\%TEMP%"));
        assert_eq!(drive_relative("D:x"), Some(('D', "x")));
        assert_eq!(drive_relative("D:"), Some(('D', "")));
        assert_eq!(drive_relative(r"D:\x"), None);
        assert_eq!(drive_relative("a::b"), None);
        assert_eq!(drive_relative("ab:x"), None);
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            "type x"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        );
        assert_eq!(decoded_command(&encoded).as_deref(), Some("type x"));
    }
}
