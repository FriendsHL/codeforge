//! Anthropic Messages API 流式客户端，支持 tool use

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{AssistantTurn, HistoryItem, LlmDelta, ToolCall, ToolSpec};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 64000;

#[derive(Debug, Deserialize)]
struct SseData {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<ContentBlock>,
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(rename = "type", default)]
    delta_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
}

fn to_wire_messages(history: &[HistoryItem]) -> Vec<Value> {
    let mut messages = Vec::new();
    for item in history {
        match item {
            HistoryItem::User(text) => {
                messages.push(json!({"role": "user", "content": text}));
            }
            HistoryItem::Assistant { text, tool_calls } => {
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    let input: Value =
                        serde_json::from_str(&call.arguments).unwrap_or(json!({}));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": input,
                    }));
                }
                messages.push(json!({"role": "assistant", "content": blocks}));
            }
            HistoryItem::ToolResult { call_id, content, is_error, .. } => {
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                        "is_error": is_error,
                    }],
                }));
            }
        }
    }
    messages
}

/// 流式状态机：按 content block index 累积 text / tool_use
#[derive(Debug, Default)]
struct BlockAccumulator {
    /// (index, ToolCall)，arguments 由 input_json_delta 拼出
    tool_blocks: Vec<(usize, ToolCall)>,
}

impl BlockAccumulator {
    fn start_tool(&mut self, index: usize, id: String, name: String) {
        self.tool_blocks.push((
            index,
            ToolCall { id, name, arguments: String::new() },
        ));
    }

    fn append_json(&mut self, index: usize, fragment: &str) {
        if let Some((_, call)) = self.tool_blocks.iter_mut().find(|(i, _)| *i == index) {
            call.arguments.push_str(fragment);
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.tool_blocks
            .into_iter()
            .map(|(_, mut call)| {
                if call.arguments.is_empty() {
                    call.arguments = "{}".into();
                }
                call
            })
            .collect()
    }
}

/// 调用 Anthropic Messages API（流式），返回完整的 assistant 轮次
pub async fn stream_chat(
    api_key: &str,
    model: &str,
    system: Option<&str>,
    history: &[HistoryItem],
    tools: &[ToolSpec],
    mut on_delta: impl FnMut(LlmDelta),
) -> Result<AssistantTurn, String> {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "messages": to_wire_messages(history),
    });
    if let Some(system) = system {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect(),
        );
    }

    let response = reqwest::Client::new()
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        return Err(format!("API 错误 ({status}): {message}"));
    }

    let mut turn = AssistantTurn::default();
    let mut accumulator = BlockAccumulator::default();

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("流读取失败: {e}"))?;
        let data: SseData =
            serde_json::from_str(&event.data).map_err(|e| format!("响应解析失败: {e}"))?;

        match data.event_type.as_str() {
            "content_block_start" => {
                if let (Some(index), Some(block)) = (data.index, data.content_block) {
                    if block.block_type == "tool_use" {
                        accumulator.start_tool(
                            index,
                            block.id.unwrap_or_default(),
                            block.name.unwrap_or_default(),
                        );
                    }
                }
            }
            "content_block_delta" => {
                if let (Some(index), Some(delta)) = (data.index, data.delta) {
                    match delta.delta_type.as_deref() {
                        Some("text_delta") => {
                            if let Some(text) = delta.text {
                                turn.text.push_str(&text);
                                on_delta(LlmDelta::Text(text));
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(fragment) = delta.partial_json {
                                accumulator.append_json(index, &fragment);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(delta) = data.delta {
                    if delta.stop_reason.is_some() {
                        turn.stop_reason = delta.stop_reason;
                    }
                }
                if let Some(usage) = data.usage {
                    if usage.output_tokens.is_some() {
                        turn.output_tokens = usage.output_tokens;
                    }
                }
            }
            "error" => {
                return Err(data
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| "API 返回未知错误".into()));
            }
            _ => {} // message_start / content_block_stop / message_stop / ping
        }
    }

    turn.tool_calls = accumulator.finish();
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_tool_use_blocks() {
        let mut acc = BlockAccumulator::default();
        acc.start_tool(1, "toolu_1".into(), "grep".into());
        acc.append_json(1, r#"{"pattern":"#);
        acc.append_json(1, r#""main"}"#);
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].arguments, r#"{"pattern":"main"}"#);
    }

    #[test]
    fn empty_tool_arguments_default_to_object() {
        let mut acc = BlockAccumulator::default();
        acc.start_tool(0, "toolu_2".into(), "list_dir".into());
        assert_eq!(acc.finish()[0].arguments, "{}");
    }

    #[test]
    fn wire_messages_include_tool_rounds() {
        let history = vec![
            HistoryItem::User("hi".into()),
            HistoryItem::Assistant {
                text: "看一下".into(),
                tool_calls: vec![ToolCall {
                    id: "toolu_1".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"a.rs"}"#.into(),
                }],
            },
            HistoryItem::ToolResult {
                call_id: "toolu_1".into(),
                name: "read_file".into(),
                content: "fn main(){}".into(),
                is_error: false,
            },
        ];
        let wire = to_wire_messages(&history);
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[1]["content"][1]["type"], "tool_use");
        assert_eq!(wire[1]["content"][1]["input"]["path"], "a.rs");
        assert_eq!(wire[2]["content"][0]["tool_use_id"], "toolu_1");
    }
}
