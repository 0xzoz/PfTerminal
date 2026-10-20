//! A test-only count of the classifier's work, for its linearity tests
//! (#1102).
//!
//! Every loop in the classifier (`tainted_action.rs` and the modules under
//! `tainted_action/`) steps a counter once per iteration; a test below holds
//! the code to that rule. The `codex-core` unit-test binary also counts the
//! bytes each thread allocates, which covers library calls that copy text
//! (`to_string`, `format!`, `collect`) outside any loop of ours. A linearity
//! test classifies the same input at two sizes and compares both counts,
//! which, unlike a wall-clock budget, do not depend on the machine or its
//! load. Outside tests a step is a no-op and nothing is counted.

#[cfg(test)]
use std::alloc::GlobalAlloc;
#[cfg(test)]
use std::alloc::Layout;
#[cfg(test)]
use std::alloc::System;
#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static STEPS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

/// One loop iteration of classifier work.
#[inline(always)]
pub(super) fn step() {
    #[cfg(test)]
    STEPS.with(|steps| steps.set(steps.get() + 1));
}

/// The system allocator, counting the bytes each thread asks for.
#[cfg(test)]
struct Counting;

#[cfg(test)]
#[global_allocator]
static COUNTING: Counting = Counting;

#[cfg(test)]
fn allocated(bytes: usize) {
    // `try_with`: a thread may still free or allocate as it exits.
    let _ = BYTES.try_with(|count| count.set(count.get() + bytes as u64));
}

#[cfg(test)]
// SAFETY: every call is passed to `System` unchanged; counting allocates
// nothing (the counters are constant-initialised thread locals).
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated(layout.size());
        // SAFETY: the caller's contract for `alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocated(layout.size());
        // SAFETY: the caller's contract for `alloc_zeroed`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract for `dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        allocated(new_size);
        // SAFETY: the caller's contract for `realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Work counted on this thread.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Work {
    /// Loop iterations.
    pub(super) steps: u64,
    /// Bytes allocated (a reallocation counts its new size).
    pub(super) bytes: u64,
}

#[cfg(test)]
fn now() -> Work {
    Work {
        steps: STEPS.with(Cell::get),
        bytes: BYTES.with(Cell::get),
    }
}

/// The work `run` does on this thread.
#[cfg(test)]
pub(super) fn measure(run: impl FnOnce()) -> Work {
    let before = now();
    run();
    let after = now();
    Work {
        steps: after.steps - before.steps,
        bytes: after.bytes - before.bytes,
    }
}

/// Input sizes a linearity test compares: `run(1)` and `run(SCALE)`.
#[cfg(test)]
pub(super) const SCALE: u64 = 4;

/// Linear work grows at most `SCALE` times (fixed costs make it less), work
/// growing with the square of the input `SCALE * SCALE` times. The bound
/// leaves room for `n log n` and fails anything near quadratic.
#[cfg(test)]
const MAX_GROWTH: u64 = 6;

