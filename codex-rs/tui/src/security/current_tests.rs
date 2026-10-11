use std::collections::HashMap;

use codex_config::LoaderOverrides;
use codex_config::types::ShellEnvironmentPolicyToml;
use codex_protocol::config_types::EnvironmentVariablePattern;
use pretty_assertions::assert_eq;

use super::*;
use crate::legacy_core::config::ConfigBuilder;
use crate::legacy_core::config::ConfigOverrides;
use crate::security::aggressive;

async fn build(home: &Path, cwd: &Path, user_config: &str, aggressive_level: bool) -> Config {
    std::fs::write(home.join("config.toml"), user_config).unwrap();
    let mut cli = Vec::new();
    let mut harness = ConfigOverrides {
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    };
    if aggressive_level {
        cli = aggressive::base_overrides(home, home);
        cli.extend(aggressive::env_overrides(
            &ShellEnvironmentPolicyToml::default(),
        ));
        aggressive::apply_launch_overrides(&mut harness);
    }
    ConfigBuilder::default()
        .codex_home(home.to_path_buf())
        .cli_overrides(cli)
        .harness_overrides(harness)
        .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await
        .unwrap()
}

/// A legacy workspace-write sandbox without temp folders, with an extra
/// writable root and the given network setting.
fn workspace_write(extra: &Path, network: bool) -> String {
    format!(
        r#"approval_policy = "on-request"
sandbox_mode = "workspace-write"
web_search = "live"

[sandbox_workspace_write]
writable_roots = [{extra:?}]
network_access = {network}
exclude_tmpdir_env_var = true
exclude_slash_tmp = true
"#,
        extra = extra.display().to_string()
    )
}

/// Code-blind case B-06: the review shows what the user would lose. The
/// legacy sandbox has no denied reads, so approved commands can leave it,
/// and its "off" network and vault rows say so.
#[tokio::test]
async fn workspace_write_rows_name_the_escape_route() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    let extra_path = extra.path().display().to_string();

    let open = build(
        home.path(),
        cwd.path(),
        &workspace_write(extra.path(), true),
        false,
    )
    .await;
    let [sandbox, approvals, network, vault, _, _] = current_values(&open);
    assert_eq!(
        [sandbox, approvals, network, vault],
        [
            format!(
                "commands can write to the current folder, {extra_path}; approved or allow-listed commands can run outside the sandbox; permission-request tools are off"
            ),
            "on-request (reviewer: you)".to_string(),
            "on for agent commands; web search live".to_string(),
            "the vault store and sign-in file are readable to agent commands; secret-like environment variables are passed through (KEY, SECRET, TOKEN, VAULT, PASSWORD, PASSPHRASE, CREDENTIAL); login profiles or shell snapshots are used".to_string(),
        ]
    );

    let closed = build(
        home.path(),
        cwd.path(),
        &workspace_write(extra.path(), false),
        false,
    )
    .await;
    let [_, _, network, _, _, _] = current_values(&closed);
    assert_eq!(
        network,
        "off inside the sandbox, on for commands that run outside it; web search live"
    );
}

/// Under Aggressive every row matches the Aggressive value.
#[tokio::test]
async fn aggressive_rows_claim_only_what_is_enforced() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    let config = build(
        home.path(),
        cwd.path(),
        &workspace_write(extra.path(), true),
        true,
    )
    .await;
    // #391: the stored level the launch path reads turns the model key
    // broker on; these overrides alone do not (see the next test).
    let [sandbox, approvals, network, vault, _, children] = current_values(&config);
    assert_eq!(
        [sandbox, approvals, network, vault, children],
        [
            format!(
                "profile {}: commands can write to the current folder; permission-request tools are off",
                aggressive::PROFILE_ID
            ),
            "untrusted (reviewer: you)".to_string(),
            "off for agent commands; web search disabled".to_string(),
            "the vault store and sign-in file are unreadable to agent commands; secret-like environment variables are removed; login profiles and shell snapshots are not used".to_string(),
            "spawned agents get this session's values".to_string(),
        ]
    );
}

/// #391: the model keys row reports whether the broker holds them.
#[tokio::test]
async fn model_keys_row_reports_the_broker_setting() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let mut config = build(home.path(), cwd.path(), "", false).await;
    assert_eq!(
        current_values(&config)[4],
        "Corbanu reads provider keys itself (broker_model_auth off)"
    );
    // Enabled after loading, so this test process is not marked brokered.
    config
        .features
        .enable(Feature::BrokerModelAuth)
        .expect("enable the broker");
    assert_eq!(
        current_values(&config)[4],
        "held by the isolated credential broker (broker_model_auth on)"
    );
}

