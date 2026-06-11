//! 工具注册表。设计为多来源：v1 只有内置工具，v2 可挂 MCP / skill 工具，loop 不感知来源。

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::llm::types::ToolSpec;

/// 副作用操作的预演：loop 据此向用户发起审批
/// 写文件：summary=路径、diff=改动；执行命令：summary=命令、diff 为空
#[derive(Debug, Clone)]
pub struct ApprovalPlan {
    pub summary: String,
    pub diff: String,
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String>;
    /// 返回 Some(plan) 的工具有副作用，执行前必须经用户审批；只读工具保持默认 None
    fn plan(&self, _workspace: &Path, _input: &Value) -> Result<Option<ApprovalPlan>, String> {
        Ok(None)
    }
    /// 长耗时工具（如 bash）可边执行边经 on_chunk 输出；默认退化为一次性 run()
    fn run_streaming(
        &self,
        workspace: &Path,
        input: &Value,
        _on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        self.run(workspace, input)
    }
}

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// 从任意工具集合组装（内置 + MCP + 将来其他来源）
    pub fn from_tools(tools: Vec<Arc<dyn Tool>>) -> Self {
        Self { tools }
    }

    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    /// v1 内置只读工具集
    pub fn builtin() -> Self {
        Self {
            tools: vec![
                Arc::new(super::fs::ReadFileTool),
                Arc::new(super::fs::ListDirTool),
                Arc::new(super::search::GlobTool),
                Arc::new(super::search::GrepTool),
                Arc::new(super::git::GitStatusTool),
                Arc::new(super::git::GitDiffTool),
                Arc::new(super::git::GitLogTool),
                Arc::new(super::write::WriteFileTool),
                Arc::new(super::write::EditFileTool),
                Arc::new(super::bash::BashTool),
                Arc::new(super::web::WebFetchTool),
                Arc::new(super::web::WebSearchTool),
                Arc::new(super::skill::LoadSkillTool),
            ],
        }
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.spec().name == name).cloned()
    }
}
