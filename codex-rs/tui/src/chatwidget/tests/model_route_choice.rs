//! #653: `/model` offers each connection of a model served by several.

use super::*;
use codex_model_provider_info::ZAI_ANTHROPIC_PROVIDER_ID;
use codex_model_provider_info::ZAI_PROVIDER_ID;
use pretty_assertions::assert_eq;

const FLASH: &str = "glm-5.3-flash";
const OPENAI_STYLE: &str = "GLM 5.3 Flash · Z.AI (OpenAI-style)";
const CLAUDE_STYLE: &str = "GLM 5.3 Flash · Z.AI (Claude-style)";

async fn zai_session(
    provider: &str,
) -> (
    ChatWidget,
    tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    tokio::sync::mpsc::UnboundedReceiver<Op>,
) {
    let (mut chat, rx, op_rx) = make_chatwidget_manual(Some(FLASH)).await;
    chat.config.model_provider_id = provider.to_string();
    chat.thread_id = Some(ThreadId::new());
    (chat, rx, op_rx)
}

fn open_all_models(chat: &mut ChatWidget) -> String {
    let presets = chat.model_catalog.try_list_models().expect("catalog");
    chat.open_all_models_popup(presets);
    render_bottom_popup_with_height(chat, /*width*/ 110, /*height*/ 36)
}

/// Selects the row named `label` in the open tab and returns the preset the
/// picker sends to the effort step.
fn select_row(
    chat: &mut ChatWidget,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    label: &str,
) -> ModelPreset {
    for _ in 0..12 {
        let popup = render_bottom_popup_with_height(chat, /*width*/ 110, /*height*/ 36);
        if popup
            .lines()
            .any(|line| line.contains('›') && line.contains(label))
        {
            chat.handle_key_event(KeyEvent::from(KeyCode::Enter));
            return std::iter::from_fn(|| rx.try_recv().ok())
                .find_map(|event| match event {
                    AppEvent::OpenReasoningPopup { model, .. } => Some(model),
                    _ => None,
                })
                .expect("a model row opens the effort step");
        }
        chat.handle_key_event(KeyEvent::from(KeyCode::Down));
    }
    panic!("no row {label}");
}

fn chosen(events: &[AppEvent]) -> (Option<String>, Option<String>) {
    let update = events.iter().find_map(|event| match event {
        AppEvent::UpdateModelSelection { model, provider } if model == FLASH => provider.clone(),
        _ => None,
    });
    let persist = events.iter().find_map(|event| match event {
        AppEvent::PersistModelSelection {
            model, provider, ..
        } if model == FLASH => provider.clone(),
        _ => None,
    });
    (update, persist)
}

#[tokio::test]
async fn zai_tab_lists_both_connections_openai_style_first() {
    let (mut chat, _rx, _op_rx) = zai_session(ZAI_PROVIDER_ID).await;
    let popup = open_all_models(&mut chat);
    let openai = popup.find(OPENAI_STYLE).expect(OPENAI_STYLE);
    let claude = popup.find(CLAUDE_STYLE).expect(CLAUDE_STYLE);
    assert!(openai < claude, "{popup}");
    assert!(
        popup.contains(&format!("{OPENAI_STYLE} (current)")),
        "{popup}"
    );
    assert_eq!(popup.matches("(current)").count(), 1, "{popup}");
    assert!(!popup.contains("via zai-anthropic"), "{popup}");
    assert_chatwidget_snapshot!("model_route_choice_zai_tab", popup);
}

#[tokio::test]
async fn a_claude_style_session_opens_on_its_row_in_the_zai_tab() {
    let (mut chat, _rx, _op_rx) = zai_session(ZAI_ANTHROPIC_PROVIDER_ID).await;
    // The runtime row the app adds for the session's connection is folded in.
    crate::model_catalog::ModelCatalog::sync_runtime_models(
        &chat.model_catalog,
        [ZAI_ANTHROPIC_PROVIDER_ID],
        Some(FLASH),
        ZAI_ANTHROPIC_PROVIDER_ID,
    );
    let popup = open_all_models(&mut chat);
    assert!(popup.contains("[Z.AI]"), "{popup}");
    assert!(
        !popup.contains("[Other]") && !popup.contains("Other"),
        "{popup}"
    );
    assert!(
        popup.contains(&format!("{CLAUDE_STYLE} (current)")),
        "{popup}"
    );
    assert_eq!(popup.matches("(current)").count(), 1, "{popup}");
    assert_eq!(popup.matches(CLAUDE_STYLE).count(), 1, "{popup}");
    assert_chatwidget_snapshot!("model_route_choice_claude_style_current", popup);
}

#[tokio::test]
async fn picking_a_connection_switches_and_saves_that_provider_both_ways() {
    for (from, label, to) in [
        (ZAI_PROVIDER_ID, CLAUDE_STYLE, ZAI_ANTHROPIC_PROVIDER_ID),
        (ZAI_ANTHROPIC_PROVIDER_ID, OPENAI_STYLE, ZAI_PROVIDER_ID),
    ] {
        let (mut chat, mut rx, _op_rx) = zai_session(from).await;
        open_all_models(&mut chat);
        let preset = select_row(&mut chat, &mut rx, label);
        assert_eq!(preset.model, FLASH);
        assert_eq!(preset.provider_id.as_deref(), Some(to));

        chat.open_reasoning_popup(preset);
        let effort = render_bottom_popup(&chat, /*width*/ 100);
        assert!(effort.contains(label), "{effort}");
        // Another connection of the session's model is not the current choice.
        assert!(!effort.contains("(current)"), "{effort}");
        chat.handle_key_event(KeyEvent::from(KeyCode::Enter));
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            chosen(&events),
            (Some(to.to_string()), Some(to.to_string())),
            "{from} -> {to}"
        );
    }
}

