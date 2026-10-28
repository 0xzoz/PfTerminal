use super::SessionUsage;
use codex_protocol::protocol::RealtimeResponseUsage;
use codex_state::accounting::Patch;
use codex_state::accounting::Presence;
use pretty_assertions::assert_eq;

fn usage(
    input: Option<u64>,
    cached: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
) -> RealtimeResponseUsage {
    RealtimeResponseUsage {
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        total_tokens: total,
    }
}

fn counts(patch: &Patch) -> [Option<i64>; 4] {
    [patch.input, patch.read, patch.output, patch.total].map(|presence| match presence {
        Presence::Number(count) => Some(i64::from(count)),
        Presence::Missing => None,
        Presence::Null => panic!("a realtime patch never states null"),
    })
}

/// #889 D223: each response's report adds to the session's running sums,
/// recorded as one cumulative patch that states tokens and nothing else.
#[test]
fn realtime_usage_sums_each_reported_response() {
    let mut session = SessionUsage::default();
    let first = session
        .add(&usage(Some(132), Some(64), Some(121), Some(253)))
        .expect("first report");
    assert_eq!(counts(&first), [Some(132), Some(64), Some(121), Some(253)]);
    let second = session
        .add(&usage(Some(200), Some(150), Some(30), Some(230)))
        .expect("second report");
    assert_eq!(
        counts(&second),
        [Some(332), Some(214), Some(151), Some(483)]
    );
    assert_eq!(
        Patch {
            input: Presence::Missing,
            read: Presence::Missing,
            output: Presence::Missing,
            total: Presence::Missing,
            ..second
        },
        Patch::default(),
        "a usage patch states no charge, refusal or session end"
    );
}

/// What the first report left out stays unknown, never zero; a later report
/// that leaves out a count the session states is not counted, so every sum
/// covers the same responses.
#[test]
fn realtime_usage_keeps_the_first_reports_shape() {
    let mut session = SessionUsage::default();
    let first = session
        .add(&usage(Some(100), None, Some(10), None))
        .expect("first report");
    assert_eq!(counts(&first), [Some(100), None, Some(10), None]);
    // Extra counts are ignored; the stated ones still add.
    let second = session
        .add(&usage(Some(50), Some(20), Some(5), Some(55)))
        .expect("second report");
    assert_eq!(counts(&second), [Some(150), None, Some(15), None]);
    // Output missing: not counted at all.
    assert_eq!(session.add(&usage(Some(70), None, None, None)), None);
    let third = session
        .add(&usage(Some(1), None, Some(1), None))
        .expect("third report");
    assert_eq!(counts(&third), [Some(151), None, Some(16), None]);
}

/// Reports the ledger could not hold as stated are left out or narrowed:
/// cached input above input, a total that is not input plus output, and a
/// count beyond the ledger's range.
#[test]
fn realtime_usage_rejects_inconsistent_reports() {
    let mut session = SessionUsage::default();
    assert_eq!(session.add(&usage(Some(10), Some(11), Some(1), None)), None);
    // The total disagrees with input plus output: read as not stated, and so
    // the session states no total.
    let first = session
        .add(&usage(Some(10), Some(5), Some(1), Some(12)))
        .expect("first report");
    assert_eq!(counts(&first), [Some(10), Some(5), Some(1), None]);
    assert_eq!(
        session.add(&usage(Some(u64::MAX), Some(0), Some(1), None)),
        None
    );
    assert_eq!(
        session.add(&usage(Some(i64::MAX as u64), Some(0), Some(1), None)),
        None,
        "a sum past the ledger's range is not counted"
    );
    let second = session
        .add(&usage(Some(2), Some(0), Some(2), Some(4)))
        .expect("second report");
    assert_eq!(counts(&second), [Some(12), Some(5), Some(3), None]);
}
