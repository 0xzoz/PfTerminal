use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;
use std::time::Duration;
use std::time::Instant;

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
    const ROUNDS: usize = 3;
    let exe = std::env::current_exe().expect("test binary");
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
        for _ in 0..LAUNCHES {
            children.push(
                std::process::Command::new(&exe)
                    .args([
                        "--exact",
                        "sqlite::tests::concurrent_launch_child",
                        "--test-threads=1",
                    ])
                    .env(RACE_HOME_ENV, &home)
                    .env(RACE_START_ENV, start.as_millis().to_string())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .expect("spawn launch"),
            );
        }
        let failures: Vec<String> = children
            .into_iter()
            .map(|child| child.wait_with_output().expect("launch output"))
            .filter(|output| {
                !output.status.success()
                    || !String::from_utf8_lossy(&output.stdout).contains("1 passed")
            })
            .map(|output| {
                format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
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
