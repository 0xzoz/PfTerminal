use super::*;
use codex_security_policy::SecurityLevel;
use pretty_assertions::assert_eq;

fn shell(script: &str) -> Vec<String> {
    vec!["/bin/zsh".into(), "-lc".into(), script.into()]
}

/// The word the command used for the first denied path it names, through
/// quoting, redirections, `--flag=` values and home variables.
#[test]
fn issue_442_finds_the_denied_path_the_command_names() {
    let cwd = Path::new("/work/project");
    let user_home = Path::new("/Users/someone");
    let codex_home = Path::new("/Users/someone/.corbanu");
    let denied = |path: &Path| {
        [
            Path::new("/work/project/.env"),
            Path::new("/Users/someone/.ssh"),
            Path::new("/Users/someone/.corbanu/auth.json"),
        ]
        .iter()
        .any(|denied| path.starts_with(denied))
    };
    let word = |script: &str| denied_word(&shell(script), cwd, Some(user_home), codex_home, denied);
    assert_eq!(word("cat .env"), Some(".env".to_string()));
    assert_eq!(word("head -3 '.env' | sort"), Some(".env".to_string()));
    assert_eq!(word("wc -l <.env;"), Some(".env".to_string()));
    assert_eq!(word("tool --file=.env"), Some(".env".to_string()));
    assert_eq!(word("cat ~/.ssh/config"), Some("~/.ssh/config".to_string()));
    assert_eq!(
        word("cat $HOME/.ssh/id_ed25519"),
        Some("$HOME/.ssh/id_ed25519".to_string())
    );
    assert_eq!(
        word("cat \"$CORBANU_HOME/auth.json\""),
        Some("$CORBANU_HOME/auth.json".to_string())
    );
    assert_eq!(word("cat notes.txt"), None);
    assert_eq!(word("cat $UNKNOWN/.env.example"), None);
}

/// #442 (milestone AGG-09): under Aggressive a denied read of a protected
/// file is explained where it failed, naming the level and /security; it is
/// not explained under Permissive, for a command that succeeded, or for a
/// failure that is not a denial.
#[tokio::test]
async fn issue_442_denied_protected_read_is_explained_under_aggressive() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let home = turn.config.codex_home.to_path_buf();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), "x").unwrap();
    let command = shell(&format!("cat '{}'", home.join("auth.json").display()));
    let denial = "cat: auth.json: Operation not permitted\n";
    let explain = |exit_code, output: &str| {
        denial_explanation(
            &session,
            &turn,
            &command,
            cwd.as_path(),
            Some(exit_code),
            output,
        )
    };
    assert_eq!(explain(1, denial), None, "Permissive: unchanged");

    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(SecurityLevel::Aggressive)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
    let explanation = explain(1, denial).expect("explained under Aggressive");
    assert!(
        explanation.starts_with("Security level Aggressive blocked reading"),
        "{explanation}"
    );
    assert!(
        explanation.contains("cannot be granted, not even once"),
        "{explanation}"
    );
    assert!(
        explanation.contains("switch to Permissive in /security"),
        "{explanation}"
    );
    assert_eq!(explain(0, denial), None);
    assert_eq!(
        explain(1, "cat: auth.json: No such file or directory\n"),
        None
    );
}