#[tokio::test]
async fn only_available_connections_are_offered() {
    use crate::provider_status_host::ProviderAccountMetadata;
    use crate::provider_status_host::ProviderStatusHost;
    use codex_provider_auth::ProviderActivationPolicy;
    use codex_provider_auth::ProviderRuntimeAuthorizations;

    let (mut chat, _rx, _op_rx) = zai_session(ZAI_PROVIDER_ID).await;
    let home = tempfile::tempdir().unwrap();
    let mut config = crate::legacy_core::config::ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .build()
        .await
        .unwrap();
    config.cli_auth_credentials_store_mode = codex_login::AuthCredentialsStoreMode::File;
    config.model_provider_id = ZAI_PROVIDER_ID.to_string();
    codex_login::login_with_provider_api_key(
        home.path(),
        "ZAI_API_KEY",
        "synthetic-route-choice-key",
        config.cli_auth_credentials_store_mode,
        config.auth_keyring_backend_kind(),
    )
    .unwrap();
    let host = ProviderStatusHost::from_config(&config, ProviderAccountMetadata::default());
    chat.model_catalog.set_provider_policy(
        super::super::provider_model_policy::ProviderModelPolicy::new(
            host.clone(),
            ProviderRuntimeAuthorizations::default(),
        ),
    );
    let both = open_all_models(&mut chat);
    assert!(
        both.contains(OPENAI_STYLE) && both.contains(CLAUDE_STYLE),
        "{both}"
    );

    // Both connections use the Z.AI key, so /providers turns them off together.
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    host.persist_policy(ZAI_PROVIDER_ID, ProviderActivationPolicy::Inactive)
        .unwrap();
    chat.model_catalog.refresh_provider_policy();
    let none = open_all_models(&mut chat);
    assert!(
        !none.contains("OpenAI-style") && !none.contains("Claude-style"),
        "{none}"
    );

    // Nor is one this session has no provider entry for.
    chat.handle_key_event(KeyEvent::from(KeyCode::Esc));
    host.persist_policy(ZAI_PROVIDER_ID, ProviderActivationPolicy::Active)
        .unwrap();
    chat.model_catalog.refresh_provider_policy();
    chat.config
        .model_providers
        .remove(ZAI_ANTHROPIC_PROVIDER_ID);
    let unconfigured = open_all_models(&mut chat);
    assert!(
        unconfigured.contains("Z.AI GLM 5.3 Flash (current)"),
        "{unconfigured}"
    );
    assert!(!unconfigured.contains("-style"), "{unconfigured}");
}

/// PF-84 accounts are per provider: connections on different accounts say so.
#[tokio::test]
async fn connections_on_different_named_accounts_name_them() {
    let (mut chat, _rx, _op_rx) = zai_session(ZAI_PROVIDER_ID).await;
    chat.config
        .model_providers
        .get_mut(ZAI_PROVIDER_ID)
        .expect("zai")
        .account = Some(codex_model_provider_info::NamedProviderAccount {
        provider_id: ZAI_PROVIDER_ID.to_string(),
        name: "work".to_string(),
    });
    let popup = open_all_models(&mut chat);
    assert!(
        popup.contains(&format!("{OPENAI_STYLE} · account work")),
        "{popup}"
    );
    assert!(
        popup.contains(&format!("{CLAUDE_STYLE} · default account")),
        "{popup}"
    );
}

/// Switching connection keeps the session's effort, even an advanced one.
#[tokio::test]
async fn switching_connection_keeps_the_sessions_max_effort() {
    let (mut chat, mut rx, _op_rx) = zai_session(ZAI_PROVIDER_ID).await;
    chat.set_reasoning_effort(Some(ReasoningEffortConfig::Max));
    open_all_models(&mut chat);
    let preset = select_row(&mut chat, &mut rx, CLAUDE_STYLE);
    chat.open_reasoning_popup(preset);
    let effort = render_bottom_popup(&chat, /*width*/ 100);
    assert!(
        effort
            .lines()
            .any(|line| line.contains('›') && line.contains("More reasoning")),
        "{effort}"
    );
    chat.handle_key_event(KeyEvent::from(KeyCode::Enter));
    let advanced = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|event| match event {
            AppEvent::OpenAdvancedReasoningPopup { model, .. } => Some(model),
            _ => None,
        })
        .expect("the advanced step opens");
    chat.open_advanced_reasoning_popup(advanced);
    let popup = render_bottom_popup(&chat, /*width*/ 100);
    assert!(!popup.contains("(current)"), "{popup}");
    chat.handle_key_event(KeyEvent::from(KeyCode::Enter));
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(
        chosen(&events),
        (
            Some(ZAI_ANTHROPIC_PROVIDER_ID.to_string()),
            Some(ZAI_ANTHROPIC_PROVIDER_ID.to_string())
        )
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::PersistModelSelection {
                effort: Some(ReasoningEffortConfig::Max),
                ..
            }
        )),
        "{events:?}"
    );
}
