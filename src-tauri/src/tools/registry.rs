//! 工具注册表。设计为多来源：v1 只有内置工具，v2 可挂 MCP / skill 工具，loop 不感知来源。

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::llm::types::ToolSpec;

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String>;
}

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
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
