//! Shared SQLite connection configuration.

#![expect(
    clippy::disallowed_methods,
    reason = "this is the centralized SQLite connection shim"
)]

use crate::DbTelemetry;
use crate::migrations::repair_legacy_recency_migration_version;
use crate::migrations::runtime_goals_migrator;
use crate::migrations::runtime_logs_migrator;
use crate::migrations::runtime_memories_migrator;
use crate::migrations::runtime_state_migrator;
use crate::migrations::runtime_thread_history_migrator;
use crate::runtime::RuntimeDbInitError;
use crate::runtime::validate_applied_migrations;
use crate::telemetry;
use crate::telemetry::DbKind;
use codex_utils_absolute_path::AbsolutePathBuf;
use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::Connection;
use sqlx::Error;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::SqliteAutoVacuum;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

const LOGS_DB_FILENAME: &str = "pfterminal_logs_2.sqlite";
const GOALS_DB_FILENAME: &str = "pfterminal_goals_1.sqlite";
const MEMORIES_DB_FILENAME: &str = "pfterminal_memories_1.sqlite";
const STATE_DB_FILENAME: &str = "pfterminal_state_5.sqlite";
const THREAD_HISTORY_DB_FILENAME: &str = "pfterminal_thread_history_1.sqlite";
/// Held while a runtime database is created and migrated (#549). Never
/// deleted: removing a lock file while another process waits on it would let
/// a third process lock a new file at the same path.
const DB_INIT_LOCK_FILENAME: &str = "pfterminal_db_init.lock";
/// How long to wait for another process's database creation or migrations.
/// A migration can rewrite a large state DB, so this is generous.
const DB_INIT_LOCK_WAIT: Duration = Duration::from_secs(120);
const DB_INIT_LOCK_POLL: Duration = Duration::from_millis(20);
/// A wait longer than this is logged, so a stalled holder can be found.
const DB_INIT_LOCK_REPORT_AFTER: Duration = Duration::from_secs(1);
const MIGRATION_RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy)]
struct RuntimeDbSpec {
    label: &'static str,
    filename: &'static str,
    legacy_filename: &'static str,
    kind: DbKind,
    open_phase: &'static str,
    migrate_phase: &'static str,
}

impl RuntimeDbSpec {
    fn path(self, codex_home: &Path) -> PathBuf {
        codex_home.join(self.filename)
    }
}

const STATE_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "state DB",
    filename: STATE_DB_FILENAME,
    legacy_filename: "state_5.sqlite",
    kind: DbKind::State,
    open_phase: "open_state",
    migrate_phase: "migrate_state",
};

const LOGS_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "log DB",
    filename: LOGS_DB_FILENAME,
    legacy_filename: "logs_2.sqlite",
    kind: DbKind::Logs,
    open_phase: "open_logs",
    migrate_phase: "migrate_logs",
};

const GOALS_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "goals DB",
    filename: GOALS_DB_FILENAME,
    legacy_filename: "goals_1.sqlite",
    kind: DbKind::Goals,
    open_phase: "open_goals",
    migrate_phase: "migrate_goals",
};

const MEMORIES_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "memories DB",
    filename: MEMORIES_DB_FILENAME,
    legacy_filename: "memories_1.sqlite",
    kind: DbKind::Memories,
    open_phase: "open_memories",
    migrate_phase: "migrate_memories",
};

const THREAD_HISTORY_DB: RuntimeDbSpec = RuntimeDbSpec {
    label: "thread history DB",
    filename: THREAD_HISTORY_DB_FILENAME,
    legacy_filename: "thread_history_1.sqlite",
    kind: DbKind::ThreadHistory,
    open_phase: "open_thread_history",
    migrate_phase: "migrate_thread_history",
};

const RUNTIME_DBS: [RuntimeDbSpec; 5] =
    [STATE_DB, LOGS_DB, GOALS_DB, MEMORIES_DB, THREAD_HISTORY_DB];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDbPath {
    pub label: &'static str,
    pub path: PathBuf,
}

/// Resolved configuration shared by all Codex SQLite connections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteConfig {
    sqlite_home: AbsolutePathBuf,
    logs_db_path_override: Option<AbsolutePathBuf>,
}

impl SqliteConfig {
    pub fn from_sqlite_home(sqlite_home: AbsolutePathBuf) -> Self {
        Self {
            sqlite_home,
            logs_db_path_override: None,
        }
    }

