//! #1901: deliberately disguised loads of zsh/stat keep D410's table-writer
//! checks enabled. These are the item-3 follow-ups from #1845 that were not
//! covered by the original #1840 regression test.

use super::ProtectedActionKind::UnseenCode;
use super::kind;
use pretty_assertions::assert_eq;

#[test]
fn issue1901_disguised_zsh_stat_loads_keep_table_checks_enabled() {
    let line = |script: &str| kind(&["zsh", "-lc", script]);

    // This load does not itself write a code table, but makes the later
    // `stat` command a possible zsh/stat table writer.
    let glob_alternation = "zmod(l|)oad zsh/st(a|)t";
    assert_eq!(line(glob_alternation), None, "{glob_alternation}");
    let glob_then_stat = format!("{glob_alternation}; stat -A functions x");
    assert_eq!(line(&glob_then_stat), Some(UnseenCode), "{glob_then_stat}");

    // Deferred code and ksh/bash 5.3 function substitution are already
    // fail-closed because the lexer cannot see what they will run or return.
    // The hidden loads must not weaken that broader protection.
    for script in [
        "value=${ zmod(l|)oad zsh/st(a|)t; }",
        "zstyle -e :x y 'zmod(l|)oad zsh/st(a|)t; reply=(ok)'; zstyle -s :x y value",
    ] {
        assert_eq!(line(script), Some(UnseenCode), "{script}");
    }
}

#[test]
fn issue1901_safe_controls_stay_quiet() {
    let line = |script: &str| kind(&["zsh", "-lc", script]);

    for script in [
        "stat -A functions x",
        "print -r -- ok",
        "zstyle :x y ok; zstyle -s :x y value; print -r -- $value",
    ] {
        assert_eq!(line(script), None, "{script}");
    }
}
