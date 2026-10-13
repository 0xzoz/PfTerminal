//! #623 (milestone "Aggressive ships" re-run, AGG-R13): under Aggressive an
//! agent command may not create a link to the project's `.codex` or
//! `.agents` (or into Corbanu's own read-only files): they are always
//! read-only (#520), and a link to them is not created either (Travis,
//! 2026-10-13). Writes through such a link were already refused; now the
//! command is refused before it runs, so nothing is created, and the person
//! is told why.
//!
//! No operating-system sandbox can refuse a symbolic link by where it points
//! (Seatbelt only filters by link name, Landlock and seccomp cannot read the
//! target), so the command text decides, before anything runs:
//! - `ln` (`-s` or hard), `cp -s`/`-l`, `mklink` and PowerShell's `New-Item
//!   -ItemType SymbolicLink|Junction|HardLink`, also inside `sh -c` scripts;
//!   a relative target is taken from the link's folder and from the command's;
//! - code an interpreter runs (`python3 -c`, `node -e`, `perl -e`, `ruby -e`,
//!   or a heredoc in a script) that calls a link function (`os.symlink`,
//!   `symlink_to`, `fs.symlinkSync`, `File.symlink`, `link`…) and names such a
//!   path.
//!
//! A target built at run time, or a link made by a script file, is out of
//! reach here; writes through it stay refused by the sandbox. Hard links to a
//! protected file are refused by the sandbox itself as well.

use std::path::Path;
use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;

use super::protected_reads::shown_path;
use super::protected_surface::ReadDenials;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

/// Shells whose `-c` argument is a script.
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];
/// Interpreters whose `-c`/`-e` argument (or heredoc) is code.
const INTERPRETERS: [&str; 6] = ["python", "python3", "node", "perl", "ruby", "deno"];
/// Words in code that create a link.
const LINK_CALLS: [&str; 4] = ["symlink", "link(", "link_to", "linkSync"];
/// Scripts inside scripts looked into.
const MAX_DEPTH: usize = 4;
/// Paths looked at in one command.
const MAX_CANDIDATES: usize = 64;

/// What a person is told when such a command is refused.
pub(crate) fn explanation(target: &str) -> String {
    format!(
        "Security level Aggressive refused this command before it ran: it creates a link to \
         `{}`, and the project's .codex and .agents folders (agent settings) and Corbanu's own \
         files are always read-only under Aggressive, so no link to them is created either \
         (nothing was created). This cannot be granted, not even once. To allow it, a person can \
         switch to Permissive in /security (it applies after a restart).",
        shown_path(target)
    )
}

/// The refusal for `command` (run in `cwd`) under Aggressive, if it creates
/// a link to a path no grant lifts; also shown to the person and listed
/// under the inspector's recent denials.
pub(crate) async fn refusal(
    session: &Session,
    turn: &TurnContext,
    command: &[String],
    cwd: &Path,
) -> Option<String> {
    let candidates = link_targets(command);
    if candidates.is_empty() {
        return None;
    }
    let inputs = Inputs::capture(session, turn)?;
    let cwd = cwd.to_path_buf();
    let found = tokio::time::timeout(
        LOOKUP_TIMEOUT,
        tokio::task::spawn_blocking(move || inputs.protected_target(&candidates, &cwd)),
    )
    .await
    .ok()?
    .ok()??;
    let refusal = explanation(&found);
    super::protected_reads::notify(session, turn, refusal.clone()).await;
    super::inspection::record_refusal(
        session.thread_id(),
        "creating a link to the project's .codex or .agents, or Corbanu's files",
        "refused before it ran; never granted under Aggressive",
    );
    Some(refusal)
}

/// How long the path lookups may take; past it nothing is refused (writes
/// through a link stay refused by the sandbox).
const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// What the protected paths are built from (see `protected_reads`).
struct Inputs {
    codex_home: PathBuf,
    turn_cwd: AbsolutePathBuf,
    workspace_roots: Vec<AbsolutePathBuf>,
    profile: codex_protocol::models::PermissionProfile,
    user_home: Option<PathBuf>,
}

