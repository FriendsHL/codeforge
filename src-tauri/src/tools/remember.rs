//! remember 工具：agent 把值得长期记住的事实写入 MEMORY.md（跨会话持久）

use std::path::Path;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::registry::Tool;
use crate::llm::types::ToolSpec;

pub struct RememberTool {
    pub app: AppHandle,
}

impl Tool for RememberTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "remember".into(),
            description: "把值得长期记住的事实写入持久记忆（跨会话存活）。用于用户偏好、项目约定、关键决策、已踩过的坑等——下次会自动出现在你的上下文里。一次只记一条、精炼。scope=project 记到当前项目，scope=global 记到用户全局。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "要记住的一条事实，精炼成一句话"},
                    "category": {"type": "string", "description": "分类，如 用户偏好 / 项目约定 / 关键决策 / 已知坑（默认 备忘）"},
                    "scope": {"type": "string", "enum": ["project", "global"], "description": "project=当前项目（默认），global=跨项目的用户级"}
                },
                "required": ["content"]
            }),
        }
    }

    /// global 记忆不依赖工作区
    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let content = input["content"].as_str().ok_or("缺少 content 参数")?;
        if content.trim().is_empty() {
            return Err("content 不能为空".into());
        }
        let category = input["category"].as_str().unwrap_or("备忘");
        let scope = input["scope"].as_str().unwrap_or("project");
        // 临时目录占位时（纯聊天模式）落到 global
        let ws = if workspace == std::env::temp_dir() { None } else { Some(workspace) };
        let scope = if ws.is_none() { "global" } else { scope };

        // v4-8：带查重 + 质量门槛的写入，防止记忆膨胀与低质堆积
        use crate::memory::RememberOutcome;
        match crate::memory::remember(ws, scope, category, content)? {
            RememberOutcome::Saved { path } => {
                let _ = self.app.emit("memory-updated", &path);
                Ok(format!("已记住（{scope}/{category}）：{content}"))
            }
            RememberOutcome::Duplicate { existing } => {
                Ok(format!("已有相近记忆，未重复记录：{existing}"))
            }
            RememberOutcome::TooWeak { reason } => Ok(reason),
        }
    }
}
