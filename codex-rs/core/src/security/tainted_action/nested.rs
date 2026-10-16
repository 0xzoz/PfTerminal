//! #885: what the word lexer cannot see because it removes quotes. A nested
//! shell's script (`bash -c "sh -c './x.sh'"`) is judged like a command
//! line of its own, and a sed program is judged whole: one that may run a
//! shell command, or that saves what it outputs to a file the same line
//! runs, is code the host cannot read.

use super::Classifier;
use super::FileText;
use super::Folders;
use super::ProtectedActionKind;
use super::WRITERS;
use super::indirect;
use super::invocation;
use super::invocation::Code;
use super::outbound;
use super::quoted;
use super::quoted::Command;
use super::quoted::Word;
use super::sed;
use super::shell::SHELLS;
use super::shell::basename;
use std::collections::HashMap;
use std::collections::HashSet;

/// Levels of quoted text read for nested commands. Deeper text is still
/// split by the word lexer, and a sed only it sees fails closed.
const MAX_QUOTED_LEVEL: usize = 4;

/// Pipeline stages that save their input to the files they are given.
const PIPE_WRITERS: &[&str] = &["tee", "dd", "sponge", "cp", "install"];

/// What the pass found in one command line.
#[derive(Default)]
struct Found {
    /// Scripts that nested shells run, with the folder they run in.
    scripts: Vec<(String, String)>,
    seds: Vec<Sed>,
    /// sed commands found (each may be read more than one way).
    sed_commands: usize,
    /// Text it could not read in full: substitutions nested too deeply, or
    /// too many places a command may start.
    unreadable: bool,
    /// Files the line writes (`None`: one it cannot name).
    written: Vec<Option<String>>,
    /// A command other than sed writes a file (a copy, move, download or
    /// redirection), which may carry what sed output.
    other_writes: bool,
    /// Redirections of compound commands (`{ ...; } > f`, `done > f`,
    /// `exec 3> f`): any command's output may go there.
    shared_targets: Vec<Option<String>>,
}

/// One reading of a sed invocation and where it runs.
struct Sed {
    call: sed::Call,
    cwd: String,
    /// Where its output is saved besides in place: its redirections and
    /// the files later pipeline stages write.
    saved_to: Vec<Option<String>>,
    /// It gets more arguments at run time (`xargs sed`, `parallel sed`)...
    fed: bool,
    /// ...that go into its words (`xargs -I{}`, `{}`).
    replaced: bool,
}

/// What quoted text is to the command that gets it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Text the command does not run.
    Data,
    /// Code an interpreter runs.
    Code,
    /// A nested shell's script.
    Script,
}

fn name(word: &str) -> String {
    basename(&word.to_lowercase()).to_string()
}

/// A word's text when the classifier knows it.
fn known(word: &Word) -> Option<String> {
    (!word.dynamic).then(|| word.text.clone())
}

impl Classifier {
    /// `lexed_seds`: how many sed commands the word lexer found in the line.
    pub(super) fn quoted_pass(
        &mut self,
        command: &[String],
        cwd: &str,
        depth: usize,
        lexed_seds: usize,
    ) {
        // The words as given, and a shell's `-c` script as a line of its
        // own, as the word lexer reads them.
        let mut argv = Command::default();
        let mut scripts = Vec::new();
        let mut previous: Option<&str> = None;
        for arg in command {
            let in_shell = argv
                .words
                .iter()
                .any(|word| SHELLS.contains(&name(&word.text).as_str()));
            let script = in_shell
                && previous.is_some_and(|flag| {
                    flag.starts_with('-') && !flag.starts_with("--") && flag.contains('c')
                });
            if script {
                scripts.push(arg.as_str());
            } else {
                argv.words.push(Word {
                    text: arg.clone(),
                    quoted: true,
                    ..Word::default()
                });
            }
            previous = Some(arg);
        }
        let mut folders = Folders {
            cwd: cwd.to_string(),
            previous: cwd.to_string(),
            stack: Vec::new(),
        };
        let mut found = Found::default();
        self.walk(
            &[argv],
            &mut folders,
            /*level*/ 0,
            /*code*/ false,
            &mut found,
        );
        for script in scripts {
            match quoted::commands(script) {
                Some(commands) => self.walk(
                    &commands,
                    &mut folders,
                    /*level*/ 0,
                    /*code*/ false,
                    &mut found,
                ),
                None => found.unreadable = true,
            }
        }
        if found.unreadable {
            self.note(ProtectedActionKind::UnseenCode);
        }
        self.written.extend(found.written.iter().cloned());
        let mut judged = HashSet::new();
        for (script, cwd) in std::mem::take(&mut found.scripts) {
            if judged.insert((script.clone(), cwd.clone())) {
                self.script(&script, &cwd, depth + 1);
            }
        }
        for sed in std::mem::take(&mut found.seds) {
            self.judge_sed(sed, &found, depth);
        }
        // A sed the word lexer finds that this reading does not (spelled
        // through a variable, braces or `$IFS`) fails closed.
        if lexed_seds > found.sed_commands {
            self.note(ProtectedActionKind::UnseenCode);
        }
    }

