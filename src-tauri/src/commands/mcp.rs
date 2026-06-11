use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::mcp::McpManager;

pub struct McpState {
    pub manager: Mutex<Arc<McpManager>>,
    pub config_path: PathBuf,
}

const CONFIG_TEMPLATE: &str = r#"{
  "servers": {
  }
}
"#;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

fn status_of(manager: &McpManager) -> Vec<McpServerInfo> {
    manager
        .servers
        .iter()
        .map(|s| match &s.connection {
            Ok(conn) => McpServerInfo {
                name: s.name.clone(),
                connected: true,
                tool_count: conn.tools.len(),
                error: None,
            },
            Err(e) => McpServerInfo {
                name: s.name.clone(),
                connected: false,
                tool_count: 0,
                error: Some(e.clone()),
            },
        })
        .collect()
}

#[tauri::command]
pub fn mcp_status(state: State<'_, McpState>) -> Vec<McpServerInfo> {
    status_of(&state.manager.lock().unwrap())
}

/// 重新读配置并重连所有 server（编辑 mcp.json 后点"重新加载"）
#[tauri::command]
pub async fn mcp_reload(app: AppHandle) -> Result<Vec<McpServerInfo>, String> {
    let state = app.state::<McpState>();
    let config_path = state.config_path.clone();
    // 连接握手是阻塞 IO，放 blocking 线程
    let manager = tauri::async_runtime::spawn_blocking(move || McpManager::load(&config_path))
        .await
        .map_err(|e| e.to_string())?;
    let manager = Arc::new(manager);
    let info = status_of(&manager);
    *app.state::<McpState>().manager.lock().unwrap() = manager;
    Ok(info)
}

/// 返回配置文件路径（不存在则先写入模板），前端展示给用户编辑
#[tauri::command]
pub fn mcp_config_path(state: State<'_, McpState>) -> Result<String, String> {
    if !state.config_path.exists() {
        std::fs::write(&state.config_path, CONFIG_TEMPLATE).map_err(|e| e.to_string())?;
    }
    Ok(state.config_path.display().to_string())
}
