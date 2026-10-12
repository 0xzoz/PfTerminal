use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use std::time::Duration;
use std::time::Instant;

use super::DB_INIT_LOCK_FILENAME;
use super::database_has_content;

#[tokio::test]
async fn connecting_to_an_existing_database_does_not_wait_for_its_writer() -> anyhow::Result<()> {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let path = sqlite.state_db_path();
    // A database this call creates still gets incremental auto-vacuum.
    let writer = sqlite.open_read_write_pool(&path).await?;
    sqlx::query("CREATE TABLE t (v INTEGER)")
        .execute(&writer)
        .await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA auto_vacuum")
            .fetch_one(&writer)
            .await?,
        2
    );

    // Another process holds the write lock for longer than the busy timeout.
    let mut held = writer.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("INSERT INTO t VALUES (1)")
        .execute(&mut *held)
        .await?;

    // A new connection - a reader here - must not need that lock to open.
    let started = Instant::now();
    let reader = sqlite.open_read_write_pool(&path).await?;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM t")
        .fetch_one(&reader)
        .await?;
    assert_eq!(rows, 0);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "connecting waited {:?} for another writer",
        started.elapsed()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("PRAGMA auto_vacuum")
            .fetch_one(&reader)
            .await?,
        2
    );
    held.rollback().await?;
    reader.close().await;
    writer.close().await;
    Ok(())
}

/// Set for the child processes of
/// [`concurrent_first_launches_on_a_new_home_all_get_a_state_db`].
const RACE_HOME_ENV: &str = "CODEX_STATE_TEST_RACE_HOME";
const RACE_START_ENV: &str = "CODEX_STATE_TEST_RACE_START_MS";

/// One launch of the concurrent-launch test; does nothing unless that test
/// started this process.
#[test]
fn concurrent_launch_child() {
    let (Some(home), Some(start)) = (
        std::env::var_os(RACE_HOME_ENV),
        std::env::var(RACE_START_ENV).ok(),
    ) else {
        return;
    };
    let start =
        std::time::UNIX_EPOCH + Duration::from_millis(start.parse().expect("start time in ms"));
    if let Ok(wait) = start.duration_since(std::time::SystemTime::now()) {
        std::thread::sleep(wait);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let sqlite = crate::SqliteConfig::new_for_testing(std::path::Path::new(&home).abs());
        crate::StateRuntime::init(sqlite, "test-provider".to_string())
            .await
            .map(drop)
            .unwrap_or_else(|err| panic!("state runtime failed to open: {err:#}"));
    });
}

/// #549: processes started at the same moment on a brand-new home race to
/// create and migrate its databases; every one must still get them.
#[test]
fn concurrent_first_launches_on_a_new_home_all_get_a_state_db() {
    const LAUNCHES: usize = 10;
    const ROUNDS: usize = 2;
    let exe = std::env::current_exe().expect("test binary");
    // `sqlite::tests::concurrent_launch_child`, whatever this module is called.
    let child_test = format!(
        "{}::concurrent_launch_child",
        module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, path)| path)
    );
    for round in 0..ROUNDS {
        let home = crate::runtime::test_support::unique_temp_dir();
        std::fs::create_dir_all(&home).expect("create home");
        let _cleanup = scopeguard::guard(home.clone(), |home| {
            let _ = std::fs::remove_dir_all(home);
        });
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            + Duration::from_millis(1_500);
        // Every launch is started before any is waited for.
        let mut children = Vec::with_capacity(LAUNCHES);
        for index in 0..LAUNCHES {
            children.push(
                std::process::Command::new(&exe)
                    .args(["--exact", &child_test, "--test-threads=1"])
                    .env(RACE_HOME_ENV, &home)
                    .env(RACE_START_ENV, start.as_millis().to_string())
                    // Files, not pipes: a child that fails loudly never blocks.
                    .stdout(std::fs::File::create(home.join(format!("out-{index}"))).expect("out"))
                    .stderr(std::fs::File::create(home.join(format!("err-{index}"))).expect("err"))
                    .spawn()
                    .expect("spawn launch"),
            );
        }
        // Under nextest's 60 s kill, a stuck launch still fails readably.
        let deadline = Instant::now() + Duration::from_secs(20);
        let failures: Vec<String> = children
            .into_iter()
            .enumerate()
            .filter_map(|(index, mut child)| {
                let status = loop {
                    if let Some(status) = child.try_wait().expect("poll launch") {
                        break Some(status);
                    }
                    if Instant::now() > deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                let stdout =
                    std::fs::read_to_string(home.join(format!("out-{index}"))).unwrap_or_default();
                let stderr =
                    std::fs::read_to_string(home.join(format!("err-{index}"))).unwrap_or_default();
                let passed =
                    status.is_some_and(|status| status.success()) && stdout.contains("1 passed");
                (!passed).then(|| format!("{status:?}\n{stdout}\n{stderr}"))
            })
            .collect();
        assert!(
            failures.is_empty(),
            "round {round}: {} of {LAUNCHES} launches had no state DB:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// #549: while another process holds the init lock, a runtime DB is neither
/// created nor migrated; it is once the lock is released.
#[tokio::test]
async fn runtime_databases_wait_for_the_init_lock() -> anyhow::Result<()> {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    // Another process's hold: a separate open of the lock file.
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sqlite_home.join(DB_INIT_LOCK_FILENAME))?;
    holder.lock()?;

    let opening = tokio::spawn({
        let sqlite = sqlite.clone();
        async move {
            let migrator = crate::migrations::runtime_goals_migrator();
            sqlite
                .open_goals_db(
                    &migrator, /*telemetry_override*/ None, /*init_lock*/ None,
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!opening.is_finished(), "opened while the lock was held");
    assert!(!database_has_content(&sqlite.goals_db_path()));

    holder.unlock()?;
    let pool = tokio::time::timeout(Duration::from_secs(10), opening).await???;
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(tables, 1);
    pool.close().await;
    Ok(())
}

/// #586: once a launch holds the init lock, the file is not empty. On Linux an
/// empty file at that path can be a sandbox placeholder, which the command
/// run's cleanup removes even while it is locked.
#[tokio::test]
async fn the_init_lock_file_is_not_left_empty() -> anyhow::Result<()> {
    let sqlite_home = crate::runtime::test_support::unique_temp_dir();
    tokio::fs::create_dir_all(&sqlite_home).await?;
    let _cleanup = scopeguard::guard(sqlite_home.clone(), |sqlite_home| {
        let _ = std::fs::remove_dir_all(sqlite_home);
    });
    let lock = sqlite_home.join(DB_INIT_LOCK_FILENAME);
    std::fs::write(&lock, b"")?;
    let sqlite = crate::SqliteConfig::new_for_testing(sqlite_home.as_path().abs());
    let held = sqlite.lock_runtime_db_init().await;
    assert_eq!(std::fs::metadata(&lock)?.len(), 1);
    drop(held);
    // Later launches leave it as it is.
    let held = sqlite.lock_runtime_db_init().await;
    assert_eq!(std::fs::metadata(&lock)?.len(), 1);
    drop(held);
    Ok(())
}
