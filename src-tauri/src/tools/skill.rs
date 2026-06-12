//! load_skill 工具：按需读取技能完整指令（progressive disclosure 的"展开"动作）

use std::path::Path;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::llm::types::ToolSpec;
use crate::skills;

pub struct LoadSkillTool;

impl Tool for LoadSkillTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "load_skill".into(),
            description: "读取某个技能的完整指令（SKILL.md 正文）。当任务与 system prompt 中列出的某技能匹配时，先调用本工具再按指令执行。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "技能名（见 system prompt 的技能清单）"}
                },
                "required": ["name"]
            }),
        }
    }

    /// 全局技能（~/.codeforge/skills）不依赖工作区
    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let name = input["name"].as_str().ok_or("缺少 name 参数")?;
        skills::load(Some(workspace), name)
    }
}
