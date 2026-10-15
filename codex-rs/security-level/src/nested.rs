//! Agents started by agent commands while Aggressive is enforced (nested
//! launches).
//!
//! The level always comes from a stored `security_level.toml`, never from the
//! environment. A Corbanu home is the origin of a nested launch when it
//! enforces Aggressive and this process can neither write it nor read its
//! vault store, which is exactly what the Aggressive profile does to agent
//! commands; a person's own launch, and a Permissive sandbox, can read it. Candidate homes are the
//! one named by [`ORIGIN_ENV`] (which Aggressive puts into every agent
//! command's environment), this process's own home, the account's default
//! homes, and every home registered by an Aggressive launch. Only the first
//! two can be changed by a command; the registry sits outside the workspace,
//! where agent commands cannot write.
//!
//! #766 (Travis, 2026-10-14): under Moderate an agent command cannot read the
//! level of the home it protects, so the origin is marked differently. Core
//! denies the agent command [`MODERATE_MARKER`], a file it keeps in that
//! home's runtime folder, and only while Moderate is in force; the runtime
//! folder is read-only to agent commands whenever Core's protected-file rules
//! apply, so they can neither remove nor replace it. A marker this process
//! cannot read is therefore set by the sandbox of an agent command under
//! Moderate, never by the command itself. Homes Moderate ran on are also
//! registered (see [`register_moderate_home`]), so a launch pointed at
//! another home still finds its origin.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use sha2::Digest;
use sha2::Sha256;

use crate::level;
use crate::level::ChosenLevel;
use crate::level::NestedAgents;

/// Set in agent command environments to the home that chose Aggressive.
pub const ORIGIN_ENV: &str = "CORBANU_SECURITY_ORIGIN";

/// #766: in a Corbanu home, the file Core denies to agent commands while
/// Moderate is in force on that home.
pub const MODERATE_MARKER: &str = "tmp/security-moderate-marker";

/// What a `corbanu` subcommand does, for nested-launch decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NestedKind {
    /// Runs an agent without a person (`exec`, `review`); held to Aggressive
    /// when nested launches pass.
    Agent,
    /// An interactive session. Its approval prompts would be answered by the
    /// agent that started it, so it is never allowed nested.
    Interactive,
    /// Serves agents to a client that chooses their sandbox and approvals,
    /// or runs an agent outside Corbanu's sandbox. Never allowed nested.
    Host,
    /// Reads stored credentials. Never allowed nested.
    Credentials,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NestedLaunch {
    NotNested,
    Refuse(String),
    /// Run with Aggressive enforced; the path is the home that chose it.
    EnforceAggressive(PathBuf),
    /// #766: run with Moderate enforced; the path is the home of the
    /// session whose agent command started it.
    EnforceModerate(PathBuf),
}

/// Decide whether `corbanu <name>` may run here (`name` is empty for a new
/// interactive session).
pub fn nested_launch(name: &str, kind: NestedKind) -> NestedLaunch {
    match detect() {
        Ok((origins, moderate)) => decide_with(name, kind, &origins, moderate.as_deref()),
        Err(message) => NestedLaunch::Refuse(message),
    }
}

/// The Aggressive origins and the Moderate origin of this process. With a
/// Moderate origin only Aggressive origins whose level reads as Aggressive
/// count: inside the sandbox of an agent command under Moderate every
/// protected Corbanu home's level is unreadable, which on its own would
/// read as Aggressive.
fn detect() -> Result<(Vec<(PathBuf, NestedAgents)>, Option<PathBuf>), String> {
    let homes = candidate_homes()?;
    let moderate = moderate_origin(&homes);
    let mut origins = nested_origins(homes);
    if moderate.is_some() {
        origins.retain(|(home, _)| level_reads_aggressive(home));
    }
    Ok((origins, moderate))
}

