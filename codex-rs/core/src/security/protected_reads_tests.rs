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

/// #554: `find ~` or `ls -aR ~` reports the home's listing denial below the
/// folder the command names; the line is attributed to the home as it
/// spells it, in BSD and GNU forms and relative to the folder a `cd` names.
#[test]
fn issue_554_recursive_listing_of_a_parent_is_attributed_to_the_home() {
    let cwd = Path::new("/work/project");
    let user_home = Path::new("/users/me");
    let codex_home = Path::new("/users/me/.corbanu");
    let unlistable = |path: &Path| {
        AbsolutePathBuf::from_absolute_path(path).is_ok_and(|path| path.as_path() == codex_home)
    };
    let found = |script: &str, line: &str| {
        unlistable_path_in_lines(
            &shell(script),
            cwd,
            Some(user_home),
            codex_home,
            &[line.to_string()],
            &unlistable,
        )
    };
    for (script, line, expected) in [
        (
            "find ~ -name '*.toml'",
            "find: /users/me/.corbanu: Operation not permitted",
            "/users/me/.corbanu",
        ),
        (
            "ls -aR \"$HOME\"",
            "ls: /users/me/.corbanu: Operation not permitted",
            "/users/me/.corbanu",
        ),
        (
            "find /users/me",
            "find: \u{2018}/users/me/.corbanu\u{2019}: Permission denied",
            "/users/me/.corbanu",
        ),
        (
            "ls -aR /users/me",
            "ls: cannot open directory '/users/me/.corbanu': Permission denied",
            "/users/me/.corbanu",
        ),
        (
            "cd ~ && find .",
            "find: ./.corbanu: Operation not permitted",
            "./.corbanu",
        ),
        (
            "cd ~; find .",
            "find: ./.corbanu: Operation not permitted",
            "./.corbanu",
        ),
        (
            "ls -aR ../../users/me",
            "ls: ../../users/me/.corbanu: Operation not permitted",
            "../../users/me/.corbanu",
        ),
    ] {
        assert_eq!(found(script, line), Some(expected.to_string()), "{script}");
    }
    for (script, line) in [
        // Another folder, or a name that only starts like the home's.
        (
            "find ~",
            "find: /users/me/Library/Mail: Operation not permitted",
        ),
        (
            "find ~",
            "find: /users/me/.corbanu-old: Operation not permitted",
        ),
        // A path below the home, not the home itself.
        (
            "find ~",
            "find: /users/me/.corbanu/vault: Operation not permitted",
        ),
        // Relative to a folder the command does not name.
        ("find .", "find: ./.corbanu: Operation not permitted"),
    ] {
        assert_eq!(found(script, line), None, "{script}: {line}");
    }
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

/// #554: under Aggressive, `find` of the folder above the Corbanu home is
/// explained like a direct listing of the home (#506); not under Permissive,
/// and not for a denial of another folder.
#[tokio::test]
async fn issue_554_find_above_the_home_is_explained_under_aggressive() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let home = turn.config.codex_home.to_path_buf();
    std::fs::create_dir_all(&home).unwrap();
    let parent = home.parent().expect("the home has a parent").to_path_buf();
    let command = shell(&format!("find '{}' -name '*.toml'", parent.display()));
    let denial = format!("find: {}: Operation not permitted\n", home.display());
    let explain = async |output: &str| {
        denial_explanation(&session, &turn, &command, cwd.as_path(), Some(1), output).await
    };
    assert_eq!(explain(&denial).await, None, "Permissive: unchanged");

    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(SecurityLevel::Aggressive)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
    let explanation = explain(&denial).await.expect("explained under Aggressive");
    assert_eq!(
        explanation,
        listing_explanation(&home.display().to_string())
    );
    let other = format!(
        "find: {}: Operation not permitted\n",
        parent.join("elsewhere").display()
    );
    assert_eq!(explain(&other).await, None);
}

