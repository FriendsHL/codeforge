//! 快捷命令（/tools /skills）的数据源：当前能力清单

use serde::Serialize;
use tauri::State;

use crate::commands::mcp::McpState;
use crate::skills;
use crate::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityItem {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub tools: Vec<CapabilityItem>,
    pub skills: Vec<CapabilityItem>,
}

/// /trace：聚合本会话的 trace 文件
#[tauri::command]
pub fn trace_summary(
    session_id: i64,
    app: tauri::AppHandle,
) -> Result<crate::trace::TraceSummary, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(crate::trace::summarize(&dir.join("traces"), session_id))
}

#[tauri::command]
pub fn list_capabilities(
    state: State<'_, AppState>,
    mcp: State<'_, McpState>,
) -> Capabilities {
    let workspace = state.workspace.lock().unwrap().clone();

    let mut tools: Vec<CapabilityItem> = state
        .tools
        .specs(workspace.is_some(), false)
        .into_iter()
        .map(|s| CapabilityItem { name: s.name, description: s.description })
        .collect();
    // 运行时动态注入的两个工具（不在静态注册表里）
    tools.push(CapabilityItem {
        name: "spawn_subagents".into(),
        description: "把 1~4 个独立子任务并行派给子 agent（独立上下文、可用全部工具）".into(),
    });
    tools.push(CapabilityItem {
        name: "browser_open".into(),
        description: "在内嵌浏览器面板中打开 URL 给用户看".into(),
    });
    tools.push(CapabilityItem {
        name: "todo_write".into(),
        description: "维护多步任务的待办清单".into(),
    });
    tools.push(CapabilityItem {
        name: "remember".into(),
        description: "把值得长期记住的事实写入持久记忆（跨会话）".into(),
    });
    for connection in mcp.manager.lock().unwrap().connections() {
        for def in &connection.tools {
            tools.push(CapabilityItem {
                name: format!("mcp__{}__{}", connection.server_name, def.name),
                description: format!("[MCP:{}] {}", connection.server_name, def.description),
            });
        }
    }

    let skills = skills::discover(workspace.as_deref())
        .into_iter()
        .map(|s| CapabilityItem { name: s.name, description: s.description })
        .collect();

    Capabilities { tools, skills }
}
