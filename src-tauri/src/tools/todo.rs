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
    /// 命令式描述（"运行测试"）
    pub content: String,
    /// 进行时描述（"正在运行测试"），用于 in_progress 时的状态展示；对齐 Claude TodoWrite
    #[serde(default, rename = "activeForm")]
    pub active_form: Option<String>,
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
            // 进行中的项优先显示进行时（activeForm），其余显示命令式 content
            let label = if t.status == "in_progress" {
                t.active_form.as_deref().filter(|s| !s.is_empty()).unwrap_or(&t.content)
            } else {
                &t.content
            };
            format!("{mark} {label}")
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
            description: "维护本次任务的待办清单（每次传全量、整表覆盖）。\
何时用：收到 3 步以上或复杂的多步任务、用户一次给了多件事时，开工前先列计划。单步或琐碎任务不要用。\
怎么用：开始做某项前先把它标 in_progress（同一时间只能有一项 in_progress）；做完立刻标 completed 再开下一项，不要攒着批量标；中途发现新任务就追加进来。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "完整的任务清单（每次传全量，覆盖旧的）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string", "description": "命令式任务描述，如「运行测试」"},
                                "activeForm": {"type": "string", "description": "进行时描述，如「正在运行测试」，在该项 in_progress 时展示"},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "pending 待办 / in_progress 进行中（同时仅一项）/ completed 已完成"
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
            TodoItem { content: "读代码".into(), active_form: None, status: "completed".into() },
            TodoItem { content: "改 bug".into(), active_form: Some("正在改 bug".into()), status: "in_progress".into() },
            TodoItem { content: "跑测试".into(), active_form: None, status: "pending".into() },
        ]));
        let r = reminder(&todos).unwrap();
        assert!(r.contains("[x] 读代码"));
        assert!(r.contains("[~] 正在改 bug"), "进行中应显示 activeForm");
        assert!(r.contains("[ ] 跑测试"));
        assert!(r.contains("system-reminder"));
    }

    #[test]
    fn empty_todos_no_reminder() {
        let todos: TodoList = Arc::new(Mutex::new(vec![]));
        assert!(reminder(&todos).is_none());
    }
}
