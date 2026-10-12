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
    let word = |script: &str| {
        denied_word(
            &shell(script),
            cwd,
            Some(user_home),
            codex_home,
            denied,
            None,
        )
    };
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
    // Tied to the denial the output reports: an unrelated failure in a
    // command that also names `.env` is not attributed to it.
    let lines = ["grep: ./other.txt: Permission denied".to_string()];
    assert_eq!(
        denied_word(
            &shell("grep -r x . --exclude=.env"),
            cwd,
            Some(user_home),
            codex_home,
            denied,
            Some(&lines),
        ),
        None
    );
    let lines = ["cat: .env: Operation not permitted".to_string()];
    assert_eq!(
        denied_word(
            &shell("cat .env"),
            cwd,
            Some(user_home),
            codex_home,
            denied,
            Some(&lines)
        ),
        Some(".env".to_string())
    );
}

/// #506 review: a denial line names the folder as the shell resolved it, so
/// `ls "$CODEX_HOME"` and `ls home/tmp/..` are attributed to the home; the
/// listing explanation names the path as the command spelled it.
#[test]
fn issue_506_listing_denial_is_attributed_through_variables_and_dot_dot() {
    let cwd = Path::new("/work/project");
    let user_home = Path::new("/users/me");
    let codex_home = Path::new("/users/me/.corbanu");
    let denied = |path: &Path| {
        AbsolutePathBuf::from_absolute_path(path).is_ok_and(|path| path.as_path() == codex_home)
    };
    let lines = ["ls: /users/me/.corbanu: Operation not permitted".to_string()];
    for (script, expected) in [
        ("ls -la \"$CODEX_HOME\"", "$CODEX_HOME"),
        ("ls ${CORBANU_HOME}", "${CORBANU_HOME}"),
        ("ls ~/.corbanu/tmp/..", "~/.corbanu/tmp/.."),
    ] {
        assert_eq!(
            denied_word(
                &shell(script),
                cwd,
                Some(user_home),
                codex_home,
                denied,
                Some(&lines),
            ),
            Some(expected.to_string()),
            "{script}"
        );
    }
    let other = ["ls: /users/me/other: Operation not permitted".to_string()];
    assert_eq!(
        denied_word(
            &shell("ls \"$CODEX_HOME\" other"),
            cwd,
            Some(user_home),
            codex_home,
            denied,
            Some(&other),
        ),
        None
    );
    let text = listing_explanation("$CODEX_HOME");
    assert!(
        text.starts_with("Security level Aggressive blocked listing `$CODEX_HOME`"),
        "{text}"
    );
    assert!(text.contains("not even once"), "{text}");
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
    let explain = async |exit_code, output: &str| {
        denial_explanation(
            &session,
            &turn,
            &command,
            cwd.as_path(),
            Some(exit_code),
            output,
        )
        .await
    };
    assert_eq!(explain(1, denial).await, None, "Permissive: unchanged");

    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(SecurityLevel::Aggressive)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
    let explanation = explain(1, denial)
        .await
        .expect("explained under Aggressive");
    assert!(
        explanation.starts_with("Security level Aggressive blocked access to"),
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
    assert_eq!(explain(0, denial).await, None);
    assert_eq!(
        explain(1, "cat: auth.json: No such file or directory\n").await,
        None
    );
    assert_eq!(
        protected_path_in_command(&session, &turn, &command, cwd.as_path()).await,
        Some(home.join("auth.json").display().to_string())
    );
}
