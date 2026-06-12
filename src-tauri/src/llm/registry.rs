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
        // 优先 Keychain（设置页填写），兜底环境变量。
        // Keychain 报错（被锁/未授权，如 cargo test 二进制）也降级到 env，不让整条链路挂掉
        Endpoint::OpenAiCompatible { key_env, .. } => {
            if let Ok(Some(key)) = config::get_key(key_env) {
                return Ok(key);
            }
            std::env::var(key_env).map_err(|_| {
                format!("未配置 {key_env}：请在设置中填写，或在启动 app 的终端里配置环境变量")
            })
        }
    }
}