/// Asserts that `run(scale)`, which classifies an input `scale` times as
/// long, does work (loop steps and allocated bytes, each) that grows
/// linearly with `scale`.
#[cfg(test)]
pub(super) fn assert_linear(label: &str, run: impl Fn(usize)) {
    let small = measure(|| run(1));
    let large = measure(|| run(SCALE as usize));
    assert!(small.steps > 0, "{label}: no classifier loop counted");
    for (counted, small, large) in [
        ("steps", small.steps, large.steps),
        ("bytes", small.bytes, large.bytes),
    ] {
        assert!(
            large <= MAX_GROWTH * small.max(1),
            "{label}: {small} {counted} at 1x, {large} at {SCALE}x (linear work grows at \
             most {SCALE}x, quadratic about {}x)",
            SCALE * SCALE
        );
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    /// The classifier's sources, which must step the counter in every loop.
    const SOURCES: &[(&str, &str)] = &[
        ("tainted_action.rs", include_str!("../tainted_action.rs")),
        ("arithmetic.rs", include_str!("arithmetic.rs")),
        ("indirect.rs", include_str!("indirect.rs")),
        ("invocation.rs", include_str!("invocation.rs")),
        ("nested.rs", include_str!("nested.rs")),
        ("outbound.rs", include_str!("outbound.rs")),
        ("paths.rs", include_str!("paths.rs")),
        ("quoted.rs", include_str!("quoted.rs")),
        ("sed.rs", include_str!("sed.rs")),
        ("shell.rs", include_str!("shell.rs")),
    ];

    /// The loop keyword a line starts with (`for`, `while` or `loop`),
    /// labelled or not, and the text from it.
    fn loop_start(line: &str) -> Option<(&str, &str)> {
        let mut code = line.trim_start();
        if let Some(label) = code.strip_prefix('\'')
            && let Some((name, rest)) = label.split_once(':')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            code = rest.trim_start();
        }
        ["for ", "while ", "loop {"]
            .into_iter()
            .find(|keyword| code.starts_with(keyword))
            .map(|keyword| (keyword.trim_end_matches('{').trim_end(), code))
    }

    /// Whether the body of the loop starting `header` (which runs on to
    /// the end of the file) begins with `work::step();`. The body opens at
    /// the first brace outside brackets, quotes and character literals,
    /// after `in` for a `for` loop (a pattern may hold braces).
    fn steps_first(keyword: &str, header: &str) -> bool {
        let bytes = header.as_bytes();
        let mut depth = 0usize;
        let mut seen_in = keyword != "for";
        let mut at = keyword.len();
        while at < bytes.len() {
            match bytes[at] {
                b'"' => {
                    at += 1;
                    while at < bytes.len() && bytes[at] != b'"' {
                        at += if bytes[at] == b'\\' { 2 } else { 1 };
                    }
                }
                b'\'' if bytes.get(at + 2) == Some(&b'\'') => at += 2,
                b'\'' if bytes.get(at + 1) == Some(&b'\\') => {
                    at += bytes[at + 3..]
                        .iter()
                        .position(|&byte| byte == b'\'')
                        .unwrap_or(0)
                        + 3;
                }
                b'{' if depth == 0 && seen_in => {
                    return header[at + 1..].trim_start().starts_with("work::step();");
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                _ if depth == 0 && bytes[at..].starts_with(b" in ") => seen_in = true,
                _ => {}
            }
            at += 1;
        }
        false
    }

    /// Loops in `source` (up to its tests) whose body does not start with
    /// `work::step();`, as `file:line`.
    fn unstepped(file: &str, source: &str) -> Vec<String> {
        let mut missing = Vec::new();
        let mut offset = 0;
        for (at, line) in source.split_inclusive('\n').enumerate() {
            let code = line.trim_start();
            if code.starts_with("mod tests") || code.starts_with("#[path = ") {
                break;
            }
            if !code.starts_with("//")
                && let Some((keyword, from)) = loop_start(line)
                && !steps_first(keyword, &source[offset + (line.len() - from.len())..])
            {
                missing.push(format!("{file}:{}", at + 1));
            }
            offset += line.len();
        }
        missing
    }

    #[test]
    fn every_classifier_loop_steps_the_counter() {
        let missing: Vec<String> = SOURCES
            .iter()
            .flat_map(|(file, source)| unstepped(file, source))
            .collect();
        assert_eq!(
            missing,
            Vec::<String>::new(),
            "start these loops' bodies with `work::step();` (see work.rs)"
        );
        // The check finds loops of each form.
        let loops = "for x in y {\n    work::step();\n}\nwhile a\n    && b\n{\n    f();\n}\n\
                     'outer: loop {\n}\n// for x in y {\nlet f = |x| x;\n\
                     for P { a } in s.split('{').chain(['\\'', '\\u{7d}']) {\n    work::step();\n}\n\
                     while s == \"{\" {}\nfor P { a } in x {\n    f(a);\n}\n";
        assert_eq!(
            unstepped("f.rs", loops),
            vec!["f.rs:4", "f.rs:9", "f.rs:16", "f.rs:17"]
        );
    }

    #[test]
    fn work_counts_this_thread_only() {
        let other_thread = 1 << 20;
        let counted = super::measure(|| {
            for _ in 0..3 {
                super::step();
            }
            drop(vec![0u8; 1000]);
            std::thread::spawn(move || {
                super::step();
                drop(vec![0u8; other_thread]);
            })
            .join()
            .unwrap();
        });
        assert_eq!(counted.steps, 3);
        assert!(
            (1000..other_thread as u64).contains(&counted.bytes),
            "{counted:?}"
        );
    }
}
