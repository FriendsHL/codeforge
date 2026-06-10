use serde::Serialize;
use tauri::ipc::Channel;

use crate::llm::registry::{self, Endpoint};
use crate::llm::types::{ChatMessage, LlmEvent};
use crate::llm::{anthropic, openai};

/// 推送给前端的统一事件协议（M2 起会扩展 ToolCallStart / PermissionAsk 等）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    #[serde(rename_all = "camelCase")]
    ReasoningDelta { text: String },
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        stop_reason: Option<String>,
        output_tokens: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

impl From<LlmEvent> for AgentEvent {
    fn from(event: LlmEvent) -> Self {
        match event {
            LlmEvent::TextDelta(text) => AgentEvent::TextDelta { text },
            LlmEvent::ReasoningDelta(text) => AgentEvent::ReasoningDelta { text },
            LlmEvent::TurnEnd { stop_reason, output_tokens } => {
                AgentEvent::TurnEnd { stop_reason, output_tokens }
            }
        }
    }
}

#[tauri::command]
pub async fn send_message(
    provider: String,
    model: String,
    messages: Vec<ChatMessage>,
    channel: Channel<AgentEvent>,
) -> Result<(), String> {
    let endpoint = registry::resolve(&provider)?;
    let api_key = registry::api_key_for(&endpoint)?;

    let on_event = |event: LlmEvent| {
        let _ = channel.send(event.into());
    };

    let result = match &endpoint {
        Endpoint::Anthropic => {
            anthropic::stream_chat(&api_key, &model, &messages, on_event).await
        }
        Endpoint::OpenAiCompatible { chat_url, .. } => {
            openai::stream_chat(chat_url, &api_key, &model, &messages, on_event).await
        }
    };

    if let Err(message) = &result {
        let _ = channel.send(AgentEvent::Error { message: message.clone() });
    }
    result
}
