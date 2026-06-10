//! OpenAI 兼容协议的流式客户端（火山方舟 Ark / 小米 MiMo 等）

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use super::types::{ChatMessage, LlmEvent};

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
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    completion_tokens: Option<u64>,
}

#[derive(Debug, Default, PartialEq)]
struct ParsedChunk {
    content: Option<String>,
    reasoning: Option<String>,
    finish_reason: Option<String>,
    output_tokens: Option<u64>,
}

fn parse_chunk(data: &str) -> Result<ParsedChunk, String> {
    let chunk: StreamChunk =
        serde_json::from_str(data).map_err(|e| format!("响应解析失败: {e}"))?;
    let mut parsed = ParsedChunk {
        output_tokens: chunk.usage.and_then(|u| u.completion_tokens),
        ..Default::default()
    };
    if let Some(choice) = chunk.choices.into_iter().next() {
        parsed.finish_reason = choice.finish_reason;
        if let Some(delta) = choice.delta {
            parsed.content = delta.content.filter(|s| !s.is_empty());
            parsed.reasoning = delta.reasoning_content.filter(|s| !s.is_empty());
        }
    }
    Ok(parsed)
}

/// 调用 OpenAI 兼容的 chat/completions（流式），每个事件经 on_event 回调
pub async fn stream_chat(
    chat_url: &str,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    mut on_event: impl FnMut(LlmEvent),
) -> Result<(), String> {
    let body = json!({
        "model": model,
        "stream": true,
        "messages": messages,
    });

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
        let message = serde_json::from_str::<serde_json::Value>(&text)
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

    let mut finish_reason: Option<String> = None;
    let mut output_tokens: Option<u64> = None;

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| format!("流读取失败: {e}"))?;
        if event.data.trim() == "[DONE]" {
            break;
        }
        let parsed = parse_chunk(&event.data)?;
        if let Some(text) = parsed.reasoning {
            on_event(LlmEvent::ReasoningDelta(text));
        }
        if let Some(text) = parsed.content {
            on_event(LlmEvent::TextDelta(text));
        }
        if parsed.finish_reason.is_some() {
            finish_reason = parsed.finish_reason;
        }
        if parsed.output_tokens.is_some() {
            output_tokens = parsed.output_tokens;
        }
    }

    on_event(LlmEvent::TurnEnd {
        stop_reason: finish_reason,
        output_tokens,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_delta() {
        let data = r#"{"choices":[{"index":0,"delta":{"content":"你好"}}]}"#;
        let parsed = parse_chunk(data).unwrap();
        assert_eq!(parsed.content.as_deref(), Some("你好"));
        assert!(parsed.reasoning.is_none());
    }

    #[test]
    fn parses_reasoning_delta() {
        let data = r#"{"choices":[{"index":0,"delta":{"reasoning_content":"思考中"}}]}"#;
        let parsed = parse_chunk(data).unwrap();
        assert_eq!(parsed.reasoning.as_deref(), Some("思考中"));
        assert!(parsed.content.is_none());
    }

    #[test]
    fn parses_finish_reason_and_usage() {
        let data = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"completion_tokens":42}}"#;
        let parsed = parse_chunk(data).unwrap();
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.output_tokens, Some(42));
    }

    #[test]
    fn tolerates_empty_choices() {
        let data = r#"{"choices":[],"usage":{"completion_tokens":7}}"#;
        let parsed = parse_chunk(data).unwrap();
        assert_eq!(parsed.output_tokens, Some(7));
    }
}
