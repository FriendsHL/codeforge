use serde::Serialize;

use crate::config;

/// provider id → Keychain account（OpenAI 兼容家直接复用环境变量名，env 兜底逻辑一致）
fn account_for(provider: &str) -> Result<&'static str, String> {
    match provider {
        "claude" => Ok(config::ANTHROPIC_KEY),
        "ark" => Ok("ARK_API_KEY"),
        "xiaomi-mimo" => Ok("XIAOMI_MIMO_API_KEY"),
        other => Err(format!("未知 provider: {other}")),
    }
}

#[tauri::command]
pub fn set_provider_key(provider: String, key: String) -> Result<(), String> {
    let account = account_for(&provider)?;
    let key = key.trim();
    if key.is_empty() {
        config::delete_key(account)
    } else {
        config::set_key(account, key)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub keychain: bool,
    pub env: bool,
}

#[tauri::command]
pub fn provider_key_status(provider: String) -> Result<KeyStatus, String> {
    let account = account_for(&provider)?;
    Ok(KeyStatus {
        keychain: config::get_key(account)?.is_some(),
        env: provider != "claude" && std::env::var(account).is_ok(),
    })
}

// ===== 兼容旧前端调用（Anthropic 专用） =====

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    set_provider_key("claude".into(), key)
}

#[tauri::command]
pub fn has_api_key() -> Result<bool, String> {
    Ok(config::get_api_key()?.is_some())
}
