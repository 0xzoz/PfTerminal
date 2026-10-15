use super::*;
use pretty_assertions::assert_eq;

const WINDOW: Duration = Duration::from_secs(3600);

/// `(attempt, delay, remaining)` in seconds.
fn step((attempt, delay_secs, remaining_secs): (u32, u64, u64)) -> Option<HomeWaitStep> {
    Some(HomeWaitStep {
        attempt,
        delay: Duration::from_secs(delay_secs),
        remaining: Duration::from_secs(remaining_secs),
    })
}

#[test]
fn home_failures_back_off_exponentially_up_to_a_minute() {
    let start = Instant::now();
    let mut wait = HomeWait::default();
    let mut steps = Vec::new();
    let mut elapsed = 0;
    for _ in 0..9 {
        let at = start + Duration::from_secs(elapsed);
        let next = wait.next_step(/*home_unreadable*/ true, at, WINDOW);
        elapsed += next.as_ref().map_or(0, |step| step.delay.as_secs());
        steps.push(next);
    }
    assert_eq!(
        steps,
        [
            (1, 1, 3600),
            (2, 2, 3599),
            (3, 4, 3597),
            (4, 8, 3593),
            (5, 16, 3585),
            (6, 32, 3569),
            (7, 60, 3537),
            (8, 60, 3477),
            (9, 60, 3417),
        ]
        .map(step)
        .into_iter()
        .collect::<Vec<_>>()
    );
}

#[test]
fn the_window_bounds_the_wait_and_other_failures_never_wait() {
    let start = Instant::now();
    let mut wait = HomeWait::default();
    assert_eq!(
        wait.next_step(/*home_unreadable*/ false, start, WINDOW),
        None
    );
    assert_eq!(
        wait.next_step(/*home_unreadable*/ true, start, Duration::ZERO),
        None,
        "no window (the default outside exec): fail as before"
    );
    let window = Duration::from_secs(90);
    let mut wait = HomeWait::default();
    assert_eq!(
        wait.next_step(/*home_unreadable*/ true, start, window),
        step((1, 1, 90))
    );
    assert_eq!(
        wait.next_step(
            /*home_unreadable*/ true,
            start + Duration::from_secs(80),
            window
        ),
        step((2, 2, 10))
    );
    assert_eq!(
        wait.next_step(
            /*home_unreadable*/ true,
            start + Duration::from_secs(89),
            window
        ),
        step((3, 1, 1)),
        "the last wait ends with the window"
    );
    assert_eq!(
        wait.next_step(
            /*home_unreadable*/ true,
            start + Duration::from_secs(90),
            window
        ),
        None,
        "spent: the failure is handled as usual (exec exits 1)"
    );
}

fn eperm() -> std::future::Ready<std::io::Result<()>> {
    std::future::ready(Err(std::io::Error::from_raw_os_error(1)))
}

/// Exec's wait before saving the session: until the folder lists again, or
/// what is left of the window since the first failure is spent; other errors
/// never wait.
#[tokio::test]
async fn the_exit_wait_ends_when_the_folder_lists_again_or_the_window_is_spent() {
    let reads = std::sync::atomic::AtomicU32::new(0);
    let dir = Path::new("/Volumes/Drive/home/sessions/2026/10/15");
    let mut recorded = Vec::new();
    let started = Instant::now();
    wait_until_readable(
        dir,
        |_| match reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0 => eperm(),
            _ => std::future::ready(Ok(())),
        },
        |operation, path, error| recorded.push(format!("{operation} {} {error}", path.display())),
        HomeWait::default(),
        WINDOW,
    )
    .await;
    let waited = started.elapsed();
    assert_eq!(
        (reads.into_inner(), recorded),
        (
            2,
            vec![format!(
                "read_dir (opendir) {} Operation not permitted (os error 1)",
                dir.display()
            )]
        )
    );
    assert!(waited >= FIRST_WAIT && waited < WINDOW, "{waited:?}");

    // The turn already waited most of the window: only the rest is left.
    let first_failure = Instant::now()
        .checked_sub(WINDOW - Duration::from_millis(1500))
        .expect("an hour ago");
    let started = Instant::now();
    wait_until_readable(
        dir,
        |_| eperm(),
        |_, _, _| {},
        HomeWait {
            started: Some(first_failure),
            attempts: 0,
        },
        WINDOW,
    )
    .await;
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(1400) && waited < Duration::from_secs(30),
        "only the rest of the window: {waited:?}"
    );

    let started = Instant::now();
    wait_until_readable(
        dir,
        |_| std::future::ready(Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
        |_, _, _| panic!("only EPERM is recorded"),
        HomeWait::default(),
        WINDOW,
    )
    .await;
    assert!(started.elapsed() < FIRST_WAIT);
}

