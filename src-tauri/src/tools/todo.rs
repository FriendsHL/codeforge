//! todo_write 工具：agent 维护任务清单。清单存在 AgentCtx，loop 每轮把它作为
//! <system-reminder> 拼进 system prompt，让模型始终知道"现在该干什么、做到哪步"。

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::registry::Tool;
use crate::llm::types::ToolSpec;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    /// pending | in_progress | completed
    pub status: String,
}

pub type TodoList = Arc<Mutex<Vec<TodoItem>>>;

/// 把当前 todo 渲染成给模型的 system-reminder（无任务返回 None）
pub fn reminder(todos: &TodoList) -> Option<String> {
    let todos = todos.lock().unwrap();
    if todos.is_empty() {
        return None;
    }
    let lines: Vec<String> = todos
        .iter()
        .map(|t| {
            let mark = match t.status.as_str() {
                "completed" => "[x]",
                "in_progress" => "[~]",
                _ => "[ ]",
            };
            format!("{mark} {}", t.content)
        })
        .collect();
    Some(format!(
        "<system-reminder>\n当前任务清单（[x]=已完成 [~]=进行中 [ ]=待办）：\n{}\n完成一项就用 todo_write 更新状态；同一时间只标一项 in_progress。所有项 completed 后即可结束。\n</system-reminder>",
        lines.join("\n")
    ))
}

pub struct TodoWriteTool {
    pub todos: TodoList,
    pub app: AppHandle,
}

impl Tool for TodoWriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "todo_write".into(),
            description: "维护当前任务的待办清单（整表覆盖写）。适合 3 步以上的多步任务：开工前先列计划，每完成一步就更新状态。单步小任务不必使用。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "完整的任务清单（每次传全量，覆盖旧的）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string", "description": "任务描述"},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "pending 待办 / in_progress 进行中 / completed 已完成"
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        }
    }

    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, _workspace: &Path, input: &Value) -> Result<String, String> {
        let items: Vec<TodoItem> = serde_json::from_value(input["todos"].clone())
            .map_err(|e| format!("todos 解析失败: {e}"))?;
        let in_progress = items.iter().filter(|t| t.status == "in_progress").count();
        if in_progress > 1 {
            return Err("同一时间最多只能有一项 in_progress".into());
        }

        let done = items.iter().filter(|t| t.status == "completed").count();
        let total = items.len();
        *self.todos.lock().unwrap() = items.clone();
        // 通知前端更新任务面板
        let _ = self.app.emit("todo-update", &items);

        Ok(format!("任务清单已更新（{done}/{total} 完成）"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_reminder_with_marks() {
        let todos: TodoList = Arc::new(Mutex::new(vec![
            TodoItem { content: "读代码".into(), status: "completed".into() },
            TodoItem { content: "改 bug".into(), status: "in_progress".into() },
            TodoItem { content: "跑测试".into(), status: "pending".into() },
        ]));
        let r = reminder(&todos).unwrap();
        assert!(r.contains("[x] 读代码"));
        assert!(r.contains("[~] 改 bug"));
        assert!(r.contains("[ ] 跑测试"));
        assert!(r.contains("system-reminder"));
    }

    #[test]
    fn empty_todos_no_reminder() {
        let todos: TodoList = Arc::new(Mutex::new(vec![]));
        assert!(reminder(&todos).is_none());
    }
}