    /// Override the logs database path while retaining the shared SQLite home for other runtime
    /// databases.
    pub fn with_logs_db_path(mut self, logs_db_path: AbsolutePathBuf) -> Self {
        self.logs_db_path_override = Some(logs_db_path);
        self
    }

    pub fn new_for_testing(sqlite_home: AbsolutePathBuf) -> Self {
        Self::from_sqlite_home(sqlite_home)
    }

    pub fn home(&self) -> &Path {
        self.sqlite_home.as_path()
    }

    /// Return the path to the primary state database.
    pub fn state_db_path(&self) -> PathBuf {
        STATE_DB.path(self.home())
    }

    /// Return whether this home already contains state that can supply runtime overlays.
    ///
    /// Configuration loading uses this check before opening the state runtime so read-only CLI
    /// commands do not create an otherwise absent state database. The legacy filename remains a
    /// valid source because runtime initialization adopts it into the PF namespace.
    pub fn has_existing_state_db(&self) -> bool {
        self.state_db_path().exists() || self.home().join(STATE_DB.legacy_filename).exists()
    }

    /// Return the path to the logs database.
    pub fn logs_db_path(&self) -> PathBuf {
        self.logs_db_path_override.as_ref().map_or_else(
            || LOGS_DB.path(self.home()),
            codex_utils_absolute_path::AbsolutePathBuf::to_path_buf,
        )
    }

    /// Return the path to the goals database.
    pub fn goals_db_path(&self) -> PathBuf {
        GOALS_DB.path(self.home())
    }

    /// Return the path to the memories database.
    pub fn memories_db_path(&self) -> PathBuf {
        MEMORIES_DB.path(self.home())
    }

    /// Return the path to the paginated thread-history database.
    pub fn thread_history_db_path(&self) -> PathBuf {
        THREAD_HISTORY_DB.path(self.home())
    }

    /// Return the paths to every database managed by the state runtime.
    pub fn runtime_db_paths(&self) -> Vec<RuntimeDbPath> {
        RUNTIME_DBS
            .iter()
            .map(|spec| RuntimeDbPath {
                label: spec.label,
                path: spec.path(self.home()),
            })
            .collect()
    }