impl Inputs {
    /// Only under Aggressive, for a local command.
    fn capture(session: &Session, turn: &TurnContext) -> Option<Self> {
        session
            .services
            .model_client()
            .post_taint_state()
            .filter(|state| state.level == codex_security_policy::SecurityLevel::Aggressive)?;
        if turn
            .environments
            .primary()
            .is_some_and(|environment| environment.environment.is_remote())
        {
            return None;
        }
        #[allow(deprecated)]
        let turn_cwd = turn.cwd.clone();
        let workspace_roots = turn
            .environments
            .primary()
            .map(|environment| {
                environment
                    .workspace_roots()
                    .iter()
                    .filter_map(|root| root.to_abs_path().ok())
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            codex_home: turn.config.codex_home.to_path_buf(),
            turn_cwd,
            workspace_roots,
            profile: turn.config.permissions.permission_profile().clone(),
            user_home: dirs::home_dir(),
        })
    }

    /// The first candidate, as spelled, that lands in a path no grant lifts.
    fn protected_target(&self, candidates: &[Candidate], cwd: &Path) -> Option<String> {
        let denials = ReadDenials::for_turn(
            &self.codex_home,
            &self.turn_cwd,
            &self.workspace_roots,
            &self.profile,
        );
        let mut kept: Vec<AbsolutePathBuf> = denials.never_lifted_paths().cloned().collect();
        let real: Vec<AbsolutePathBuf> =
            kept.iter().filter_map(ReadDenials::real_location).collect();
        kept.extend(real);
        let writable: Vec<&AbsolutePathBuf> = denials.home_writable_paths().collect();
        candidates.iter().find_map(|candidate| {
            candidate
                .bases(cwd)
                .into_iter()
                .filter_map(|base| self.resolve(&candidate.target, &base))
                .any(|path| lands_in(&path, &kept, &writable))
                .then(|| candidate.target.clone())
        })
    }

    fn resolve(&self, word: &str, base: &Path) -> Option<AbsolutePathBuf> {
        let codex_home = self.codex_home.to_string_lossy();
        let mut expanded = word.to_string();
        for name in ["CODEX_HOME", "CORBANU_HOME", "PFTERMINAL_HOME"] {
            for spelling in [format!("${{{name}}}"), format!("${name}")] {
                expanded = expanded.replace(&spelling, &codex_home);
            }
        }
        if let Some(home) = self.user_home.as_deref().map(Path::to_string_lossy) {
            for spelling in ["${HOME}", "$HOME"] {
                expanded = expanded.replace(spelling, &home);
            }
            if expanded == "~" {
                expanded = home.into_owned();
            } else if let Some(rest) = expanded.strip_prefix("~/") {
                expanded = format!("{home}/{rest}");
            }
        }
        if expanded.is_empty() || expanded.contains('$') || expanded.starts_with('~') {
            return None;
        }
        Some(AbsolutePathBuf::resolve_path_against_base(expanded, base))
    }
}

/// Whether `path`, as spelled, through a link on the way or where it
/// finally points, is in one of `kept` and not in a folder of `writable`
/// inside it (`worktrees` in the Corbanu home, #583).
fn lands_in(
    path: &AbsolutePathBuf,
    kept: &[AbsolutePathBuf],
    writable: &[&AbsolutePathBuf],
) -> bool {
    let mut spellings = vec![path.clone()];
    spellings.extend(ReadDenials::real_location(path));
    spellings.iter().any(|spelling| {
        kept.iter().any(|kept| {
            spelling.as_path().starts_with(kept.as_path())
                && !writable.iter().any(|folder| {
                    spelling.as_path().starts_with(folder.as_path())
                        && folder.as_path().starts_with(kept.as_path())
                })
        })
    })
}

/// A path a command links to, as spelled, and where a relative one is read
/// from besides the command's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) target: String,
    /// Folders the link is made in (a relative target is relative to it).
    pub(crate) link_folders: Vec<String>,
}

impl Candidate {
    fn bases(&self, cwd: &Path) -> Vec<PathBuf> {
        let mut bases = vec![cwd.to_path_buf()];
        for folder in &self.link_folders {
            let folder = cwd.join(folder);
            if !bases.contains(&folder) {
                bases.push(folder);
            }
        }
        bases
    }
}

/// The paths `command` creates links to (see the module docs), bounded.
pub(crate) fn link_targets(command: &[String]) -> Vec<Candidate> {
    let mut found = Vec::new();
    simple_command(command, MAX_DEPTH, &mut found);
    found.truncate(MAX_CANDIDATES);
    found
}

