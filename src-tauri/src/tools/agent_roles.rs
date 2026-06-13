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
                format!("- {}：{}\n  可用工具：{}", r.name, r.description, tools)
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!("可用 agent 角色（用 spawn_subagents 的 role 参数指派）：\n{body}"))
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