    /// Collect nested scripts, sed invocations and written files from
    /// `commands` and the quoted text in them, following `cd`. `code`: the
    /// commands are code an interpreter runs (`python3 -c "..."`), whose
    /// string literals may be shell lines; elsewhere quoted text is data
    /// unless a command runs it.
    fn walk(
        &self,
        commands: &[Command],
        folders: &mut Folders,
        level: usize,
        code: bool,
        found: &mut Found,
    ) {
        for command in commands {
            let texts: Vec<String> = command.words.iter().map(|word| word.text.clone()).collect();
            // A substitution's `cd` does not move this line.
            if !command.substituted
                && let Some((index, name)) = indirect::command_word(&texts)
            {
                self.change_folder(&name, &texts[index + 1..], folders, &mut HashMap::new());
            }
            let first = texts.first().map(|word| name(word));
            let compound = first
                .as_deref()
                .is_none_or(|first| matches!(first, "done" | "fi" | "esac" | "exec"));
            let mut targets: Vec<Option<String>> = command.writes_to.iter().map(known).collect();
            if command.writes_descriptor {
                targets.push(None);
            }
            if compound {
                found.shared_targets.extend(targets.iter().cloned());
            }
            found.written.extend(targets);
            let (starts, overflow) = sed::command_starts(&texts);
            found.unreadable |= overflow;
            let fallback = if code { Role::Code } else { Role::Data };
            let mut roles = vec![fallback; texts.len()];
            let mut input_role = fallback;
            let mut resolved = HashSet::new();
            let mut wrappers_checked = HashSet::new();
            let mut runners = 0;
            let mut sed_command = false;
            for start in starts {
                let Some((index, resolved_name)) = indirect::command_word(&texts[start..]) else {
                    continue;
                };
                let at = start + index;
                // Wrappers that run a string as a shell line (`watch`,
                // `su -c`, `flock f -c`, `env -S`).
                for wrapper in start..=at {
                    if !wrappers_checked.insert(wrapper) {
                        continue;
                    }
                    for (word, script) in runner_scripts(&texts, wrapper) {
                        roles[word] = Role::Script;
                        found.scripts.push((script, folders.cwd.clone()));
                        runners += 1;
                    }
                    // Too many shell strings in one command: fail closed.
                    if runners > outbound::MAX_JUDGED_POSITIONS {
                        found.unreadable = true;
                        break;
                    }
                }
                if !resolved.insert(at) {
                    continue;
                }
                if sed::NAMES.contains(&resolved_name.as_str()) {
                    sed_command = true;
                } else if WRITERS.contains(&resolved_name.as_str()) || !command.writes_to.is_empty()
                {
                    found.other_writes = true;
                    found.written.extend(
                        texts[at + 1..]
                            .iter()
                            .filter(|word| !word.starts_with('-'))
                            .map(|word| Some(file_operand(word).to_string())),
                    );
                }
                if resolved_name == "eval" {
                    for role in &mut roles[at + 1..] {
                        *role = Role::Code;
                    }
                    continue;
                }
                let shell = SHELLS.contains(&resolved_name.as_str());
                match invocation::parse(&resolved_name, &texts[at + 1..])
                    .map(|invocation| invocation.code)
                {
                    Some(Code::Inline(Some(inline))) => {
                        // Attached code (`-c$X`) ends its option word.
                        let Some(first) = (at + 1..texts.len()).find(|&index| {
                            let word = texts[index].as_str();
                            word.ends_with(inline)
                                && std::ptr::eq(&word[word.len() - inline.len()..], inline)
                        }) else {
                            continue;
                        };
                        if !shell {
                            roles[first] = Role::Code;
                            continue;
                        }
                        // After `-c`, bash reads further options (`-o
                        // pipefail`) and runs the first operand; other
                        // shells run the word after `-c`. Every word that
                        // may be the script is judged.
                        let candidates: Vec<usize> = if texts[first].starts_with(['-', '+']) {
                            (first..texts.len())
                                .filter(|&index| !texts[index].starts_with(['-', '+']))
                                .collect()
                        } else {
                            vec![first]
                        };
                        for candidate in candidates {
                            roles[candidate] = Role::Script;
                            found
                                .scripts
                                .push((texts[candidate].clone(), folders.cwd.clone()));
                        }
                    }
                    // A here-string or here-document the shell reads.
                    Some(Code::Stdin) if shell => {
                        input_role = Role::Script;
                        for input in &command.input {
                            found
                                .scripts
                                .push((input.text.clone(), folders.cwd.clone()));
                        }
                    }
                    Some(Code::Stdin) => input_role = Role::Code,
                    _ => {}
                }
            }
            if sed_command {
                self.collect_seds(commands, command, &texts, folders, found);
            }
            if level >= MAX_QUOTED_LEVEL {
                continue;
            }
            let quoted = command
                .words
                .iter()
                .zip(roles)
                .filter(|(word, _)| word.quoted)
                .chain(command.input.iter().map(|input| (input, input_role)));
            for (word, role) in quoted {
                // Quoted text runs in a shell of its own, if at all: its
                // `cd` does not move this line.
                let mut inner = Folders {
                    cwd: folders.cwd.clone(),
                    previous: folders.previous.clone(),
                    stack: folders.stack.clone(),
                };
                self.walk_inner(&word.text, &mut inner, level + 1, role, found);
            }
        }
    }

