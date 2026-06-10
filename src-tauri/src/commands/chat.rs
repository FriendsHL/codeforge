use serde::Serialize;
use tauri::ipc::Channel;

use crate::config;
use crate::llm::anthropic;
use crate::llm::types::{ChatMessage, LlmEvent};

/// 推送给前端的统一事件协议（M2 起会扩展 ToolCallStart / PermissionAsk 等）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        stop_reason: Option<String>,
        output_tokens: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

#[tauri::command]
pub async fn send_message(
    model: String,
    messages: Vec<ChatMessage>,
    channel: Channel<AgentEvent>,
) -> Result<(), String> {
    let api_key = config::get_api_key()?
        .ok_or("尚未设置 API key，请先在设置中填写")?;

    let result = anthropic::stream_chat(&api_key, &model, &messages, |event| {
        let agent_event = match event {
            LlmEvent::TextDelta(text) => AgentEvent::TextDelta { text },
            LlmEvent::TurnEnd { stop_reason, output_tokens } => {
                AgentEvent::TurnEnd { stop_reason, output_tokens }
            }
        };
        let _ = channel.send(agent_event);
    })
    .await;

    if let Err(message) = &result {
        let _ = channel.send(AgentEvent::Error { message: message.clone() });
    }
    result
}