/// #440 (milestone AGG-15): with the secretless launch on, ordinary
/// variables outside its allowlist (`PLAIN_VAR`) are removed too, so the
/// review says the environment is allowlisted, not only that secret-like
/// variables are removed.
#[tokio::test]
async fn issue_440_vault_row_says_the_environment_is_allowlisted_under_secretless_launch() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    let mut config = build(
        home.path(),
        cwd.path(),
        &workspace_write(extra.path(), true),
        true,
    )
    .await;
    // Enabled after loading, so this test process is not armed.
    config
        .features
        .enable(Feature::SecretlessAgentLaunch)
        .expect("enable the secretless launch");
    let vault = &current_values(&config)[3];
    assert!(
        vault.contains("only allowlisted environment variables reach agent commands"),
        "{vault}"
    );
    // What the launch does: an ordinary name outside the allowlist goes.
    let mut env = std::collections::HashMap::from([
        ("PLAIN_VAR".to_string(), "visible".to_string()),
        ("PATH".to_string(), "/bin".to_string()),
    ]);
    codex_protocol::secretless_launch::retain_launch_env(&mut env, |_| false);
    assert_eq!(env.keys().collect::<Vec<_>>(), vec!["PATH"]);
    let vault_row = aggressive::ROWS
        .iter()
        .find(|(label, _)| *label == "Vault")
        .expect("vault row");
    assert!(
        vault_row
            .1
            .contains("so is every other variable outside an allowlist")
    );
}

/// Names the user lists explicitly are reported, not just the fixed probes.
#[test]
fn secret_kinds_include_named_variables_from_the_environment() {
    let env = || {
        [
            ("PATH", "/bin"),
            ("OPENAI_API_KEY", "fake"),
            ("STRIPE_SECRET", "fake"),
        ]
        .map(|(name, value)| (name.to_string(), value.to_string()))
    };
    let include_only = ShellEnvironmentPolicy {
        include_only: vec![EnvironmentVariablePattern::new_case_insensitive(
            "OPENAI_API_KEY",
        )],
        ..Default::default()
    };
    // Aggressive's excludes: the built-in KEY/SECRET/TOKEN ones plus these.
    let exclude = ["*VAULT*", "*PASSWORD*", "*PASSPHRASE*", "*CREDENTIAL*"]
        .map(EnvironmentVariablePattern::new_case_insensitive)
        .to_vec();
    let removed = ShellEnvironmentPolicy {
        ignore_default_excludes: false,
        exclude: exclude.clone(),
        ..Default::default()
    };
    let set = ShellEnvironmentPolicy {
        ignore_default_excludes: false,
        exclude,
        r#set: HashMap::from([("DB_PASSWORD".to_string(), "fake".to_string())]),
        ..Default::default()
    };
    assert_eq!(
        [
            kept_secret_kinds(&include_only, env()),
            kept_secret_kinds(&removed, env()),
            kept_secret_kinds(&set, env()),
        ],
        [vec!["KEY"], Vec::new(), vec!["PASSWORD"]]
    );
}

/// A role counts as changing its children's values unless it sets only
/// model and instruction keys.
#[test]
fn roles_that_set_permission_keys_change_children() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, contents: &str| {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    };
    assert_eq!(
        [
            role_changes_values(&write(
                "model.toml",
                "model = \"m\"\ndeveloper_instructions = \"x\"\n"
            )),
            role_changes_values(&write(
                "network.toml",
                "[sandbox_workspace_write]\nnetwork_access = true\n"
            )),
            role_changes_values(&write("broken.toml", "model = \n")),
            role_changes_values(&dir.path().join("missing.toml")),
        ],
        [false, true, true, true]
    );
}

