//! #1901: deliberately disguised loads of zsh/stat keep D410's table-writer
//! checks enabled. These are the item-3 follow-ups from #1845 that were not
//! covered by the original #1840 regression test.

use super::ProtectedActionKind::UnseenCode;
use super::kind;
use pretty_assertions::assert_eq;

#[test]
fn issue1901_disguised_zsh_stat_loads_keep_table_checks_enabled() {
    for sh in ["bash", "zsh"] {
        let line = |script: &str| kind(&[sh, "-lc", script]);

        // This load does not itself write a code table, but makes the later
        // `stat` command a possible zsh/stat table writer.
        let glob_alternation = "zmod(l|)oad zsh/st(a|)t";
        assert_eq!(line(glob_alternation), None, "{sh}: {glob_alternation}");
        let glob_then_stat = format!("{glob_alternation}; stat -A functions x");
        assert_eq!(
            line(&glob_then_stat),
            Some(UnseenCode),
            "{sh}: {glob_then_stat}"
        );

        // A pattern-named command inside the function substitution keeps it
        // fail-closed, with or without a later zsh/stat table write.
        let function_substitution = "value=${ zmod(l|)oad zsh/st(a|)t; }";
        assert_eq!(
            line(function_substitution),
            Some(UnseenCode),
            "{sh}: {function_substitution}"
        );
        let substitution_then_stat = format!("{function_substitution}; stat -A functions x");
        assert_eq!(
            line(&substitution_then_stat),
            Some(UnseenCode),
            "{sh}: {substitution_then_stat}"
        );

        // zstyle -e values are deferred code and stay fail-closed, including
        // when the hidden load precedes a zsh/stat table write.
        let deferred_load =
            "zstyle -e :x y 'zmod(l|)oad zsh/st(a|)t; reply=(ok)'; zstyle -s :x y value";
        assert_eq!(
            line(deferred_load),
            Some(UnseenCode),
            "{sh}: {deferred_load}"
        );
        let deferred_then_stat = format!("{deferred_load}; stat -A functions x");
        assert_eq!(
            line(&deferred_then_stat),
            Some(UnseenCode),
            "{sh}: {deferred_then_stat}"
        );
    }
}

#[test]
fn issue1901_safe_controls_stay_quiet() {
    for sh in ["bash", "zsh"] {
        let line = |script: &str| kind(&[sh, "-lc", script]);

        for script in [
            "stat -A functions x",
            "print -r -- ok",
            "zstyle :x y ok; zstyle -s :x y value; print -r -- $value",
        ] {
            assert_eq!(line(script), None, "{sh}: {script}");
        }
    }
}
