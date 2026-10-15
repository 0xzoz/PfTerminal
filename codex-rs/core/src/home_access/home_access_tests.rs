use super::*;
use pretty_assertions::assert_eq;

const WINDOW: Duration = Duration::from_secs(3600);

fn step(attempt: u32, delay_secs: u64, remaining_secs: u64) -> Option<HomeWaitStep> {
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
        vec![
            step(1, 1, 3600),
            step(2, 2, 3599),
            step(3, 4, 3597),
            step(4, 8, 3593),
            step(5, 16, 3585),
            step(6, 32, 3569),
            step(7, 60, 3537),
            step(8, 60, 3477),
            step(9, 60, 3417),
        ]
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
        step(1, 1, 90)
    );
    assert_eq!(
        wait.next_step(
            /*home_unreadable*/ true,
            start + Duration::from_secs(80),
            window
        ),
        step(2, 2, 10)
    );
    assert_eq!(
        wait.next_step(
            /*home_unreadable*/ true,
            start + Duration::from_secs(89),
            window
        ),
        step(3, 1, 1),
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
    assert!(lines.iter().any(|line| line.starts_with("cwd: ")), "{record}");
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
    assert_eq!(lines.iter().filter(|line| line.starts_with("- ")).count(), 4);
    assert_eq!(
        lines.last().copied(),
        Some("Details: /Users/me/Library/Logs/Corbanu/home-access.log")
    );
    assert!(!message.contains("macOS is blocking"), "{message}");
}
