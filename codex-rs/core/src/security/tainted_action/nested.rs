//! #885: what the word lexer cannot see because it removes quotes. A nested
//! shell's script (`bash -c "sh -c './x.sh'"`) is judged like a command
//! line of its own, and a sed program is judged whole: one that may run a
//! shell command, or that saves what it outputs to a file the same line
//! runs, is code the host cannot read.

use super::Classifier;
use super::FileText;
use super::Folders;
use super::ProtectedActionKind;
use super::indirect;
use super::invocation;
use super::invocation::Code;
use super::quoted;
use super::quoted::Command;
use super::quoted::Word;
use super::sed;
use super::shell::SHELLS;
use super::shell::basename;
use std::collections::HashMap;

/// Levels of quoted text read for nested commands. Deeper text is still
/// split by the word lexer, and a sed only it sees fails closed.
const MAX_QUOTED_LEVEL: usize = 4;

/// What the pass found in one command line.
#[derive(Default)]
struct Found {
    /// Scripts that nested shells run, with the folder they run in.
    scripts: Vec<(String, String)>,
    seds: Vec<Sed>,
    /// sed commands found (each may be read more than one way).
    sed_commands: usize,
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

/// One reading of a sed invocation, the folder it runs in and where its
/// output goes.
struct Sed {
    call: sed::Call,
    cwd: String,
    writes_to: Vec<Word>,
    piped: bool,
}

fn name(word: &str) -> String {
    basename(&word.to_lowercase()).to_string()
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
        let mut lines = Vec::new();
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
                lines.extend(quoted::commands(arg));
            } else {
                argv.words.push(Word {
                    text: arg.clone(),
                    quoted: true,
                    ..Word::default()
                });
            }
            previous = Some(arg);
        }
        lines.insert(0, argv);
        let mut folders = Folders {
            cwd: cwd.to_string(),
            previous: cwd.to_string(),
            stack: Vec::new(),
        };
        let mut found = Found::default();
        self.walk(
            &lines,
            &mut folders,
            /*level*/ 0,
            /*code*/ false,
            &mut found,
        );
        let seen = found.sed_commands;
        for (script, cwd) in found.scripts {
            self.script(&script, &cwd, depth + 1);
        }
        for sed in found.seds {
            self.judge_sed(sed, depth);
        }
        // A sed the word lexer finds that this reading does not (spelled
        // through a variable, braces or `$IFS`) fails closed.
        if lexed_seds > seen {
            self.note(ProtectedActionKind::UnseenCode);
        }
    }

    /// Collect nested scripts and sed invocations from `commands` and the
    /// quoted text in them, following `cd`. `code`: the commands are code an
    /// interpreter runs (`python3 -c "..."`), whose string literals may be
    /// shell lines; elsewhere quoted text is data unless a command runs it.
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
            let mut roles = vec![if code { Role::Code } else { Role::Data }; texts.len()];
            let mut input_role = if code { Role::Code } else { Role::Data };
            for start in sed::command_starts(&texts) {
                let Some((index, name)) = indirect::command_word(&texts[start..]) else {
                    continue;
                };
                let at = start + index;
                let shell = SHELLS.contains(&name.as_str());
                if name == "eval" {
                    for role in &mut roles[at + 1..] {
                        *role = Role::Code;
                    }
                    continue;
                }
                match invocation::parse(&name, &texts[at + 1..]).map(|invocation| invocation.code) {
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
                        let mut candidates = vec![first];
                        // bash runs the first operand after its options
                        // (`bash -c -e '...'`); other shells the word after `-c`.
                        if texts[first].starts_with(['-', '+']) {
                            let operand = (first..texts.len())
                                .find(|&index| !texts[index].starts_with(['-', '+']));
                            let after_dashes = (first..texts.len())
                                .find(|&index| texts[index] == "--")
                                .map(|index| index + 1);
                            candidates.extend(operand);
                            candidates.extend(after_dashes.filter(|index| *index < texts.len()));
                        }
                        for candidate in candidates {
                            roles[candidate] = Role::Script;
                            found
                                .scripts
                                .push((texts[candidate].clone(), folders.cwd.clone()));
                        }
                    }
                    // A here-string or here-document the command reads as code.
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
            for at in sed::positions(&texts) {
                found.sed_commands += 1;
                for call in sed::calls(&command.words[at + 1..]) {
                    found.seds.push(Sed {
                        call,
                        cwd: folders.cwd.clone(),
                        writes_to: command.writes_to.clone(),
                        piped: command.piped,
                    });
                }
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

    fn walk_inner(
        &self,
        text: &str,
        folders: &mut Folders,
        level: usize,
        role: Role,
        found: &mut Found,
    ) {
        let commands = quoted::commands(text);
        if role == Role::Code {
            self.walk(&commands, folders, level, /*code*/ true, found);
            return;
        }
        // Data, or a nested shell's script (found when `script` reads it):
        // only its sed commands count here.
        let mut ignored = Found::default();
        self.walk(&commands, folders, level, /*code*/ false, &mut ignored);
        found.sed_commands += ignored.sed_commands;
        found.seds.extend(ignored.seds);
    }

    /// A sed that may run a shell command, whose program or options the
    /// classifier cannot read, or that saves what it outputs to a file the
    /// line runs is code the host cannot read; an in-place edit that only
    /// keeps, drops or adds lines has the text it adds judged instead.
    fn judge_sed(&mut self, sed: Sed, depth: usize) {
        let call = &sed.call;
        let dynamic = call
            .programs
            .iter()
            .chain(&call.files)
            .any(|word| word.dynamic);
        if call.unknown || (dynamic && self.rebuilt == 0) {
            self.note(ProtectedActionKind::UnseenCode);
            return;
        }
        let mut chunks: Vec<String> = call.programs.iter().map(|word| word.text.clone()).collect();
        for file in &call.files {
            let path = self.homes.resolve(&file.text, &sed.cwd);
            match self.read_file(&path) {
                FileText::Text(text) => chunks.push(text),
                FileText::Missing | FileText::Unreadable | FileText::Binary => {
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
        let writes: Vec<&String> = readings
            .iter()
            .flat_map(|reading| &reading.writes)
            .collect();
        let elsewhere = !sed.writes_to.is_empty() || sed.piped || !writes.is_empty();
        if !elsewhere && !call.in_place {
            return;
        }
        let known = |word: &Word| (!word.dynamic).then(|| word.text.clone());
        let mut targets: Vec<Option<String>> = Vec::new();
        if call.in_place {
            targets.extend(call.operands.iter().map(known));
        }
        targets.extend(sed.writes_to.iter().map(known));
        targets.extend(writes.iter().map(|file| Some((*file).clone())));
        if sed.piped {
            targets.push(None);
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
