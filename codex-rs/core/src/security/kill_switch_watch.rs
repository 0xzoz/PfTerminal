//! #507 (Travis, 2026-10-12): the kill switch reaches every Corbanu Terminal
//! process running on the same Corbanu home, not only the one it was turned
//! on in, and not only at their next start.
//!
//! Each process watches its homes' `security_state.json` (one thread per
//! home, while a policy tree of this process or a subscriber uses it). The
//! file is written atomically by every confirmed transition, so a change of
//! it is seen by a file notification (`notify`: FSEvents, inotify,
//! ReadDirectoryChangesW) at once, and at the latest by the next poll
//! ([`POLL`]), when notifications are lost or unavailable. The file is only
//! read when it changed; nothing spins.
//!
//! When the stored kill-switch event differs from the one a policy tree of
//! this process holds, the stored revocations merge into it (the later
//! kill-switch event wins, as between trees of one process): turned on, the
//! tree's sinks stop its running work (#522, `kill_switch_stop`);
//! turned off, its turns are allowed again. Nothing else is taken from the
//! file here: a level saved by another process still applies at the next
//! session start. An unreadable file changes nothing while running (a
//! transient read error must not flip the switch); the next start enforces
//! it as before.
//!
//! Latency: within [`POLL`] (1 s) of the save, usually at once, plus the
//! stop itself; the user-facing text says "within about a second".

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::mpsc;
use std::time::Duration;

use notify::RecursiveMode;
use notify::Watcher;

use super::SharedEffectivePolicy;
use super::TREES;
use super::adopt;
use super::canonical;
use super::notify as notify_sinks;
use crate::security::recovery::STATE_FILE;

/// The longest a change of the saved state goes unseen when no file
/// notification arrives.
pub(crate) const POLL: Duration = Duration::from_secs(1);

/// The watched homes of this process, by canonical path.
static WATCHES: LazyLock<Mutex<HashMap<PathBuf, Arc<HomeWatch>>>> = LazyLock::new(Default::default);

struct HomeWatch {
    /// Whether the kill switch is on for the home, for subscribers (the TUI
    /// stops and resumes Claude panes with it).
    on: tokio::sync::watch::Sender<bool>,
}

/// Start watching `home` unless this process already does.
pub(super) fn ensure(home: &Path) {
    // Unit tests build many trees on temporary homes and model "another
    // process" by writing the file; they start a watch explicitly.
    #[cfg(not(test))]
    start(home);
    #[cfg(test)]
    let _ = home;
}

/// Watch `home` (once per process) and return the watch.
fn start(home: &Path) -> Arc<HomeWatch> {
    let home = canonical(home);
    let mut watches = WATCHES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(watch) = watches.get(&home) {
        return Arc::clone(watch);
    }
    let (on, _) =
        tokio::sync::watch::channel(crate::security::revocation_change::kill_switch_on(&home));
    let watch = Arc::new(HomeWatch { on });
    watches.insert(home.clone(), Arc::clone(&watch));
    let thread_watch = Arc::clone(&watch);
    let spawned = std::thread::Builder::new()
        .name("corbanu-kill-switch-watch".to_string())
        .spawn(move || run(&home, &thread_watch));
    if let Err(error) = spawned {
        tracing::warn!(
            target: "codex_core::security::transition",
            %error,
            "cannot watch the security state for a kill switch from another process"
        );
    }
    watch
}

/// #507: whether the kill switch is on for `home`, updated while this
/// process runs, also when another process on the home changes it.
pub(crate) fn subscribe(home: &Path) -> tokio::sync::watch::Receiver<bool> {
    start(home).on.subscribe()
}

