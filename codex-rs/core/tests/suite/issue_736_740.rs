//! Issues #736, #737 and #740 (third "Aggressive ships" run: AGG-10,
//! AGG-T03, AGG-11, AGG-T04), end to end through the orchestrator and the
//! real sandbox (Seatbelt on macOS, bubblewrap on Linux):
//! - #736 (macOS): `security find-generic-password` against a disposable
//!   keychain (never the login keychain) finds its item outside the
//!   sandbox, but under Aggressive the approved command cannot reach the
//!   keychain service. The prompt and the result say so; no grant is
//!   offered. #758: the same when an organization's requirements enforce a
//!   managed network proxy.
//! - #737: `git config --global` writes `~/.gitconfig`, a start-up file
//!   outside the current folder: no grant, with the #620 explanation.
//! - #740: a write outside the current folder, or to the project's `.git`,
//!   is offered no grant either, and the prompt says why.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::path::Path;
use std::sync::Arc;

use codex_core::config::Constrained;
use codex_core::security_grant;
use codex_core::security_inspection;
use codex_features::Feature;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::user_input::UserInput;
use codex_security_policy::SecurityLevel;
use core_test_support::managed_network_requirements_loader;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::skip_if_sandbox;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::json;
use tempfile::TempDir;

const NEVER: &str = "cannot be granted, not even once";
const DENIED: &str = "denied by the sandbox; never granted under Aggressive";

/// What one approved command showed.
struct Run {
    /// Whether Core offered a grant on its approval prompt.
    offered: bool,
    /// The approval prompt's reason.
    reason: String,
    /// What the model got back.
    output: String,
    /// Warnings shown to the person.
    warnings: Vec<String>,
    /// The inspector's recent denials for the session (reason, outcome).
    denials: Vec<(&'static str, &'static str)>,
}

/// Runs `command` in an Aggressive session whose current folder holds a
/// `.git/hooks` folder, approves it once (y, no grant) and reports what
/// happened. Agent commands get `shell_env` set.
async fn run(command: &str, shell_env: &[(&str, &Path)]) -> anyhow::Result<Run> {
    run_with(command, shell_env, /*managed_network*/ false).await
}

/// [`run`], with `managed_network`: an organization's requirements enforce
/// a managed network proxy (#758), so the sandbox takes its restricted
/// network rules.
async fn run_with(
    command: &str,
    shell_env: &[(&str, &Path)],
    managed_network: bool,
) -> anyhow::Result<Run> {
    let shell_env: Vec<(String, String)> = shell_env
        .iter()
        .map(|(name, path)| (name.to_string(), path.display().to_string()))
        .collect();
    let server = start_mock_server().await;
    let captured = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("call-0"),
                ev_function_call(
                    "call-0",
                    "exec_command",
                    &json!({ "cmd": command }).to_string(),
                ),
                ev_completed("call-0"),
            ]),
            sse(vec![
                ev_response_created("done"),
                ev_assistant_message("msg-done", "done"),
                ev_completed("done"),
            ]),
        ],
    )
    .await;
    let mut builder = test_codex();
    if managed_network {
        builder = builder.with_cloud_config_bundle(managed_network_requirements_loader());
    }
    let test = builder
        .with_home(Arc::new(TempDir::new()?))
        .with_model("test-gpt-5.1-codex")
        .with_workspace_setup(|cwd, _fs| async move {
            std::fs::create_dir_all(cwd.as_path().join(".git/hooks"))?;
            Ok(())
        })
        .with_config(move |config| {
            for (name, value) in shell_env {
                config
                    .permissions
                    .shell_environment_policy
                    .r#set
                    .insert(name, value);
            }
            config.security_level = SecurityLevel::Aggressive;
            config.permissions.approval_policy =
                Constrained::allow_any(AskForApproval::UnlessTrusted);
            // Aggressive's own sandbox: writes only in the current folder
            // (named, as the workspace roots of a real session name it; the
            // harness's are elsewhere), not in /tmp or $TMPDIR either;
            // network off.
            let folder = config.cwd.clone();
            config
                .permissions
                .set_permission_profile(PermissionProfile::workspace_write_with(
                    &[folder],
                    NetworkSandboxPolicy::Restricted,
                    /*exclude_tmpdir_env_var*/ true,
                    /*exclude_slash_tmp*/ true,
                ))
                .expect("workspace write");
            config
                .features
                .enable(Feature::SourceEnvelopes)
                .expect("enable source envelopes");
            config.use_experimental_unified_exec_tool = true;
            config
                .features
                .enable(Feature::UnifiedExec)
                .expect("enable unified exec");
        })
        // Host-sandbox security test: local executor on purpose, also in the
        // remote lane (#708).
        .build(&server)
        .await?;
    std::mem::forget(server);
    assert_eq!(
        test.config.permissions.network.is_some(),
        managed_network,
        "managed network proxy"
    );
    let thread = test.session_configured.thread_id;
    test.codex
        .submit(Op::UserInput {
            items: vec![UserInput::Text {
                text: "run the command".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            additional_context: Default::default(),
            thread_settings: Default::default(),
        })
        .await?;
    let EventMsg::ExecApprovalRequest(approval) = wait_for_event(&test.codex, |event| {
        matches!(
            event,
            EventMsg::ExecApprovalRequest(_) | EventMsg::TurnComplete(_)
        )
    })
    .await
    else {
        anyhow::bail!("the command did not ask first: {command}");
    };
    let offered = security_grant::offer(thread, &approval.effective_approval_id()).is_some();
    test.codex
        .submit(Op::ExecApproval {
            id: approval.effective_approval_id(),
            turn_id: None,
            decision: ReviewDecision::Approved,
        })
        .await?;
    let mut warnings = Vec::new();
    while let EventMsg::Warning(warning) = wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_) | EventMsg::Warning(_))
    })
    .await
    {
        warnings.push(warning.message);
    }
    let requests = captured.requests();
    let last = requests.last().expect("the model was asked again");
    let denials = security_inspection::observe(
        test.codex_home_path(),
        SecurityLevel::Aggressive,
        Some(thread),
        i64::MAX / 2,
    )
    .denials
    .into_iter()
    .map(|denial| (denial.reason, denial.outcome))
    .collect();
    Ok(Run {
        offered,
        reason: approval.reason.unwrap_or_default(),
        output: last.function_call_output_text("call-0").unwrap_or_default(),
        warnings,
        denials,
    })
}

