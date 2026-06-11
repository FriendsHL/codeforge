//! 把 MCP server 的工具适配成内置 Tool trait——M2 多来源注册表设计的兑现。

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use super::registry::{ApprovalPlan, Tool};
use crate::llm::types::ToolSpec;
use crate::mcp::{McpConnection, McpToolDef};

pub struct McpToolAdapter {
    connection: Arc<McpConnection>,
    def: McpToolDef,
}

impl McpToolAdapter {
    pub fn wrap_all(connection: &Arc<McpConnection>) -> Vec<Arc<dyn Tool>> {
        connection
            .tools
            .iter()
            .map(|def| {
                Arc::new(McpToolAdapter {
                    connection: connection.clone(),
                    def: def.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }
}

impl Tool for McpToolAdapter {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            // mcp__<server>__<tool>，与模型工具名约束（字母数字下划线）兼容
            name: format!("mcp__{}__{}", self.connection.server_name, self.def.name),
            description: format!("[MCP:{}] {}", self.connection.server_name, self.def.description),
            input_schema: self.def.input_schema.clone(),
        }
    }

    /// 外部工具副作用未知，一律走审批（会话级"全部允许"可放行）
    fn plan(&self, _workspace: &Path, input: &Value) -> Result<Option<ApprovalPlan>, String> {
        Ok(Some(ApprovalPlan {
            summary: format!(
                "MCP {}/{} {}",
                self.connection.server_name,
                self.def.name,
                serde_json::to_string(input).unwrap_or_default()
            ),
            diff: String::new(),
        }))
    }

    fn run(&self, _workspace: &Path, input: &Value) -> Result<String, String> {
        self.connection.call_tool(&self.def.name, input.clone())
    }
}