/// Whether `home`'s stored level, or its rule file, can be read here and
/// says Aggressive.
fn level_reads_aggressive(home: &Path) -> bool {
    level::load_state(home).0 == level::StoredLevel::Chosen(ChosenLevel::Aggressive)
        || std::fs::read_to_string(level::rules_path(home))
            .is_ok_and(|contents| level::is_rules_contents(&contents))
}

/// #766: the first of `homes`, or of the homes Moderate registered, whose
/// marker this process cannot read.
pub fn moderate_origin(homes: &[PathBuf]) -> Option<PathBuf> {
    let registered = account_home()
        .ok()
        .flatten()
        .map(|account| registered_homes(&moderate_registry_dir(&account)))
        .unwrap_or_default();
    homes
        .iter()
        .chain(&registered)
        .find(|home| moderate_marker_denied(home))
        .cloned()
}

/// Whether this process may not read `home`'s Moderate marker: Seatbelt
/// refuses even its metadata; bubblewrap masks it with an unreadable file.
/// A missing marker, a link or a folder in its place, or one this process
/// can read is not denied.
pub fn moderate_marker_denied(home: &Path) -> bool {
    let marker = home.join(MODERATE_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Err(err) => err.kind() == io::ErrorKind::PermissionDenied,
        Ok(metadata) if metadata.file_type().is_file() => std::fs::File::open(&marker)
            .is_err_and(|err| err.kind() == io::ErrorKind::PermissionDenied),
        Ok(_) => false,
    }
}

/// Core, before an agent command under Moderate on `codex_home` runs: the
/// marker, created when it is missing. An error when something other than a
/// regular file holds its place.
pub fn prepare_moderate_marker(codex_home: &Path) -> io::Result<PathBuf> {
    let marker = codex_home.join(MODERATE_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_file() => return Ok(marker),
        Ok(_) => {
            return Err(io::Error::other(format!(
                "{} is not a regular file",
                marker.display()
            )));
        }
        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
        Err(_) => {}
    }
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(_) => Ok(marker),
        // Another session made it meanwhile.
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            prepare_moderate_marker(codex_home)
        }
        Err(err) => Err(err),
    }
}