    fn collect_seds(
        &self,
        commands: &[Command],
        command: &Command,
        texts: &[String],
        folders: &Folders,
        found: &mut Found,
    ) {
        // Where its output goes: its redirections, then every later stage
        // of its pipeline that writes files.
        let mut saved_to: Vec<Option<String>> = command.writes_to.iter().map(known).collect();
        if command.writes_descriptor {
            saved_to.push(None);
        }
        if command.piped && command.pipe_to.is_none() {
            saved_to.push(None);
        }
        let mut stage = command.pipe_to;
        let mut steps = 0;
        while let Some(index) = stage
            && steps < commands.len()
        {
            steps += 1;
            let Some(consumer) = commands.get(index) else {
                saved_to.push(None);
                break;
            };
            saved_to.extend(consumer.writes_to.iter().map(known));
            if consumer.writes_descriptor {
                saved_to.push(None);
            }
            let consumer_texts: Vec<String> = consumer
                .words
                .iter()
                .map(|word| word.text.clone())
                .collect();
            if let Some((at, consumer_name)) = indirect::command_word(&consumer_texts)
                && PIPE_WRITERS.contains(&consumer_name.as_str())
            {
                saved_to.extend(
                    consumer.words[at + 1..]
                        .iter()
                        .filter(|word| !word.text.starts_with('-'))
                        .map(|word| known(word).map(|text| file_operand(&text).to_string())),
                );
            }
            stage = consumer.pipe_to;
        }
        for at in sed::positions(texts) {
            found.sed_commands += 1;
            let fed = texts[..at]
                .iter()
                .any(|word| matches!(name(word).as_str(), "xargs" | "parallel"));
            let replaced = fed
                && (texts[..at].iter().any(|word| {
                    ["-I", "-i", "-J", "--replace"]
                        .iter()
                        .any(|flag| word.starts_with(flag))
                }) || texts[at + 1..].iter().any(|word| word.contains("{}")));
            for call in sed::calls(&command.words[at + 1..]) {
                found.seds.push(Sed {
                    call,
                    cwd: folders.cwd.clone(),
                    saved_to: saved_to.clone(),
                    fed,
                    replaced,
                });
            }
        }
    }

    fn walk_inner(
        &self,
        text: &str,
        folders: &mut Folders,
        level: usize,
        role: Role,
        found: &mut Found,
    ) {
        let Some(commands) = quoted::commands(text) else {
            found.unreadable = true;
            return;
        };
        if role == Role::Code {
            self.walk(&commands, folders, level, /*code*/ true, found);
            return;
        }
        // Data is not run, and a nested shell's script is judged when
        // `script` reads it: here only its sed commands are counted.
        let mut ignored = Found::default();
        self.walk(&commands, folders, level, /*code*/ false, &mut ignored);
        found.sed_commands += ignored.sed_commands;
        found.unreadable |= ignored.unreadable;
    }