fn program_name(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.to_ascii_lowercase();
    let name = name
        .strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or(name);
    // `python3.12` reads as `python3`.
    match name.split_once('.') {
        Some((stem, version))
            if stem.starts_with("python") && version.chars().all(|c| c.is_ascii_digit()) =>
        {
            stem.to_string()
        }
        _ => name,
    }
}

/// One simple command (already split into words).
fn simple_command(words: &[String], depth: usize, found: &mut Vec<Candidate>) {
    let words: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .skip_while(|word| {
            (word.contains('=') && !word.starts_with('-'))
                || matches!(
                    *word,
                    "sudo" | "command" | "env" | "exec" | "nohup" | "time" | "builtin"
                )
        })
        .collect();
    let Some((program, arguments)) = words.split_first() else {
        return;
    };
    let name = program_name(program);
    let name = name.as_str();
    if SHELLS.contains(&name) {
        let script = arguments
            .iter()
            .skip_while(|argument| {
                !(argument.starts_with('-')
                    && !argument.starts_with("--")
                    && argument.contains('c'))
            })
            .nth(1);
        if let Some(script) = script {
            shell_script(script, depth, found);
        }
        return;
    }
    if matches!(name, "cmd") {
        // `cmd /c mklink …`
        if let Some(index) = arguments
            .iter()
            .position(|word| word.eq_ignore_ascii_case("/c") || word.eq_ignore_ascii_case("/k"))
        {
            let rest: Vec<String> = arguments[index + 1..]
                .iter()
                .flat_map(|word| word.split_whitespace())
                .map(str::to_string)
                .collect();
            simple_command(&rest, depth, found);
        }
        return;
    }
    if matches!(name, "powershell" | "pwsh") {
        if let Some(index) = arguments.iter().position(|word| {
            word.eq_ignore_ascii_case("-command") || word.eq_ignore_ascii_case("-c")
        }) {
            let script = arguments[index + 1..].join(" ");
            for statement in script.split([';', '\n', '|']) {
                let words: Vec<String> = shlex::split(statement)
                    .unwrap_or_else(|| statement.split_whitespace().map(str::to_string).collect());
                simple_command(&words, depth, found);
            }
        }
        return;
    }
    match name {
        "ln" => ln(arguments, found),
        "cp" if arguments.iter().any(|word| {
            matches!(*word, "--symbolic-link" | "--link")
                || (word.starts_with('-')
                    && !word.starts_with("--")
                    && (word.contains('s') || word.contains('l')))
        }) =>
        {
            let paths = paths_of(arguments, &["-t", "--target-directory", "-S", "--suffix"]);
            if paths.len() >= 2 {
                let link_folder = paths[paths.len() - 1].to_string();
                for target in &paths[..paths.len() - 1] {
                    push(found, target, vec![link_folder.clone()]);
                }
            }
        }
        "mklink" => {
            // `mklink [/D|/H|/J] link target`
            let paths: Vec<&&str> = arguments
                .iter()
                .filter(|word| !word.starts_with('/'))
                .collect();
            if let [link, target] = paths.as_slice() {
                push(found, target, vec![parent_of(link)]);
            }
        }
        "new-item" | "ni" => new_item(arguments, found),
        _ if INTERPRETERS.contains(&name) => {
            // Inline code (`-c`, `-e`, `--eval`), or every argument (heredoc
            // scripts arrive through the shell script instead).
            for word in arguments {
                code(word, found);
            }
        }
        _ => {}
    }
}

/// `ln [options] target… [link | folder]`, `ln -t folder target…`.
fn ln(arguments: &[&str], found: &mut Vec<Candidate>) {
    let mut folder = None;
    let mut iter = arguments.iter();
    while let Some(word) = iter.next() {
        if *word == "-t" || *word == "--target-directory" {
            folder = iter.next().map(|folder| (*folder).to_string());
        } else if let Some(value) = word.strip_prefix("--target-directory=") {
            folder = Some(value.to_string());
        }
    }
    let paths = paths_of(arguments, &["-t", "--target-directory", "-S", "--suffix"]);
    let (targets, link_folders): (&[&str], Vec<String>) = match (&folder, paths.as_slice()) {
        (Some(folder), paths) => (paths, vec![folder.clone()]),
        (None, [target]) => (std::slice::from_ref(target), Vec::new()),
        (None, [targets @ .., last]) => (targets, vec![parent_of(last), (*last).to_string()]),
        (None, []) => (&[], Vec::new()),
    };
    for target in targets {
        push(found, target, link_folders.clone());
    }
}