    /// Adopt pre-namespace PF Terminal databases once, while refusing to touch an upstream
    /// `.codex` home or a database whose applied migrations do not match this distribution.
    pub(crate) async fn migrate_legacy_runtime_db_names(&self) -> anyhow::Result<()> {
        let has_legacy_db = RUNTIME_DBS
            .iter()
            .any(|spec| self.home().join(spec.legacy_filename).exists());
        if !has_legacy_db {
            return Ok(());
        }
        if self.home().file_name().and_then(|name| name.to_str()) == Some(".codex") {
            anyhow::bail!(
                "refusing to rename upstream-named databases in {}: this database appears to belong to a different Codex/Corbanu Terminal distribution; follow the repair recipe in pfterminal_codex_home_collision_incident_20260710.md",
                self.home().display()
            );
        }

        for spec in RUNTIME_DBS {
            let legacy_path = self.home().join(spec.legacy_filename);
            let namespaced_path = spec.path(self.home());
            if !tokio::fs::try_exists(&legacy_path).await?
                || tokio::fs::try_exists(&namespaced_path).await?
            {
                continue;
            }
            let migrator = match spec.kind {
                DbKind::State => runtime_state_migrator(),
                DbKind::Logs => runtime_logs_migrator(),
                DbKind::Goals => runtime_goals_migrator(),
                DbKind::Memories => runtime_memories_migrator(),
                DbKind::ThreadHistory => runtime_thread_history_migrator(),
            };
            validate_applied_migrations(self, &legacy_path, &migrator).await?;
            rename_if_source_still_exists(&legacy_path, &namespaced_path).await?;
            for suffix in ["-wal", "-shm"] {
                let legacy_sidecar = PathBuf::from(format!("{}{suffix}", legacy_path.display()));
                let namespaced_sidecar =
                    PathBuf::from(format!("{}{suffix}", namespaced_path.display()));
                rename_if_source_still_exists(&legacy_sidecar, &namespaced_sidecar).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn open_state_db(
        &self,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        // New state DBs should use incremental auto-vacuum, but retrofitting an
        // existing DB requires a full VACUUM. Do not attempt that during process
        // startup: it is maintenance work that can contend with foreground writers.
        self.open_runtime_db(STATE_DB, migrator, telemetry_override, init_lock)
            .await
    }

    /// A second, small pool on the already opened state DB whose connections
    /// wait at most `busy_timeout` for another connection's write lock, for
    /// writes a model request waits on (see `StateRuntime::quick`).
    pub(super) async fn open_state_db_quick(
        &self,
        busy_timeout: Duration,
    ) -> Result<SqlitePool, Error> {
        let options = SqliteConnectOptions::new()
            .filename(self.state_db_path())
            .create_if_missing(false)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(busy_timeout)
            .log_statements(LevelFilter::Off);
        SqlitePoolOptions::new()
            .max_connections(2)
            // Waiting for one of its connections counts against the same
            // short wait; a timeout reads as a busy DB.
            .acquire_timeout(busy_timeout)
            .connect_with(options)
            .await
    }

    pub(super) async fn open_logs_db(
        &self,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        self.open_runtime_db(LOGS_DB, migrator, telemetry_override, init_lock)
            .await
    }

    pub(super) async fn open_goals_db(
        &self,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        self.open_runtime_db(GOALS_DB, migrator, telemetry_override, init_lock)
            .await
    }

    pub(super) async fn open_memories_db(
        &self,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        self.open_runtime_db(MEMORIES_DB, migrator, telemetry_override, init_lock)
            .await
    }

    pub(super) async fn open_thread_history_db(
        &self,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        self.open_runtime_db(THREAD_HISTORY_DB, migrator, telemetry_override, init_lock)
            .await
    }

    /// Opens and migrates one runtime database under the init lock (#549):
    /// `init_lock` when the caller holds it already, otherwise taken here.
    async fn open_runtime_db(
        &self,
        spec: RuntimeDbSpec,
        migrator: &Migrator,
        telemetry_override: Option<&dyn DbTelemetry>,
        init_lock: Option<&RuntimeDbInitLock>,
    ) -> anyhow::Result<SqlitePool> {
        let path = spec.path(self.home());
        let _own_lock = match init_lock {
            Some(_) => None,
            None => Some(self.lock_runtime_db_init().await),
        };
        let started = Instant::now();
        let pool_result = self
            .open_read_write_pool(&path)
            .await
            .map_err(anyhow::Error::from);
        telemetry::record_init_result(
            telemetry_override,
            spec.kind,
            spec.open_phase,
            started.elapsed(),
            &pool_result,
        );
        let pool = pool_result.map_err(|source| {
            RuntimeDbInitError::new(spec.label, "open", path.as_path(), source)
        })?;
        let started = Instant::now();
        let migrate_result = async {
            if matches!(spec.kind, DbKind::State) {
                repair_legacy_recency_migration_version(&pool, migrator).await?;
            }
            match migrator.run(&pool).await {
                // A process that does not take the init lock (an older
                // build, or one that timed out waiting) may have applied the
                // same migration first; it is then recorded, and one more run
                // usually completes (best effort: a process still applying a
                // long series can collide again). A failed migration is rolled
                // back, so the retry starts clean.
                Err(first) => {
                    tracing::info!("retrying the {} migration after: {first}", spec.label);
                    tokio::time::sleep(MIGRATION_RETRY_DELAY).await;
                    migrator.run(&pool).await.map_err(anyhow::Error::from)
                }
                Ok(()) => Ok(()),
            }
        }
        .await;
        telemetry::record_init_result(
            telemetry_override,
            spec.kind,
            spec.migrate_phase,
            started.elapsed(),
            &migrate_result,
        );
        if let Err(source) = migrate_result {
            pool.close().await;
            return Err(
                RuntimeDbInitError::new(spec.label, "migrate", path.as_path(), source).into(),
            );
        }
        Ok(pool)
    }

    /// Serializes creating and migrating the runtime databases across the
    /// processes sharing this home (#549). SQLite migrations take no lock of
    /// their own: processes started together on a new home all ran
    /// `CREATE TABLE`, and the losers failed and ran without a state DB.
    ///
    /// The lock is an OS file lock on [`DB_INIT_LOCK_FILENAME`], released
    /// when the returned guard is dropped, including when a process dies. It
    /// is not re-entrant: take it once and pass it to the `open_*_db` calls.
    /// If it can't be had within [`DB_INIT_LOCK_WAIT`] (or the file can't be
    /// opened), the guard holds nothing and the caller goes on unlocked; the
    /// migration retry in [`Self::open_runtime_db`] still covers the race.
    pub(crate) async fn lock_runtime_db_init(&self) -> RuntimeDbInitLock {
        let path = self.home().join(DB_INIT_LOCK_FILENAME);
        let unlocked = RuntimeDbInitLock { _file: None };
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) => {
                log_unlocked(&path, &err);
                return unlocked;
            }
        };
        let started = Instant::now();
        let mut reported_wait = false;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    keep_lock_file_nonempty(&file);
                    return RuntimeDbInitLock { _file: Some(file) };
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    let waited = started.elapsed();
                    if waited >= DB_INIT_LOCK_WAIT {
                        tracing::warn!(
                            "another process held the database init lock {} for over {:?}; opening unlocked",
                            path.display(),
                            DB_INIT_LOCK_WAIT
                        );
                        return unlocked;
                    }
                    if !reported_wait && waited >= DB_INIT_LOCK_REPORT_AFTER {
                        reported_wait = true;
                        tracing::info!(
                            "waiting for another process to create or migrate the databases ({})",
                            path.display()
                        );
                    }
                    tokio::time::sleep(DB_INIT_LOCK_POLL).await;
                }
                Err(std::fs::TryLockError::Error(err)) => {
                    log_unlocked(&path, &err);
                    return unlocked;
                }
            }
        }
    }

    /// Open a writable Codex SQLite database, creating it if necessary.
    pub async fn open_read_write_pool(&self, path: &Path) -> Result<SqlitePool, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5))
            .log_statements(LevelFilter::Off);
        // Setting `auto_vacuum` takes the write lock even when the value is
        // unchanged, so it must not be a per-connection option: every pooled
        // connection would run it as it connects, and on a database another
        // process is writing, opening a connection - for a reader too - waited
        // out the busy timeout and failed. It only takes effect before the first
        // table exists (later changes need a VACUUM), so set it once, on the one
        // connection that creates the database, and leave existing ones alone.
        if !database_has_content(path) {
            let mut creator = options
                .clone()
                .auto_vacuum(SqliteAutoVacuum::Incremental)
                .connect()
                .await?;
            // An empty file keeps no settings; writing the header persists them.
            sqlx::query("VACUUM").execute(&mut creator).await?;
            creator.close().await?;
        }
        SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
    }

    /// Open an existing Codex SQLite database without creating or modifying it.
    pub async fn open_read_only_pool(&self, path: &Path) -> Result<SqlitePool, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .read_only(true)
            .log_statements(LevelFilter::Off);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
    }
}