    /// A sed that may run a shell command, whose program or options the
    /// classifier cannot read, or that saves what it outputs to a file the
    /// line runs is code the host cannot read; an in-place edit that only
    /// keeps, drops or adds lines has the text it adds judged instead.
    fn judge_sed(&mut self, sed: Sed, line: &Found, depth: usize) {
        let call = &sed.call;
        let dynamic = call
            .programs
            .iter()
            .chain(&call.files)
            .any(|word| word.dynamic);
        // Arguments added at run time may be the program or options.
        let fed = sed.fed
            && !call.bsd
            && (sed.replaced || (call.programs.is_empty() && call.files.is_empty()));
        if call.unknown || fed || (dynamic && self.rebuilt == 0) {
            self.note(ProtectedActionKind::UnseenCode);
            return;
        }
        let mut chunks: Vec<String> = call.programs.iter().map(|word| word.text.clone()).collect();
        for file in &call.files {
            // A script file the action writes may differ when sed runs.
            let rewritten = self.written.iter().any(|written| {
                written
                    .as_deref()
                    .is_none_or(|written| name(written) == name(&file.text))
            });
            let path = self.homes.resolve(&file.text, &sed.cwd);
            match (rewritten, self.read_file(&path)) {
                (false, FileText::Text(text)) => chunks.push(text),
                _ => {
                    self.note(ProtectedActionKind::UnseenCode);
                    return;
                }
            }
        }
        // GNU sed joins the pieces with newlines; each is also read alone.
        let mut readings = vec![sed::program(&chunks.join("\n"))];
        if chunks.len() > 1 {
            readings.extend(chunks.iter().map(|chunk| sed::program(chunk)));
        }
        if readings.iter().any(|reading| reading.runs) {
            self.note(ProtectedActionKind::UnseenCode);
            return;
        }
        let mut targets = sed.saved_to.clone();
        targets.extend(line.shared_targets.iter().cloned());
        targets.extend(
            readings
                .iter()
                .flat_map(|reading| reading.writes.iter().cloned().map(Some)),
        );
        let elsewhere = !targets.is_empty();
        if !elsewhere && !call.in_place {
            return;
        }
        // Another command may copy or move what sed saved.
        if line.other_writes {
            targets.push(None);
        }
        if call.in_place {
            targets.extend(call.operands.iter().map(known));
        }
        if !targets.iter().any(|target| self.may_run(target.as_deref())) {
            return;
        }
        if elsewhere || !readings.iter().all(|reading| reading.keeps_lines) {
            self.note(ProtectedActionKind::UnseenCode);
            return;
        }
        // The edited file holds its own lines and this text: running the
        // file reads the former, the text is judged here.
        let texts: Vec<String> = readings
            .iter()
            .flat_map(|reading| reading.texts.iter().cloned())
            .collect();
        for text in texts {
            self.script(&text, &sed.cwd, depth + 1);
        }
    }

    /// Whether the action runs a script that may be `target` (by name;
    /// `None` is a file the classifier cannot name).
    fn may_run(&self, target: Option<&str>) -> bool {
        if self.ran_unknown {
            return true;
        }
        match target {
            None => !self.ran.is_empty(),
            Some(target) => {
                let target = name(target);
                self.ran.iter().any(|ran| name(ran) == target)
            }
        }
    }
}

/// The file in an operand such as `of=r.sh`.
fn file_operand(word: &str) -> &str {
    word.rsplit_once('=').map_or(word, |(_, file)| file)
}

/// Scripts a wrapper at `at` runs as a shell line, with the word that
/// holds each: `watch CMD...`, `su -c CMD`, `runuser -c`, `flock F -c`,
/// `script -c`, `env -S STRING`. Options are looked for among the next
/// `MAX_JUDGED_POSITIONS` words.
fn runner_scripts(texts: &[String], at: usize) -> Vec<(usize, String)> {
    let wrapper = name(&texts[at]);
    let args = at + 1..texts.len().min(at + 1 + outbound::MAX_JUDGED_POSITIONS);
    match wrapper.as_str() {
        "watch" => {
            // Its options, then the command, joined into one shell line.
            let mut index = at + 1;
            while let Some(word) = texts.get(index)
                && word.starts_with('-')
            {
                let valued = matches!(word.as_str(), "-n" | "--interval" | "-q" | "--equexit");
                index += 1 + usize::from(valued);
            }
            if index >= texts.len() {
                return Vec::new();
            }
            vec![(index, texts[index..].join(" "))]
        }
        "su" | "runuser" | "flock" | "script" | "env" => {
            let (short, long) = if wrapper == "env" {
                ("-S", "--split-string")
            } else {
                ("-c", "--command")
            };
            let mut scripts = Vec::new();
            for index in args {
                let word = &texts[index];
                if word == short || word == long {
                    if let Some(next) = texts.get(index + 1) {
                        scripts.push((index + 1, next.clone()));
                    }
                } else if let Some(attached) = word
                    .strip_prefix(&format!("{long}="))
                    .or_else(|| word.strip_prefix(short).filter(|rest| !rest.is_empty()))
                {
                    scripts.push((index, attached.to_string()));
                }
            }
            scripts
        }
        _ => Vec::new(),
    }
}
