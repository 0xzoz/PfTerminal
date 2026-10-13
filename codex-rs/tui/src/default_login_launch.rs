//! PF-84-S06: whether a TUI launch leaves the default OpenAI login in place
//! or enforces the login restrictions on it at startup.

use crate::legacy_core::config::Config;
use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_client::RemoteAppServerClient;
use codex_app_server_client::RemoteAppServerConnectArgs;
use codex_app_server_client::RemoteAppServerEndpoint;
use codex_login::enforce_login_restrictions;

use crate::AppServerTarget;

/// PF-84-S06: what a launch does with the default OpenAI login.
pub(crate) struct DefaultLoginAtLaunch {
    /// The app server the launch uses: a reused local daemon is replaced by
    /// the embedded one when the daemon would use a ruled-out default login.
    pub(crate) app_server_target: AppServerTarget,
    /// The default login was left in place; the caller records it with
    /// `codex_login::leave_default_login_unenforced` before any auth manager
    /// is built.
    pub(crate) left_in_place: bool,
}

/// PF-84-S06 (#570, #595): a session on a named sign-in account checks the
/// restrictions against that account at thread start and never reads or
/// logs out the default login. A resumed or forked thread's account is known
/// only later, so those launches leave the default login in place too. The
/// app server then never uses a ruled-out default login and refuses a thread
/// on it with the reason: the embedded one because this process records it,
/// a reused local daemon because it does the same with named accounts on and
/// says so at initialize. A daemon that does not (an older build, named
/// accounts off when it started, or other login restrictions) is not reused;
/// the launch starts its own app server instead. A remote server decides for
/// itself. Every other launch enforces at startup as before.
pub(crate) async fn settle_default_login_at_launch(
    config: &Config,
    launches_resume_or_fork: bool,
    app_server_target: AppServerTarget,
) -> std::io::Result<DefaultLoginAtLaunch> {
    let leaves_default_login = !app_server_target.uses_remote_workspace()
        && (config.runs_on_named_sign_in_account()
            || (config.named_accounts_enabled() && launches_resume_or_fork));
    if !leaves_default_login {
        if !app_server_target.uses_remote_workspace() {
            enforce_login_restrictions(&config.default_login_restrictions()).await?;
        }
        return Ok(DefaultLoginAtLaunch {
            app_server_target,
            left_in_place: false,
        });
    }
    let app_server_target = match app_server_target {
        AppServerTarget::LocalDaemon { endpoint }
            if !daemon_leaves_ruled_out_default_login(&endpoint, config).await =>
        {
            tracing::info!(
                "not reusing the local app-server daemon: it would use a default login this \
                 session's login restrictions rule out; starting an embedded app server"
            );
            AppServerTarget::Embedded
        }
        target => target,
    };
    Ok(DefaultLoginAtLaunch {
        app_server_target,
        left_in_place: true,
    })
}

/// PF-84-S06 (#595): whether the local daemon at `endpoint` leaves a
/// ruled-out default login in place and refuses to use it, under the same
/// login restrictions as this launch.
async fn daemon_leaves_ruled_out_default_login(
    endpoint: &RemoteAppServerEndpoint,
    config: &Config,
) -> bool {
    let client = match RemoteAppServerClient::connect(RemoteAppServerConnectArgs {
        endpoint: endpoint.clone(),
        client_name: "codex-tui".to_string(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
        experimental_api: true,
        mcp_server_openai_form_elicitation: false,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    {
        Ok(client) => client,
        Err(err) => {
            tracing::debug!(%err, "could not ask the local app-server daemon about the default login");
            return false;
        }
    };
    let leaves = client.ruled_out_default_login().is_some_and(|policy| {
        policy.forced_login_method == config.configured_forced_login_method
            && policy.forced_chatgpt_workspace_id == config.forced_chatgpt_workspace_id
    });
    if let Err(err) = client.shutdown().await {
        tracing::debug!(%err, "closing the local app-server daemon probe");
    }
    leaves
}

#[cfg(test)]
#[path = "default_login_launch_tests.rs"]
mod tests;
