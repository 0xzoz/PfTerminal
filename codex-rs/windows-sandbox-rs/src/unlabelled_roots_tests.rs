use super::*;
use pretty_assertions::assert_eq;

#[test]
fn a_failed_root_is_listed_until_it_is_labelled() {
    let root = PathBuf::from(r"Z:\unlabelled_roots_tests\a");
    record(&root, /*labelled*/ false);
    assert!(unlabelled_writable_roots().contains(&root));
    record(&root, /*labelled*/ true);
    assert!(!unlabelled_writable_roots().contains(&root));
}

#[test]
fn labelling_an_unknown_root_lists_nothing() {
    let root = PathBuf::from(r"Z:\unlabelled_roots_tests\b");
    record(&root, /*labelled*/ true);
    assert!(!unlabelled_writable_roots().contains(&root));
}

#[test]
fn warning_names_the_root_the_reason_and_both_fixes() {
    let root = PathBuf::from(r"D:\shared\repo");
    assert_eq!(
        unlabelled_writable_root_warning(&root),
        format!(
            "`{}` is read-only to commands in the non-admin Windows sandbox, although it is a \
             writable folder: that sandbox runs commands at Low integrity, and the folder could \
             not be labelled Low. Labelling needs permission to change the folder's owner \
             settings (which owning it gives) on a file system that holds integrity labels, such \
             as NTFS. To let commands write there, own the folder, or switch to the default \
             sandbox by running /setup-default-sandbox, or set `sandbox = \"elevated\"` under \
             `[windows]` in config.toml.",
            root.display()
        )
    );
}