fn run(home: &Path, watch: &HomeWatch) {
    let (tx, rx) = mpsc::channel::<()>();
    // The file is replaced by a rename, so its folder is watched.
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let relevant = match &event {
            Ok(event) => {
                event.need_rescan()
                    || event
                        .paths
                        .iter()
                        .any(|path| path.file_name().is_some_and(|name| name == STATE_FILE))
            }
            Err(_) => true,
        };
        if relevant {
            let _ = tx.send(());
        }
    })
    .and_then(|mut watcher| {
        watcher.watch(home, RecursiveMode::NonRecursive)?;
        Ok(watcher)
    });
    let _watcher = match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::warn!(
                target: "codex_core::security::transition",
                %error,
                home = %home.display(),
                "no file notifications for the security state; polling it"
            );
            None
        }
    };
    let mut last = std::fs::read(home.join(STATE_FILE)).ok();
    loop {
        match rx.recv_timeout(POLL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            // No watcher: poll only (never spin on a closed channel).
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
        }
        while rx.try_recv().is_ok() {}
        {
            let mut watches = WATCHES.lock().unwrap_or_else(PoisonError::into_inner);
            if trees_on(home).is_empty() && watch.on.receiver_count() == 0 {
                watches.remove(home);
                return;
            }
        }
        let contents = std::fs::read(home.join(STATE_FILE)).ok();
        if contents == last {
            continue;
        }
        last = contents;
        let reached = apply(home);
        let has_trees = !trees_on(home).is_empty();
        // Subscribers are told of a change made by another process: one this
        // process made itself (its trees already held it) only updates the
        // value, since the confirming screen reports it.
        watch.on.send_if_modified(|on| {
            let now = crate::security::revocation_change::kill_switch_on(home);
            let changed = std::mem::replace(on, now) != now;
            changed && (reached > 0 || !has_trees)
        });
    }
}

fn trees_on(home: &Path) -> Vec<Arc<SharedEffectivePolicy>> {
    let mut trees = TREES.lock().unwrap_or_else(PoisonError::into_inner);
    trees.retain(|(_, tree)| tree.strong_count() > 0);
    trees
        .iter()
        .filter(|(tree_home, _)| tree_home == home)
        .filter_map(|(_, tree)| tree.upgrade())
        .collect()
}

/// The saved kill switch reaches this process's trees on `home`. Returns how
/// many took a change.
pub(super) fn apply(home: &Path) -> usize {
    let Ok(Some(stored)) = crate::security::recovery::load(home) else {
        return 0;
    };
    // Never between a commit of this process saving the file and applying
    // it: the commit's own trees take it then, under its epoch.
    let _commit = super::COMMITS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut reached = 0;
    for tree in trees_on(home) {
        let turned_on = {
            let mut guard = tree.state.write().unwrap_or_else(PoisonError::into_inner);
            let Some(state) = guard.as_mut() else {
                continue;
            };
            let held = state.persisted.revocations.kill_switch_event_id().cloned();
            if held.as_ref() == stored.revocations.kill_switch_event_id() {
                continue;
            }
            let mut revocations = state.persisted.revocations.clone();
            if let Err(error) = revocations.merge(&stored.revocations) {
                tracing::warn!(
                    target: "codex_core::security::transition",
                    %error,
                    "cannot take the kill switch saved by another process"
                );
                continue;
            }
            // This tree's switch is the later one: nothing to take.
            if revocations.kill_switch_event_id() == held.as_ref() {
                continue;
            }
            let level = state.persisted.settings.level;
            if let Err(error) = adopt(state, level, revocations) {
                tracing::warn!(
                    target: "codex_core::security::transition",
                    %error,
                    "cannot take the kill switch saved by another process"
                );
                continue;
            }
            reached += 1;
            state.persisted.revocations.kill_switch_active
        };
        tracing::info!(
            target: "codex_core::security::transition",
            kill_switch_active = turned_on,
            "kill switch changed by another process on this home"
        );
        // Its sessions stop their work and the broker's channels close.
        if turned_on {
            notify_sinks(&tree);
        }
    }
    reached
}

#[cfg(test)]
pub(super) fn start_for_test(home: &Path) -> tokio::sync::watch::Receiver<bool> {
    subscribe(home)
}
