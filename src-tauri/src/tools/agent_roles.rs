//! list_agents 工具：让主 agent 一等地查询可用的 agent 角色（名称/职责/工具集），
//! 据此决定把子任务派给谁（配合 spawn_subagents 的 role 参数）。

use std::path::Path;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::llm::types::ToolSpec;

pub struct ListAgentsTool;

impl Tool for ListAgentsTool {
    fn parallel_safe(&self) -> bool {
        true
    }

    // 角色含内置项，纯聊天模式（无工作区）也能列
    fn needs_workspace(&self) -> bool {
        false
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_agents".into(),
            description: "列出当前可用的 agent 角色（名称、职责、可用工具）。当你要把子任务分工派发时，先用它看清有哪些角色可选，再用 spawn_subagents 的 role 参数指派。".into(),
            input_schema: json!({"type": "object", "properties": {}, "required": []}),
        }
    }

    fn run(&self, workspace: &Path, _input: &Value) -> Result<String, String> {
        // 纯聊天模式占位的临时目录不当作真实工作区
        let ws = if workspace == std::env::temp_dir() { None } else { Some(workspace) };
        let roles = crate::agents::discover(ws);
        if roles.is_empty() {
            return Ok("当前没有可用角色".into());
        }
        let body = roles
            .iter()
            .map(|r| {
                let tools = if r.tools.is_empty() {
                    "全部工具".to_string()
                } else {
                    r.tools.join(", ")
                };
                format!("- {}（{}）：{}\n  可用工具：{}", r.name, r.source, r.description, tools)
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!(
            "可用 agent 角色（用 spawn_subagents 的 role 参数指派；不够用时可用 save_agent 新建或调优）：\n{body}"
        ))
    }
}

/// save_agent：创建或调优一个 agent 角色（写 AGENT.md 配置文件）。
/// 让"角色不够用/要优化"时通过工具改配置，而不是改代码。
pub struct SaveAgentTool {
    pub app: tauri::AppHandle,
}

impl super::registry::Tool for SaveAgentTool {
    fn needs_workspace(&self) -> bool {
        false // 默认写全局，不依赖工作区
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "save_agent".into(),
            description: "创建或覆盖一个 agent 角色（写入 AGENT.md，即时生效、跨会话存活）。现有角色不够用、或要优化某角色的人设/工具集时用它。覆盖内置角色只需用同名。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "角色名（字母/数字/-/_）"},
                    "description": {"type": "string", "description": "一句话职责，会出现在 list_agents 里"},
                    "system_prompt": {"type": "string", "description": "角色的 system prompt（人设+工作方式+纪律）"},
                    "tools": {"type": "array", "items": {"type": "string"}, "description": "允许的工具名白名单；省略或空=不限制（全部工具）"},
                    "scope": {"type": "string", "enum": ["project", "global"], "description": "project=当前项目，global=跨项目（默认 global）"}
                },
                "required": ["name", "description", "system_prompt"]
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        use tauri::Emitter;
        let name = input["name"].as_str().ok_or("缺少 name")?;
        let description = input["description"].as_str().ok_or("缺少 description")?;
        let system_prompt = input["system_prompt"].as_str().ok_or("缺少 system_prompt")?;
        let tools: Vec<String> = input["tools"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let scope = input["scope"].as_str().unwrap_or("global");
        let ws = if workspace == std::env::temp_dir() { None } else { Some(workspace) };
        let scope = if ws.is_none() && scope == "project" { "global" } else { scope };

        let path = crate::agents::save_role(scope, ws, name, description, &tools, system_prompt)?;
        let _ = self.app.emit("agents-updated", &path);
        let tools_desc = if tools.is_empty() { "全部工具".into() } else { tools.join(", ") };
        Ok(format!("已保存角色「{name}」（{scope}，工具：{tools_desc}）→ {path}"))
    }
}

/// delete_agent：删除一个用户定义的角色配置文件
pub struct DeleteAgentTool {
    pub app: tauri::AppHandle,
}

impl super::registry::Tool for DeleteAgentTool {
    fn needs_workspace(&self) -> bool {
        false
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "delete_agent".into(),
            description: "删除一个用户定义的 agent 角色配置文件。删除覆盖内置的同名角色后，内置版本会重新生效。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "要删除的角色名"},
                    "scope": {"type": "string", "enum": ["project", "global"], "description": "默认 global"}
                },
                "required": ["name"]
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        use tauri::Emitter;
        let name = input["name"].as_str().ok_or("缺少 name")?;
        let scope = input["scope"].as_str().unwrap_or("global");
        let ws = if workspace == std::env::temp_dir() { None } else { Some(workspace) };
        crate::agents::delete_role(scope, ws, name)?;
        let _ = self.app.emit("agents-updated", name);
        Ok(format!("已删除角色「{name}」（{scope}）"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_builtin_roles() {
        // 用一个肯定不存在自定义角色的临时目录当工作区
        let dir = tempfile::tempdir().unwrap();
        let out = ListAgentsTool.run(dir.path(), &json!({})).unwrap();
        assert!(out.contains("research"));
        assert!(out.contains("dev"));
        assert!(out.contains("review"));
        assert!(out.contains("全部工具")); // dev 角色 tools=*
    }
}
