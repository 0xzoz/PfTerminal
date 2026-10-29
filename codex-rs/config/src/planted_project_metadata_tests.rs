use super::*;
use pretty_assertions::assert_eq;

#[test]
fn planted_paths_match_canonically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dot_codex = dir.path().join(".codex");
    std::fs::create_dir(&dot_codex).expect("create .codex");
    std::fs::create_dir(dir.path().join("sub")).expect("create sub");
    let planted = vec![dunce::canonicalize(&dot_codex).expect("canonical")];
    let other = dir.path().join(".agents");
    assert_eq!(
        (
            is_planted_project_metadata(&dot_codex, &planted),
            is_planted_project_metadata(
                &dir.path().join("sub").join("..").join(".codex"),
                &planted
            ),
            is_planted_project_metadata(&other, &planted),
            is_planted_project_metadata(&dot_codex, &[]),
        ),
        (true, true, false, false)
    );
}