/// PowerShell `New-Item -ItemType SymbolicLink|Junction|HardLink -Path link
/// -Target target` (or `-Value`).
fn new_item(arguments: &[&str], found: &mut Vec<Candidate>) {
    let value_of = |names: &[&str]| {
        arguments
            .iter()
            .position(|word| names.iter().any(|name| word.eq_ignore_ascii_case(name)))
            .and_then(|index| arguments.get(index + 1))
            .map(|value| value.trim_matches(['\'', '"']).to_string())
    };
    let is_link = value_of(&["-ItemType", "-Type"]).is_some_and(|kind| {
        ["symboliclink", "junction", "hardlink"].contains(&kind.to_ascii_lowercase().as_str())
    });
    if !is_link {
        return;
    }
    if let Some(target) = value_of(&["-Target", "-Value"]) {
        let link_folders = value_of(&["-Path", "-Name"])
            .map(|link| vec![parent_of(&link)])
            .unwrap_or_default();
        push(found, &target, link_folders);
    }
}

/// The path arguments: every word that is not an option or an option's
/// value (`takes_value`).
fn paths_of<'a>(arguments: &[&'a str], takes_value: &[&str]) -> Vec<&'a str> {
    let mut paths = Vec::new();
    let mut skip = false;
    let mut options_done = false;
    for word in arguments {
        if skip {
            skip = false;
        } else if !options_done && *word == "--" {
            options_done = true;
        } else if !options_done && takes_value.contains(word) {
            skip = true;
        } else if options_done || !word.starts_with('-') || *word == "-" {
            paths.push(*word);
        }
    }
    paths
}

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn push(found: &mut Vec<Candidate>, target: &str, link_folders: Vec<String>) {
    let target = target.trim_matches(['\'', '"']).to_string();
    let candidate = Candidate {
        target,
        link_folders,
    };
    if !candidate.target.is_empty() && !found.contains(&candidate) {
        found.push(candidate);
    }
}

/// A shell script: each simple command in it, and, when it runs an
/// interpreter on several lines (a heredoc) that call a link function, every
/// string it quotes.
fn shell_script(script: &str, depth: usize, found: &mut Vec<Candidate>) {
    let Some(depth) = depth.checked_sub(1) else {
        return;
    };
    let mut segment: Vec<String> = Vec::new();
    let mut runs_interpreter = false;
    let mut finish = |segment: &mut Vec<String>, found: &mut Vec<Candidate>| {
        if segment
            .first()
            .is_some_and(|program| INTERPRETERS.contains(&program_name(program).as_str()))
        {
            runs_interpreter = true;
        }
        simple_command(&std::mem::take(segment), depth, found);
    };
    // A line ends a command (a quoted string over several lines is split
    // too, which only cuts that string into words), and so do `;`, `&&`,
    // `||`, `|` and `&` between words (not inside quoted code).
    for line in script.lines() {
        let words = shlex::split(line)
            .unwrap_or_else(|| line.split_whitespace().map(str::to_string).collect());
        for word in words {
            if matches!(word.as_str(), "&&" | "||" | "|" | "&" | ";") {
                finish(&mut segment, found);
            } else if let Some(word) = word.strip_suffix(';') {
                if !word.is_empty() {
                    segment.push(word.to_string());
                }
                finish(&mut segment, found);
            } else {
                segment.push(word);
            }
        }
        finish(&mut segment, found);
    }
    finish(&mut segment, found);
    // Code fed through a heredoc (`python3 - <<'EOF'`), or over several
    // lines (cut into words above).
    if runs_interpreter && (script.contains("<<") || script.contains('\n')) {
        code(script, found);
    }
}

/// Code an interpreter runs: when it calls a link function, every quoted
/// string in it is a candidate target.
fn code(code: &str, found: &mut Vec<Candidate>) {
    if !LINK_CALLS.iter().any(|call| code.contains(call)) {
        return;
    }
    for quote in ['\'', '"', '`'] {
        for literal in code.split(quote).skip(1).step_by(2) {
            if !literal.is_empty() && !literal.contains('\n') && literal.len() < 4096 {
                push(found, literal, Vec::new());
            }
        }
    }
}

#[cfg(test)]
#[path = "protected_links_tests.rs"]
mod tests;