/// Core, before an agent command under Moderate on `codex_home` runs:
/// record the home in `registry` (see [`moderate_registry_dir`]), so a
/// nested launch pointed at another home finds it, and drop entries for
/// homes whose marker is gone.
pub fn register_moderate_home(registry: &Path, codex_home: &Path) -> io::Result<()> {
    registry_entry(registry, codex_home, /*present*/ true)?;
    for entry in std::fs::read_dir(registry).into_iter().flatten().flatten() {
        // Only a marker known to be missing: one this process may not see
        // still marks a home.
        let gone = std::fs::read_to_string(entry.path())
            .map(PathBuf::from)
            .is_ok_and(|home| {
                std::fs::symlink_metadata(home.join(MODERATE_MARKER))
                    .is_err_and(|err| err.kind() == io::ErrorKind::NotFound)
            });
        if gone {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// Where Core registers the homes Moderate runs on for this account.
pub fn moderate_registry_dir(account_home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        account_home.join("Library/Application Support/Corbanu/moderate-homes")
    } else {
        account_home.join(".local/state/corbanu/moderate-homes")
    }
}

/// For a standalone binary (`codex-exec`, `codex-tui`, `codex-app-server`,
/// `codex-mcp-server`): the refusal when an agent command under Aggressive
/// started it. These binaries cannot hold a run to Aggressive, so a launch
/// that `corbanu exec` could run with Aggressive enforced is refused too.
pub fn standalone_nested_refusal(binary: &str, kind: NestedKind) -> Option<String> {
    match detect() {
        Ok((origins, moderate)) => {
            standalone_refusal_with(binary, kind, &origins, moderate.as_deref())
        }
        Err(message) => Some(message),
    }
}

/// [`standalone_nested_refusal`] for known origins.
pub fn standalone_refusal_for(
    binary: &str,
    kind: NestedKind,
    origins: &[(PathBuf, NestedAgents)],
) -> Option<String> {
    standalone_refusal_with(binary, kind, origins, /*moderate*/ None)
}

/// [`standalone_refusal_for`] with the Moderate origin, if any (#766).
pub fn standalone_refusal_with(
    binary: &str,
    kind: NestedKind,
    origins: &[(PathBuf, NestedAgents)],
    moderate: Option<&Path>,
) -> Option<String> {
    match describe(&format!("`{binary}`"), kind, origins, moderate) {
        NestedLaunch::NotNested => None,
        NestedLaunch::Refuse(message) => Some(message),
        NestedLaunch::EnforceAggressive(origin) => Some(format!(
            "`{binary}` was started by an agent command while security level Aggressive is enforced ({}). Only `corbanu exec` and `corbanu review` can run there, with Aggressive enforced.",
            level::state_path(&origin).display()
        )),
        NestedLaunch::EnforceModerate(origin) => Some(format!(
            "`{binary}` was started by an agent command while security level Moderate is enforced ({}). Only `corbanu exec` and `corbanu review` can run there, with Moderate enforced.",
            origin.display()
        )),
    }
}

/// Homes to check: this process's own, the account's default homes and
/// registered origins, and the marker's. Fails closed when the account
/// lookup fails while the marker is set.
fn candidate_homes() -> Result<Vec<PathBuf>, String> {
    let marker = std::env::var_os(ORIGIN_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let mut homes = Vec::new();
    homes.extend(
        codex_utils_home_dir::find_codex_home()
            .ok()
            .map(|home| home.to_path_buf()),
    );
    match account_home() {
        Ok(Some(account_home)) => {
            homes.push(account_home.join(".corbanu"));
            homes.push(account_home.join(".pfterminal"));
            homes.extend(registered_homes(&registry_dir(&account_home)));
        }
        Ok(None) => {}
        // An agent command can make the lookup fail; it is the only
        // launcher that sets the marker.
        Err(err) if marker.is_some() => {
            return Err(format!(
                "Could not check whether security level Aggressive applies to this launch ({err}), so it is refused."
            ));
        }
        Err(_) => {}
    }
    // The marker's home comes last, so a real origin found above is the one
    // the nested run protects.
    homes.extend(marker);
    Ok(homes)
}

/// Every candidate home that is the origin of a nested launch, with its
/// setting.
pub fn nested_origins(homes: Vec<PathBuf>) -> Vec<(PathBuf, NestedAgents)> {
    let mut origins: Vec<(PathBuf, NestedAgents)> = Vec::new();
    for home in homes {
        if origins.iter().any(|(seen, _)| *seen == home) || is_not_a_folder(&home) {
            continue;
        }
        let (stored, nested) = level::load_state(&home);
        // An Aggressive session that saved Permissive stays Aggressive, with
        // its rule file, until it restarts.
        let aggressive = stored.enforced() == ChosenLevel::Aggressive
            || std::fs::read_to_string(level::rules_path(&home))
                .is_ok_and(|contents| level::is_rules_contents(&contents));
        if aggressive && sandboxed_away_from(&home) {
            let nested = match stored {
                level::StoredLevel::Chosen(ChosenLevel::Aggressive) => nested,
                _ => NestedAgents::Refuse,
            };
            origins.push((home, nested));
        }
    }
    origins
}

/// Whether `home` is known to be something other than a folder, so it holds
/// no level. On Linux the sandbox masks a denied home that does not exist
/// (`~/.pfterminal`, say) by mounting an empty file there, which bubblewrap
/// first creates on the real file system; while that command runs every
/// process sees the file. Its unreadable "level" and failing probes must not
/// make every launch on the account look nested. A home whose metadata cannot
/// be read stays a candidate: that is what a sandbox denial looks like (a
/// denial never reports "not a directory", which a placeholder parent does).
fn is_not_a_folder(home: &Path) -> bool {
    match std::fs::metadata(home) {
        Ok(metadata) => !metadata.is_dir(),
        Err(err) => err.kind() == io::ErrorKind::NotADirectory,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    Allowed,
    Denied,
    /// Another error, such as no free file descriptors, which a command can
    /// cause. Counted as denied.
    Unknown,
}

fn probe<T>(result: io::Result<T>) -> Probe {
    match result {
        Ok(_) => Probe::Allowed,
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            Probe::Denied
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Probe::Allowed,
        Err(_) => Probe::Unknown,
    }
}

/// Whether this process can neither write `home` nor read its vault store,
/// as inside an Aggressive agent command. A person's own launch and a
/// Permissive sandbox can read the store; Aggressive launches create its
/// folder so the denial is observable.
fn sandboxed_away_from(home: &Path) -> bool {
    probe(tempfile::NamedTempFile::new_in(home)) != Probe::Allowed
        && probe(std::fs::read_dir(home.join("secrets"))) != Probe::Allowed
}

/// Refuse wins when several origins apply.
pub fn decide(name: &str, kind: NestedKind, origins: &[(PathBuf, NestedAgents)]) -> NestedLaunch {
    decide_with(name, kind, origins, /*moderate*/ None)
}

/// [`decide`] with the Moderate origin, if any (#766). An Aggressive origin
/// wins over it: Aggressive applies wherever it is enforced further out.
pub fn decide_with(
    name: &str,
    kind: NestedKind,
    origins: &[(PathBuf, NestedAgents)],
    moderate: Option<&Path>,
) -> NestedLaunch {
    let command = if name.is_empty() {
        "`corbanu`".to_string()
    } else {
        format!("`corbanu {name}`")
    };
    describe(&command, kind, origins, moderate)
}

fn describe(
    command: &str,
    kind: NestedKind,
    origins: &[(PathBuf, NestedAgents)],
    moderate: Option<&Path>,
) -> NestedLaunch {
    let Some((origin, nested)) = origins
        .iter()
        .find(|(_, nested)| *nested == NestedAgents::Refuse)
        .or_else(|| origins.first())
    else {
        return match moderate {
            Some(origin) => describe_moderate(command, kind, origin),
            None => NestedLaunch::NotNested,
        };
    };
    let started = format!(
        "{command} was started by an agent command while security level Aggressive is enforced ({})",
        level::state_path(origin).display()
    );
    match (kind, nested) {
        (NestedKind::Agent, NestedAgents::Pass) => NestedLaunch::EnforceAggressive(origin.clone()),
        (NestedKind::Agent, NestedAgents::Refuse) => NestedLaunch::Refuse(format!(
            "{started}. Nested agent launches are set to refuse; a person can let `corbanu exec` and `corbanu review` run with Aggressive enforced in /security."
        )),
        (NestedKind::Interactive, _) => NestedLaunch::Refuse(format!(
            "{started}. Interactive sessions are never allowed there: the agent that started one would answer its approval prompts."
        )),
        (NestedKind::Host, _) => NestedLaunch::Refuse(format!(
            "{started}. It is never allowed there: its client chooses each agent's sandbox and approvals, or the agent runs outside Corbanu's sandbox, so Aggressive cannot be enforced."
        )),
        (NestedKind::Credentials, _) => NestedLaunch::Refuse(format!(
            "{started}. It reads stored credentials, which Aggressive keeps from agent commands."
        )),
    }
}

/// #766 (D5): `corbanu exec` and `review` run with Moderate enforced; what
/// cannot be held to it is refused, as under Aggressive.
fn describe_moderate(command: &str, kind: NestedKind, origin: &Path) -> NestedLaunch {
    let started = format!(
        "{command} was started by an agent command while security level Moderate is enforced ({})",
        origin.display()
    );
    match kind {
        NestedKind::Agent => NestedLaunch::EnforceModerate(origin.to_path_buf()),
        NestedKind::Interactive => NestedLaunch::Refuse(format!(
            "{started}. Interactive sessions are never allowed there: the agent that started one would answer its approval prompts. `corbanu exec` and `corbanu review` run with Moderate enforced."
        )),
        NestedKind::Host => NestedLaunch::Refuse(format!(
            "{started}. It is never allowed there: its client chooses each agent's sandbox and approvals, or the agent runs outside Corbanu's sandbox, so Moderate cannot be enforced. `corbanu exec` and `corbanu review` run with Moderate enforced."
        )),
        NestedKind::Credentials => NestedLaunch::Refuse(format!(
            "{started}. It reads stored credentials, which Moderate keeps from agent commands."
        )),
    }
}

/// Record or forget `codex_home` as an Aggressive origin, so nested launches
/// that point `CODEX_HOME` elsewhere still find it, and drop entries for
/// homes that no longer store a level.
pub fn register_origin(codex_home: &Path, aggressive: bool) -> io::Result<()> {
    let Some(account_home) = account_home()? else {
        return Ok(());
    };
    let registry = registry_dir(&account_home);
    registry_entry(&registry, codex_home, aggressive)?;
    for entry in std::fs::read_dir(&registry).into_iter().flatten().flatten() {
        let stale = std::fs::read_to_string(entry.path())
            .map(PathBuf::from)
            .is_ok_and(|home| !level::state_path(&home).exists());
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// Record (`present`) or forget `codex_home` in `registry`.
pub fn registry_entry(registry: &Path, codex_home: &Path, present: bool) -> io::Result<()> {
    let digest = Sha256::digest(codex_home.as_os_str().as_encoded_bytes());
    let name = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let entry = registry.join(name);
    if !present {
        return match std::fs::remove_file(&entry) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        };
    }
    let path = codex_home
        .to_str()
        .ok_or_else(|| io::Error::other("Corbanu home path is not UTF-8"))?;
    if std::fs::read_to_string(&entry).ok().as_deref() == Some(path) {
        return Ok(());
    }
    std::fs::create_dir_all(registry)?;
    std::fs::write(entry, path)
}

pub fn registered_homes(registry: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(registry)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .collect()
}

/// Where Aggressive launches register their home for this account.
pub fn registry_dir(account_home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        account_home.join("Library/Application Support/Corbanu/aggressive-homes")
    } else {
        account_home.join(".local/state/corbanu/aggressive-homes")
    }
}

/// The account's home directory from the account database, not `$HOME`,
/// which an agent command can change. `Ok(None)` when the account has no
/// entry. Debug builds honour `CORBANU_TEST_ACCOUNT_HOME` so tests never read
/// the operator's profile.
pub fn account_home() -> io::Result<Option<PathBuf>> {
    if cfg!(debug_assertions)
        && let Some(home) = std::env::var_os("CORBANU_TEST_ACCOUNT_HOME")
    {
        return Ok(Some(PathBuf::from(home)));
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let mut size = 16 * 1024;
        loop {
            let mut buffer = vec![0 as libc::c_char; size];
            // SAFETY: `passwd` is plain data that getpwuid_r overwrites.
            let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
            let mut result = std::ptr::null_mut();
            // SAFETY: every pointer is valid for the call and `buffer.len()`
            // is its real size.
            let status = unsafe {
                libc::getpwuid_r(
                    libc::getuid(),
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut result,
                )
            };
            if status == libc::ERANGE && size < 4 * 1024 * 1024 {
                size *= 4;
                continue;
            }
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status));
            }
            if result.is_null() || entry.pw_dir.is_null() {
                return Ok(None);
            }
            // SAFETY: getpwuid_r succeeded, so `pw_dir` is a NUL-terminated
            // string inside `buffer`, which is still alive.
            let dir = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) };
            let home = PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()));
            return Ok(home.is_absolute().then_some(home));
        }
    }
    #[cfg(not(unix))]
    {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "nested_tests.rs"]
mod tests;
