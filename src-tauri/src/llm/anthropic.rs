use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use super::types::{ChatMessage, LlmEvent};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 64000;

#[derive(Debug, Deserialize)]
struct SseData {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct Delta {
    #[serde(rename = "type", default)]
    delta_type: Option<String>,
    #[serde(default)]
    text: Option<String>,
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

/// 解析一条 SSE data 负载，转成 LlmEvent（无关事件返回 None，error 事件返回 Err）
fn parse_sse_data(data: &str) -> Result<Option<LlmEvent>, String> {
    let parsed: SseData =
        serde_json::from_str(data).map_err(|e| format!("响应解析失败: {e}"))?;

    match parsed.event_type.as_str() {
        "content_block_delta" => {
            if let Some(delta) = parsed.delta {
                if delta.delta_type.as_deref() == Some("text_delta") {
                    if let Some(text) = delta.text {
                        return Ok(Some(LlmEvent::TextDelta(text)));
                    }
                }
            }
            Ok(None)
        }
        "message_delta" => Ok(Some(LlmEvent::TurnEnd {
            stop_reason: parsed.delta.and_then(|d| d.stop_reason),
            output_tokens: parsed.usage.and_then(|u| u.output_tokens),
        })),
        "error" => Err(parsed
            .error
            .map(|e| e.message)
            .unwrap_or_else(|| "API 返回未知错误".into())),
        _ => Ok(None), // message_start / content_block_start / stop / ping 等
    }
}

/// 调用 Anthropic Messages API（流式），每个事件经 on_event 回调
pub async fn stream_chat(
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    mut on_event: impl FnMut(LlmEvent),
) -> Result<(), String> {
    let body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "messages": messages,
    });

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
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        return Err(format!("API 错误 ({status}): {message}"));
    }

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("流读取失败: {e}"))?;
        if let Some(llm_event) = parse_sse_data(&event.data)? {
            on_event(llm_event);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        match parse_sse_data(data).unwrap() {
            Some(LlmEvent::TextDelta(text)) => assert_eq!(text, "Hello"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_message_delta_as_turn_end() {
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#;
        match parse_sse_data(data).unwrap() {
            Some(LlmEvent::TurnEnd { stop_reason, output_tokens }) => {
                assert_eq!(stop_reason.as_deref(), Some("end_turn"));
                assert_eq!(output_tokens, Some(12));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn ignores_irrelevant_events() {
        let data = r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
        assert!(parse_sse_data(data).unwrap().is_none());
    }

    #[test]
    fn surfaces_api_error_events() {
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        assert_eq!(parse_sse_data(data).unwrap_err(), "Overloaded");
    }
}
