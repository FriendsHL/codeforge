//! OpenAI 兼容协议的流式客户端（火山方舟 Ark / 小米 MiMo 等），支持 tool calls

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{AssistantTurn, HistoryItem, LlmDelta, ToolCall, ToolSpec};

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    completion_tokens: Option<u64>,
}

/// 按 index 累积流式 tool_calls 片段
#[derive(Debug, Default)]
struct ToolCallAccumulator {
    calls: Vec<ToolCall>,
}

impl ToolCallAccumulator {
    fn apply(&mut self, delta: ToolCallDelta) {
        while self.calls.len() <= delta.index {
            self.calls.push(ToolCall {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });
        }
        let call = &mut self.calls[delta.index];
        if let Some(id) = delta.id {
            call.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                call.name.push_str(&name);
            }
            if let Some(args) = function.arguments {
                call.arguments.push_str(&args);
            }
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .collect()
    }
}

fn to_wire_messages(system: Option<&str>, history: &[HistoryItem]) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for item in history {
        match item {
            HistoryItem::User(text) => {
                messages.push(json!({"role": "user", "content": text}));
            }
            HistoryItem::Assistant { text, tool_calls } => {
                let mut msg = json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(
                        tool_calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": {"name": c.name, "arguments": c.arguments},
                                })
                            })
                            .collect(),
                    );
                }
                messages.push(msg);
            }
            HistoryItem::ToolResult { call_id, content, .. } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }));
            }
        }
    }
    messages
}

fn to_wire_tools(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                },
            })
        })
        .collect()
}

/// 调用 OpenAI 兼容的 chat/completions（流式），返回完整的 assistant 轮次
pub async fn stream_chat(
    chat_url: &str,
    api_key: &str,
    model: &str,
    system: Option<&str>,
    history: &[HistoryItem],
    tools: &[ToolSpec],
    cancel: &std::sync::atomic::AtomicBool,
    mut on_delta: impl FnMut(LlmDelta),
) -> Result<AssistantTurn, String> {
    // 不设 max_tokens 会落到 provider 默认值（火山仅 4K，长回答被静默截断）
    let mut body = json!({
        "model": model,
        "stream": true,
        "max_tokens": 16384,
        "messages": to_wire_messages(system, history),
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(to_wire_tools(tools));
    }

    let response = reqwest::Client::new()
        .post(chat_url)
        .header("Authorization", format!("Bearer {api_key}"))
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
            .and_then(|v| {
                v["error"]["message"]
                    .as_str()
                    .or(v["message"].as_str())
                    .map(String::from)
            })
            .unwrap_or(text);
        return Err(format!("API 错误 ({status}): {message}"));
    }

    let mut turn = AssistantTurn::default();
    let mut accumulator = ToolCallAccumulator::default();

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(crate::llm::types::CANCELLED_ERR.into());
        }
        let event = event.map_err(|e| format!("流读取失败: {e}"))?;
        if event.data.trim() == "[DONE]" {
            break;
        }
        let chunk: StreamChunk =
            serde_json::from_str(&event.data).map_err(|e| format!("响应解析失败: {e}"))?;
        if let Some(usage) = chunk.usage {
            if usage.completion_tokens.is_some() {
                turn.output_tokens = usage.completion_tokens;
            }
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if choice.finish_reason.is_some() {
                turn.stop_reason = choice.finish_reason;
            }
            if let Some(delta) = choice.delta {
                if let Some(text) = delta.reasoning_content.filter(|s| !s.is_empty()) {
                    on_delta(LlmDelta::Reasoning(text));
                }
                if let Some(text) = delta.content.filter(|s| !s.is_empty()) {
                    turn.text.push_str(&text);
                    on_delta(LlmDelta::Text(text));
                }
                if let Some(tool_deltas) = delta.tool_calls {
                    for tool_delta in tool_deltas {
                        accumulator.apply(tool_delta);
                    }
                }
            }
        }
    }

    turn.tool_calls = accumulator.finish();
    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(json_str: &str) -> ToolCallDelta {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn accumulates_streamed_tool_call_fragments() {
        let mut acc = ToolCallAccumulator::default();
        acc.apply(delta(
            r#"{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}"#,
        ));
        acc.apply(delta(r#"{"index":0,"function":{"arguments":"{\"path\":"}}"#));
        acc.apply(delta(r#"{"index":0,"function":{"arguments":"\"a.rs\"}"}}"#));
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, r#"{"path":"a.rs"}"#);
    }

    #[test]
    fn accumulates_parallel_tool_calls() {
        let mut acc = ToolCallAccumulator::default();
        acc.apply(delta(r#"{"index":0,"id":"c0","function":{"name":"glob","arguments":"{}"}}"#));
        acc.apply(delta(r#"{"index":1,"id":"c1","function":{"name":"grep","arguments":"{}"}}"#));
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, "grep");
    }

    #[test]
    fn wire_messages_include_tool_rounds() {
        let history = vec![
            HistoryItem::User("看下结构".into()),
            HistoryItem::Assistant {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "list_dir".into(),
                    arguments: "{}".into(),
                }],
            },
            HistoryItem::ToolResult {
                call_id: "c1".into(),
                name: "list_dir".into(),
                content: "src/".into(),
                is_error: false,
            },
        ];
        let wire = to_wire_messages(Some("sys"), &history);
        assert_eq!(wire.len(), 4);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[2]["tool_calls"][0]["function"]["name"], "list_dir");
        assert_eq!(wire[3]["role"], "tool");
        assert_eq!(wire[3]["tool_call_id"], "c1");
    }
}