/// A disposable keychain file, made and deleted with `HOME` and the
/// preferences folder (`CFFIXED_USER_HOME`) pointed at its own folder, so
/// the search list and login keychain of the person running the test are
/// never touched; every query names the file.
#[cfg(target_os = "macos")]
struct Keychain {
    folder: TempDir,
    path: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
impl Keychain {
    const PASSWORD: &str = "disposable-736";

    fn security(folder: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("/usr/bin/security")
            .args(args)
            .env("HOME", folder)
            .env("CFFIXED_USER_HOME", folder)
            .output()
            .expect("run /usr/bin/security")
    }

    fn create(service: &str, secret: &str) -> anyhow::Result<Self> {
        let folder = TempDir::new()?;
        let path = folder.path().join("probe-736.keychain-db");
        let keychain = path.to_str().expect("UTF-8 path").to_string();
        for args in [
            vec!["create-keychain", "-p", Self::PASSWORD, keychain.as_str()],
            vec!["unlock-keychain", "-p", Self::PASSWORD, keychain.as_str()],
            vec![
                "add-generic-password",
                "-A",
                "-s",
                service,
                "-a",
                "agg-736",
                "-w",
                secret,
                keychain.as_str(),
            ],
        ] {
            let output = Self::security(folder.path(), &args);
            anyhow::ensure!(
                output.status.success(),
                "security {}: {}",
                args[0],
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(Self { folder, path })
    }

    /// `security find-generic-password -w` for `service` in this keychain.
    fn find_command(&self, service: &str) -> String {
        format!(
            "security find-generic-password -s {service} -w '{}'",
            self.path.display()
        )
    }
}

#[cfg(target_os = "macos")]
impl Drop for Keychain {
    fn drop(&mut self) {
        let keychain = self.path.to_str().unwrap_or_default();
        let _ = Self::security(self.folder.path(), &["delete-keychain", keychain]);
    }
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_736_keychain_is_out_of_reach_and_explained() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    keychain_is_out_of_reach_and_explained(/*managed_network*/ false).await
}

/// #758: also when an organization's requirements enforce a managed network
/// proxy, whose restricted network rules allow `com.apple.SecurityServer`:
/// Aggressive denies the keychain services after them (Travis, 2026-10-14).
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_758_keychain_is_out_of_reach_with_a_managed_network_proxy() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    keychain_is_out_of_reach_and_explained(/*managed_network*/ true).await
}

#[cfg(target_os = "macos")]
async fn keychain_is_out_of_reach_and_explained(managed_network: bool) -> anyhow::Result<()> {
    const SERVICE: &str = "corbanu-probe-736";
    const SECRET: &str = "keychain-canary-736";
    let keychain = Keychain::create(SERVICE, SECRET)?;
    // Outside any sandbox the item is there: the run below is not vacuous.
    let control = Keychain::security(
        keychain.folder.path(),
        &[
            "find-generic-password",
            "-s",
            SERVICE,
            "-w",
            keychain.path.to_str().expect("UTF-8 path"),
        ],
    );
    assert_eq!(
        String::from_utf8_lossy(&control.stdout).trim(),
        SECRET,
        "{}",
        String::from_utf8_lossy(&control.stderr)
    );

    let folder = keychain.folder.path();
    let run = run_with(
        &keychain.find_command(SERVICE),
        &[("HOME", folder), ("CFFIXED_USER_HOME", folder)],
        managed_network,
    )
    .await?;
    // The boundary: the approved command does not get the item.
    assert!(!run.output.contains(SECRET), "{}", run.output);
    // What the person is told, before and after.
    assert!(!run.offered, "no grant for a keychain command");
    assert!(
        run.reason
            .contains("Security level Aggressive keeps the macOS keychain out of reach"),
        "{}",
        run.reason
    );
    assert!(run.reason.contains(NEVER), "{}", run.reason);
    let warning = run
        .warnings
        .iter()
        .find(|warning| {
            warning.starts_with("Security level Aggressive kept the macOS keychain out of reach")
        })
        .unwrap_or_else(|| panic!("not explained: {:?}", run.warnings));
    assert!(warning.contains(NEVER), "{warning}");
    assert!(run.output.contains(warning.as_str()), "{}", run.output);
    assert!(
        run.denials.contains(&("using the macOS keychain", DENIED)),
        "{:?}",
        run.denials
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_737_git_config_global_is_offered_no_grant() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    // The command's `~` is a folder of its own (Core still names the file
    // by the real home folder): no git config of the person running the
    // test is touched, whatever happens.
    let shell_home = TempDir::new()?;
    let run = run(
        "git config --global core.hooksPath /tmp/t03-hooks",
        &[("HOME", shell_home.path())],
    )
    .await?;
    assert!(!shell_home.path().join(".gitconfig").exists());
    assert!(!run.offered, "no grant for ~/.gitconfig");
    let explained = "Security level Aggressive blocks writing `~/.gitconfig`: it is, or leads \
                     to, a start-up file outside the current folder";
    assert!(run.reason.contains(explained), "{}", run.reason);
    assert!(run.reason.contains(NEVER), "{}", run.reason);
    assert!(
        run.warnings
            .iter()
            .any(|warning| warning.starts_with(explained)),
        "{:?} {}",
        run.warnings,
        run.output
    );
    assert!(
        run.denials
            .contains(&("writing a start-up file outside the current folder", DENIED)),
        "{:?}",
        run.denials
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_740_no_grant_where_it_cannot_apply() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));
    skip_if_sandbox!(Ok(()));
    let outside = format!("../outside-740-{}.txt", std::process::id());
    for (command, explained, denial) in [
        (
            format!("echo t04 >> {outside}"),
            format!(
                "Security level Aggressive blocks writing `{outside}`: it is outside the current folder"
            ),
            "writing outside the current folder",
        ),
        (
            "printf '#!/bin/sh\\n' > .git/hooks/pre-commit".to_string(),
            "Security level Aggressive blocks writing `.git/hooks/pre-commit`: agent commands \
             cannot write it, even when you approve them"
                .to_string(),
            "writing a read-only part of the current folder",
        ),
    ] {
        let run = run(&command, &[]).await?;
        assert!(!run.offered, "{command}: no grant");
        assert!(run.reason.contains(&explained), "{command}: {}", run.reason);
        assert!(run.reason.contains(NEVER), "{command}: {}", run.reason);
        assert!(
            run.warnings
                .iter()
                .any(|warning| warning.starts_with(&explained)),
            "{command}: {:?} {}",
            run.warnings,
            run.output
        );
        assert!(
            run.denials.contains(&(denial, DENIED)),
            "{command}: {:?}",
            run.denials
        );
    }
    // A command whose text shows every write, none of them liftable (a
    // link in the current folder, then a read; AGG-T06): no grant, and why.
    let run = run("ln -s notes.txt notes-link.txt && cat notes-link.txt", &[]).await?;
    assert!(!run.offered, "nothing to lift: no grant");
    assert!(
        run.reason
            .contains("Grant once is not offered for this command"),
        "{}",
        run.reason
    );
    Ok(())
}
