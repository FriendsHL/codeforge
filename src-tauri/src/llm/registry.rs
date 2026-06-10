//! Provider 注册表：provider id → 协议 + 端点 + key 来源
//! 端点配置参照 skillForge application.yml（Ark base 已带版本段，不能再加 /v1；
//! 小米模型名必须全小写）。

use crate::config;

pub enum Endpoint {
    /// Anthropic Messages API，key 存 macOS Keychain
    Anthropic,
    /// OpenAI 兼容 chat/completions，key 从环境变量读取
    OpenAiCompatible {
        chat_url: &'static str,
        key_env: &'static str,
    },
}

pub fn resolve(provider: &str) -> Result<Endpoint, String> {
    match provider {
        "claude" => Ok(Endpoint::Anthropic),
        "ark" => Ok(Endpoint::OpenAiCompatible {
            chat_url: "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions",
            key_env: "ARK_API_KEY",
        }),
        "xiaomi-mimo" => Ok(Endpoint::OpenAiCompatible {
            chat_url: "https://token-plan-cn.xiaomimimo.com/v1/chat/completions",
            key_env: "XIAOMI_MIMO_API_KEY",
        }),
        other => Err(format!("未知 provider: {other}")),
    }
}

pub fn api_key_for(endpoint: &Endpoint) -> Result<String, String> {
    match endpoint {
        Endpoint::Anthropic => config::get_api_key()?
            .ok_or_else(|| "尚未设置 Anthropic API key，请先在设置中填写".to_string()),
        Endpoint::OpenAiCompatible { key_env, .. } => std::env::var(key_env)
            .map_err(|_| format!("环境变量 {key_env} 未设置（请在启动 app 的终端里配置）")),
    }
}