/// #621 (milestone AGG-R07): a path inside code an interpreter runs is found
/// like a path argument, so no grant is offered for it; code that names no
/// protected path is left alone.
#[test]
fn issue_621_finds_the_denied_path_inside_interpreter_code() {
    let cwd = Path::new("/work/project");
    let user_home = Path::new("/Users/someone");
    let codex_home = Path::new("/Users/someone/.corbanu");
    let denied = |path: &Path| {
        AbsolutePathBuf::from_absolute_path(path).is_ok_and(|path| {
            path.as_path().starts_with(codex_home)
                || path.as_path() == Path::new("/work/project/.env")
        })
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
    for (script, expected) in [
        (
            "python3 -c \"import os; print(os.listdir('/Users/someone/.corbanu'))\"",
            "/Users/someone/.corbanu",
        ),
        (
            "python -c 'print(open(\"/Users/someone/.corbanu/auth.json\").read())'",
            "/Users/someone/.corbanu/auth.json",
        ),
        (
            "node -e \"console.log(require('fs').readdirSync('$CORBANU_HOME'))\"",
            "$CORBANU_HOME",
        ),
        (
            "perl -e 'opendir(D, \"~/.corbanu\") or die; print readdir(D)'",
            "~/.corbanu",
        ),
        (
            "ruby -e 'puts Dir.entries(\"/Users/someone/.corbanu\")'",
            "/Users/someone/.corbanu",
        ),
        ("python3 -c \"print(open('.env').read())\"", ".env"),
    ] {
        assert_eq!(word(script), Some(expected.to_string()), "{script}");
    }
    for script in [
        "python3 -c \"print(1 + 2)\"",
        "python3 -c \"import os; print(os.listdir('/work/project'))\"",
        "node -e \"console.log('hello, world')\"",
    ] {
        assert_eq!(word(script), None, "{script}");
    }
}

/// An Aggressive session (with the source labels it turns on).
fn aggressive(session: &Session) {
    let client = (*session.services.model_client())
        .clone()
        .with_ingress_level(SecurityLevel::Aggressive)
        .with_source_envelopes(true);
    session.services.replace_model_client(client);
}

/// The recent denials the inspector shows for `session` (not the
/// process-wide ones other tests in this process may have recorded).
fn recent_denials(session: &Session, turn: &TurnContext) -> Vec<(&'static str, &'static str)> {
    crate::security::inspection::observe(
        turn.config.codex_home.as_path(),
        SecurityLevel::Aggressive,
        Some(session.thread_id()),
        crate::security::aggressive::now_unix_seconds(),
    )
    .denials
    .into_iter()
    .filter(|denial| denial.thread == Some(session.thread_id()))
    .map(|denial| (denial.reason, denial.outcome))
    .collect()
}

/// #621 and #622: under Aggressive, a protected read or home listing an
/// interpreter makes is explained where it failed, also when the command
/// built the path at run time (the denial line names it), and it is listed
/// under the inspector's recent denials. The approval prompt for the listing
/// offers no grant (`protected_note_in_command`).
#[tokio::test]
async fn issue_621_622_interpreter_denials_are_explained_and_listed() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let home = turn.config.codex_home.to_path_buf();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("auth.json"), "x").unwrap();
    let listing = shell(&format!(
        "python3 -c \"import os; print(os.listdir('{}'))\"",
        home.display()
    ));
    let listing_denied = format!(
        "Traceback (most recent call last):\n  File \"<string>\", line 1, in <module>\n\
         PermissionError: [Errno 1] Operation not permitted: '{}'\n",
        home.display()
    );
    // The path is built at run time: no word names it.
    let built = shell("python3 -c \"import os; open(os.environ['P'] + '/auth.json').read()\"");
    let built_denied = format!(
        "PermissionError: [Errno 1] Operation not permitted: '{}'\n",
        home.join("auth.json").display()
    );
    let explain = async |command: &[String], output: &str| {
        denial_explanation(&session, &turn, command, cwd.as_path(), Some(1), output).await
    };
    assert_eq!(explain(&listing, &listing_denied).await, None, "Permissive");
    assert_eq!(recent_denials(&session, &turn), Vec::new());

    aggressive(&session);
    assert_eq!(
        explain(&listing, &listing_denied).await,
        Some(listing_explanation(&home.display().to_string()))
    );
    assert_eq!(
        explain(&built, &built_denied).await,
        Some(explanation(&home.join("auth.json").display().to_string()))
    );
    let note = protected_note_in_command(&session, &turn, &listing, cwd.as_path())
        .await
        .expect("the listing is recognised before it runs");
    assert!(
        note.explanation()
            .contains("cannot be granted, not even once"),
        "{}",
        note.explanation()
    );
    let outcome = "denied by the sandbox; never granted under Aggressive";
    assert_eq!(
        recent_denials(&session, &turn),
        vec![
            ("reading a protected file", outcome),
            ("listing the Corbanu home", outcome),
        ]
    );
    // A failure that is not a denial of a protected path is not listed.
    assert_eq!(
        explain(
            &built,
            "PermissionError: [Errno 13] Permission denied: '/elsewhere/file'\n"
        )
        .await,
        None
    );
    assert_eq!(recent_denials(&session, &turn).len(), 2);
}

/// #620 (milestone AGG-18, AGG-R17): under Aggressive a start-up file
/// outside the current folder (`~/.zshrc`) can never be written, not even
/// with Grant once, so the approval prompt offers no grant and says why;
/// a denied write is explained the same way where it failed. A write in the
/// current folder, and Permissive sessions, are left alone.
#[tokio::test]
async fn issue_620_startup_file_outside_the_folder_is_never_granted() {
    let (session, turn) = crate::session::tests::make_session_and_context().await;
    #[allow(deprecated)]
    let cwd = turn.cwd.clone();
    let user_home = dirs::home_dir().expect("a home folder");
    let append = shell("echo '# grant-once-1' >> ~/.zshrc");
    let denied = format!(
        "zsh:1: operation not permitted: {}\n",
        user_home.join(".zshrc").display()
    );
    let note = async |command: &[String]| {
        protected_note_in_command(&session, &turn, command, cwd.as_path())
            .await
            .map(|note| note.explanation())
    };
    assert_eq!(note(&append).await, None, "Permissive");

    aggressive(&session);
    let explained = note(&append).await.expect("no grant for ~/.zshrc");
    assert_eq!(explained, startup_explanation("~/.zshrc"));
    assert!(
        explained.contains("cannot be granted, not even once"),
        "{explained}"
    );
    assert!(explained.contains("Permissive in /security"), "{explained}");
    assert_eq!(
        note(&shell("echo '# x' >> \"$HOME/.bash_profile\"")).await,
        Some(startup_explanation("$HOME/.bash_profile"))
    );
    assert_eq!(note(&shell("echo '# x' >> notes.txt")).await, None);
    assert_eq!(
        denial_explanation(&session, &turn, &append, cwd.as_path(), Some(1), &denied).await,
        Some(startup_explanation("~/.zshrc"))
    );
    assert_eq!(
        recent_denials(&session, &turn),
        vec![(
            "writing a start-up file outside the current folder",
            "denied by the sandbox; never granted under Aggressive"
        )]
    );
}