/// Held while the runtime databases are created and migrated (see
/// [`SqliteConfig::lock_runtime_db_init`]); holds nothing when the lock could
/// not be had.
pub(crate) struct RuntimeDbInitLock {
    _file: Option<std::fs::File>,
}

/// A read-only or sandboxed home is expected not to take the lock: quiet.
/// #586: the lock file is never left empty. On Linux an empty file at its
/// path can be the sandbox's placeholder for a protected command's run (the
/// lock is denied to agent commands even before it exists). The run's cleanup
/// removes an empty placeholder even while a launch holds it locked, and the
/// next launch would then lock a new file.
fn keep_lock_file_nonempty(mut file: &std::fs::File) {
    use std::io::Write;
    if file.metadata().is_ok_and(|metadata| metadata.len() == 0) {
        let _ = file.write_all(b"\n");
    }
}

fn log_unlocked(path: &Path, err: &std::io::Error) {
    if matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) {
        tracing::debug!(
            "no database init lock at {}; opening unlocked: {err}",
            path.display()
        );
    } else {
        tracing::warn!(
            "could not take the database init lock {}; opening unlocked: {err}",
            path.display()
        );
    }
}

/// Whether `path` is an existing, non-empty database file.
pub(crate) fn database_has_content(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0)
}

/// Keep fault-fixture connection options and checkout barriers intact at the SQLite boundary.
#[cfg(test)]
pub(crate) async fn open_pool_for_testing(
    pool_options: SqlitePoolOptions,
    connect_options: SqliteConnectOptions,
) -> Result<SqlitePool, Error> {
    pool_options.connect_with(connect_options).await
}

async fn rename_if_source_still_exists(source: &Path, destination: &Path) -> anyhow::Result<()> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(err)
            if err.kind() == std::io::ErrorKind::NotFound
                && tokio::fs::try_exists(destination).await? =>
        {
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
#[path = "sqlite_tests.rs"]
mod tests;