#[test]
fn an_unsaved_rollout_keeps_its_first_failure_until_an_append_succeeds() {
    let start = Instant::now();
    let later = start + Duration::from_secs(30);
    let dir = PathBuf::from("/home/sessions");
    let first = next_unsaved(None, Some(dir.clone()), start);
    let again = next_unsaved(first.clone(), Some(dir.clone()), later);
    assert_eq!(
        (first, again.clone(), next_unsaved(again, None, later)),
        (Some((dir.clone(), start)), Some((dir, start)), None)
    );
}

#[test]
fn only_marked_home_failures_count() {
    let marked = CodexErr::Io(codex_protocol::home_unreadable::home_unreadable_io_error(
        "helper: Corbanu could not read its home".to_string(),
    ));
    let plain = CodexErr::Io(std::io::Error::from_raw_os_error(1));
    assert_eq!(
        (marked.is_home_unreadable(), plain.is_home_unreadable()),
        (true, false)
    );
}

#[test]
fn a_record_lists_the_failure_and_each_parent_without_contents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).expect("home");
    std::fs::write(home.join("auth.json"), "secret-token-value").expect("file");
    let log = dir.path().join("logs").join("home-access.log");

    write_record(
        &log,
        "read_dir (opendir)",
        &home,
        "Operation not permitted (os error 1)",
    )
    .expect("record");

    let record = std::fs::read_to_string(&log).expect("log");
    let lines: Vec<&str> = record.lines().collect();
    assert!(lines[0].starts_with("=== "), "{record}");
    assert!(
        lines[0].contains(&format!("pid={}", std::process::id())),
        "{record}"
    );
    assert_eq!(
        &lines[1..3],
        &[
            format!("operation: read_dir (opendir) {}", home.display()).as_str(),
            "error: Operation not permitted (os error 1)",
        ]
    );
    assert!(
        lines.iter().any(|line| line.starts_with("cwd: ")),
        "{record}"
    );
    for ancestor in home.ancestors() {
        assert!(
            record.contains(&format!("  {}: ", ancestor.display())),
            "{} missing from:\n{record}",
            ancestor.display()
        );
    }
    #[cfg(target_os = "macos")]
    for fact in [
        "responsible process: pid=",
        "sandbox_check(file-read-data): ",
        "mounted volume: /",
        " flags=0x",
        " acl=",
    ] {
        assert!(record.contains(fact), "{fact} missing from:\n{record}");
    }
    assert!(!record.contains("secret-token-value"), "{record}");
}

#[test]
fn the_message_lists_causes_without_asserting_one() {
    let message = unreadable_home_message(
        Path::new("/Volumes/Drive/home"),
        "Operation not permitted (os error 1)",
        Some(Path::new("/Users/me/Library/Logs/Corbanu/home-access.log")),
    );
    let lines: Vec<&str> = message.lines().collect();
    assert_eq!(
        lines[0],
        "Corbanu could not read its home (Operation not permitted): /Volumes/Drive/home \
         (Operation not permitted (os error 1))."
    );
    assert_eq!(lines[1], "Possible causes:");
    assert_eq!(
        lines.iter().filter(|line| line.starts_with("- ")).count(),
        4
    );
    assert_eq!(
        lines.last().copied(),
        Some("Details: /Users/me/Library/Logs/Corbanu/home-access.log")
    );
    assert!(!message.contains("macOS is blocking"), "{message}");
}
