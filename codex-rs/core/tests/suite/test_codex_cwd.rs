//! sec-745 follow-up: the config `TestCodex` builds is not built for the
//! test process's own working folder (the crate's source tree, e.g.
//! `codex-rs/core`). The folder a config is built for is its workspace root,
//! so sandboxed commands in tests could write into the checkout.

use codex_utils_absolute_path::AbsolutePathBuf;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_codex_config_does_not_make_the_source_tree_a_workspace_root() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let test = test_codex().build(&server).await?;
    let source_tree = AbsolutePathBuf::current_dir()?;
    let roots = test.config.permissions.workspace_roots();
    assert!(
        !roots
            .iter()
            .any(|root| source_tree.as_path().starts_with(root.as_path())),
        "{source_tree:?} is under a workspace root: {roots:?}"
    );
    Ok(())
}