/// Denied reads keep commands inside the sandbox, but an approved permission
/// request can still add network or reads, so those rows say so.
#[tokio::test]
async fn permission_requests_qualify_network_and_vault_rows() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let secrets = home.path().join("secrets").display().to_string();
    let auth = home.path().join("auth.json").display().to_string();
    let user_config = |request_tools: bool| {
        format!(
            r#"approval_policy = "on-request"
default_permissions = "locked"

[permissions.locked]
extends = ":workspace"

[permissions.locked.filesystem]
":tmpdir" = "read"
":slash_tmp" = "read"
{secrets:?} = "deny"
{auth:?} = "deny"

[permissions.locked.network]
enabled = false

[features]
request_permissions_tool = {request_tools}
"#
        )
    };
    let [_, _, network, vault, _, _] =
        current_values(&build(home.path(), cwd.path(), &user_config(true), false).await);
    assert_eq!(
        [network, vault.split("; ").next().unwrap().to_string()],
        [
            "off unless an approved permission request turns it on; web search cached".to_string(),
            "the vault store and sign-in file are unreadable (unless an approved permission request grants access) to agent commands".to_string(),
        ]
    );
    let [sandbox, _, network, _, _, _] =
        current_values(&build(home.path(), cwd.path(), &user_config(false), false).await);
    assert_eq!(
        [sandbox, network],
        [
            "profile locked: commands can write to the current folder; permission-request tools are off".to_string(),
            "off for agent commands; web search cached".to_string(),
        ]
    );
}

/// #437 (AGG-24/34, AGG-20 child pane): after a session's first turn the app
/// server sends its thread settings, which carry only the legacy sandbox
/// projection. Rebuilding the profile from it dropped the Aggressive denied
/// reads, so the rows claimed approved commands leave the sandbox and the
/// vault is readable. The known profile is kept while nothing changed.
#[tokio::test]
async fn sec_437_thread_settings_keep_the_aggressive_rows() {
    use crate::legacy_core::config::PermissionProfileSnapshot;

    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    let mut config = build(
        home.path(),
        cwd.path(),
        &workspace_write(extra.path(), true),
        true,
    )
    .await;
    let before = current_values(&config);
    let profile = config.permissions.permission_profile().clone();
    let active = config.permissions.active_permission_profile();
    let cwd_abs = config.cwd.clone();
    let settings = codex_app_server_protocol::ThreadSettings {
        cwd: cwd_abs.clone(),
        approval_policy: codex_app_server_protocol::AskForApproval::UnlessTrusted,
        approvals_reviewer: codex_app_server_protocol::ApprovalsReviewer::User,
        sandbox_policy: codex_sandboxing::compatibility_sandbox_policy_for_permission_profile(
            &config.permissions.effective_permission_profile(),
            cwd_abs.as_path(),
        )
        .into(),
        active_permission_profile: active.clone().map(Into::into),
        model: "glm-5.3-flash".to_string(),
        model_provider: "zai".to_string(),
        service_tier: None,
        effort: None,
        summary: None,
        collaboration_mode: codex_protocol::config_types::CollaborationMode {
            mode: codex_protocol::config_types::ModeKind::Default,
            settings: codex_protocol::config_types::Settings {
                model: "glm-5.3-flash".to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        },
        multi_agent_mode: Default::default(),
        personality: None,
    };

    // The milestone's rows: rebuilt from the projection alone.
    let lossy = codex_protocol::models::PermissionProfile::from_legacy_sandbox_policy_for_cwd(
        &settings.sandbox_policy.to_core(),
        cwd_abs.as_path(),
    );
    let mut weakened = config.clone();
    weakened
        .permissions
        .replace_permission_profile_from_session_snapshot(
            PermissionProfileSnapshot::from_session_snapshot(lossy, active.clone()),
        )
        .unwrap();
    let rows = current_values(&weakened);
    assert!(
        rows[0].contains("approved or allow-listed commands can run outside the sandbox"),
        "{rows:?}"
    );

    let kept = crate::app::thread_settings::permission_profile_for_thread_settings(
        &profile,
        Some(&config.permissions.effective_permission_profile()),
        active.as_ref(),
        &settings,
    );
    assert_eq!(kept, profile);
    config
        .permissions
        .replace_permission_profile_from_session_snapshot(
            PermissionProfileSnapshot::from_session_snapshot(kept, active.clone()),
        )
        .unwrap();
    assert_eq!(current_values(&config), before);

    // A real change (a different sandbox) is still applied.
    let mut changed = settings;
    changed.sandbox_policy = codex_app_server_protocol::SandboxPolicy::DangerFullAccess;
    assert_eq!(
        crate::app::thread_settings::permission_profile_for_thread_settings(
            &profile,
            None,
            active.as_ref(),
            &changed,
        ),
        codex_protocol::models::PermissionProfile::Disabled
    );
}
