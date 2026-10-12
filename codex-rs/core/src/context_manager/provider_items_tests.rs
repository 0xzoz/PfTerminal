use super::*;
use codex_protocol::ResponseItemId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemContent;
use pretty_assertions::assert_eq;
use serde_json::json;

fn id(value: &str) -> Option<ResponseItemId> {
    Some(ResponseItemId::from_server(value.to_string()))
}

fn from(producer: Option<&str>, mut item: ResponseItem) -> ResponseItem {
    item.set_turn_id_if_missing("turn-1");
    if let Some(producer) = producer {
        item.set_producer(producer);
    }
    item
}

/// Reasoning as each wire's adapter records it.
fn responses_reasoning(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::Reasoning {
            id: id("rs_openai"),
            summary: Vec::new(),
            content: None,
            encrypted_content: Some("sealed".into()),
            anthropic_content_block: None,
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn chat_reasoning(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::Reasoning {
            id: id("rs_2026101213421902b19754973e467e"),
            summary: Vec::new(),
            content: Some(vec![ReasoningItemContent::ReasoningText {
                text: "glm thinking".into(),
            }]),
            encrypted_content: None,
            anthropic_content_block: None,
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn anthropic_reasoning(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::Reasoning {
            id: id("rs_msg_01"),
            summary: Vec::new(),
            content: Some(vec![ReasoningItemContent::ReasoningText {
                text: "claude thinking".into(),
            }]),
            encrypted_content: None,
            anthropic_content_block: Some(
                json!({"type": "thinking", "thinking": "claude thinking", "signature": "sig"}),
            ),
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn anthropic_web_search(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::WebSearchCall {
            id: id("srvtoolu_1"),
            status: Some("completed".into()),
            action: None,
            anthropic_content_block: Some(json!({"type": "server_tool_use", "id": "srvtoolu_1"})),
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn assistant(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::Message {
            id: id("msg_chatcmpl-1"),
            role: "assistant".into(),
            content: vec![ContentItem::OutputText {
                text: "answer".into(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn call(producer: Option<&str>) -> ResponseItem {
    from(
        producer,
        ResponseItem::FunctionCall {
            id: id("fc_call_-1"),
            name: "shell".into(),
            namespace: None,
            arguments: "{}".into(),
            encrypted_function_args: Some(vec!["sealed".into()]),
            call_id: "call_-1".into(),
            internal_chat_message_metadata_passthrough: None,
        },
    )
}

fn target(producer: &str, wire: WireApi, stores_items: bool) -> ProviderTarget {
    ProviderTarget {
        producer: producer.into(),
        wire,
        stores_items,
    }
}

fn adapted(items: Vec<ResponseItem>, target: &ProviderTarget) -> Vec<ResponseItem> {
    let mut items = items;
    adapt_for_provider(&mut items, target);
    items
}

fn without_id(mut item: ResponseItem) -> ResponseItem {
    item.set_id(None);
    item
}

#[test]
fn same_provider_items_are_unchanged_on_every_wire() {
    for (producer, wire, items) in [
        (
            "openai",
            WireApi::Responses,
            vec![responses_reasoning(Some("openai")), call(Some("openai"))],
        ),
        (
            "zai",
            WireApi::Chat,
            vec![chat_reasoning(Some("zai")), assistant(Some("zai"))],
        ),
        (
            "anthropic",
            WireApi::Anthropic,
            vec![
                anthropic_reasoning(Some("anthropic")),
                anthropic_web_search(Some("anthropic")),
            ],
        ),
    ] {
        let target = target(producer, wire, /*stores_items*/ false);
        assert_eq!(adapted(items.clone(), &target), items, "{producer}");
    }
}

/// The issue: GLM (Chat) reasoning reached OpenAI as `rs_…` references.
#[test]
fn chat_and_anthropic_items_reach_responses_inline_without_references() {
    let openai = target("openai", WireApi::Responses, /*stores_items*/ false);
    let items = vec![
        responses_reasoning(Some("openai")),
        chat_reasoning(Some("zai")),
        assistant(Some("zai")),
        call(Some("zai")),
        anthropic_reasoning(Some("anthropic")),
        anthropic_web_search(Some("anthropic")),
        assistant(Some("anthropic")),
    ];
    let mut expected_call = without_id(call(Some("zai")));
    if let ResponseItem::FunctionCall {
        encrypted_function_args,
        ..
    } = &mut expected_call
    {
        *encrypted_function_args = None;
    }
    assert_eq!(
        adapted(items, &openai),
        vec![
            responses_reasoning(Some("openai")),
            without_id(assistant(Some("zai"))),
            expected_call,
            without_id(assistant(Some("anthropic"))),
        ]
    );
}

/// Another Responses provider (or account): its encrypted reasoning is bound to it.
#[test]
fn another_responses_providers_reasoning_is_dropped() {
    let work = target(
        "openai:work",
        WireApi::Responses,
        /*stores_items*/ false,
    );
    assert_eq!(
        adapted(
            vec![
                responses_reasoning(Some("openai")),
                assistant(Some("openai"))
            ],
            &work
        ),
        vec![without_id(assistant(Some("openai")))]
    );
}

#[test]
fn another_anthropic_hosts_signed_blocks_are_dropped() {
    let anthropic = target("anthropic", WireApi::Anthropic, /*stores_items*/ false);
    let items = vec![
        anthropic_reasoning(Some("zai-anthropic")),
        anthropic_web_search(Some("zai-anthropic")),
        assistant(Some("zai-anthropic")),
        call(Some("openai")),
        responses_reasoning(Some("openai")),
    ];
    assert_eq!(
        adapted(items, &anthropic),
        vec![assistant(Some("zai-anthropic")), call(Some("openai"))]
    );
}

#[test]
fn chat_requests_keep_every_item() {
    let zai = target("zai", WireApi::Chat, /*stores_items*/ false);
    let items = vec![
        responses_reasoning(Some("openai")),
        anthropic_reasoning(Some("anthropic")),
        call(Some("openai")),
    ];
    assert_eq!(adapted(items.clone(), &zai), items);
}

/// Sessions recorded before #578: only what a stateless Responses host
/// could read as a reference, or Anthropic-native blocks, are dropped.
#[test]
fn unattributed_items_keep_only_what_the_target_can_read() {
    let items = vec![
        responses_reasoning(None),
        chat_reasoning(None),
        anthropic_reasoning(None),
        anthropic_web_search(None),
        assistant(None),
        call(None),
    ];
    assert_eq!(
        adapted(
            items.clone(),
            &target("openai", WireApi::Responses, /*stores_items*/ false)
        ),
        vec![responses_reasoning(None), assistant(None), call(None)]
    );
    assert_eq!(
        adapted(
            items.clone(),
            &target("azure", WireApi::Responses, /*stores_items*/ true)
        ),
        vec![
            responses_reasoning(None),
            chat_reasoning(None),
            assistant(None),
            call(None)
        ]
    );
    for wire in [WireApi::Chat, WireApi::Anthropic] {
        let target = target("other", wire, /*stores_items*/ false);
        assert_eq!(adapted(items.clone(), &target), items);
    }
}
