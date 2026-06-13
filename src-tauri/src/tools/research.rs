//! deep_research 工具：对一个问题做严谨调研。
//! 流程:多角度并行检索(子 agent 各搜一面)→ 综合 → Generator-Verifier 自验证。
//! 复用主 agent 的 spawn_subagents 思路,但封装成单一工具,产出带引用的结论。
//!
//! 实现取舍:工具本身不直接跑子 agent(loop 才有那套递归基建),而是返回一份
//! "调研计划提示",引导主 agent 用 web_search/web_fetch + spawn_subagents 按
//! Generator-Verifier 模式执行。真正的并行检索由主 agent 的 spawn_subagents 完成。
//! 这样保持工具薄、不重复 loop 逻辑,又把"严谨调研方法论"固化进流程。

use std::path::Path;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::llm::types::ToolSpec;

pub struct ResearchPlanTool;

impl Tool for ResearchPlanTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "research_plan".into(),
            description: "当用户要求做调研/深度调查/技术选型/竞品对比等需要严谨多源信息的任务时调用。它返回一套结构化调研方法,你按它执行(并行多角度检索→综合→自我反驳验证→带引用的结论)。普通的单点查询用 web_search 即可,不必用本工具。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": {"type": "string", "description": "要调研的核心问题"}
                },
                "required": ["question"]
            }),
        }
    }

    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, _workspace: &Path, input: &Value) -> Result<String, String> {
        let question = input["question"].as_str().ok_or("缺少 question 参数")?;
        Ok(format!(
            "对「{question}」执行以下严谨调研流程,逐步进行:\n\
\n\
1. 拆解角度:把问题拆成 2~4 个独立子角度(如:官方文档/现状、对立观点/批评、最新进展、实际案例)。\n\
2. 并行检索:用 spawn_subagents 给每个角度派一个子 agent,各自 web_search + web_fetch 深入。要求每个子 agent 的汇报里,每条关键事实都附带来源 URL。\n\
3. 综合:汇总各角度发现,合并重复、标注分歧点。\n\
4. 自验证(Generator-Verifier):对你打算下的每个关键结论,主动反问「有没有相反证据?来源可靠吗?是否过时?」,必要时再 web_search 一轮反驳性查询。证据不足的结论要明确标注「不确定」。\n\
5. 产出:给出结论 + 关键依据,每条依据标注来源 [序号](URL);最后列「参考来源」清单。区分「确证」与「存疑」。\n\
\n\
注意:不要编造来源;时效性强的话题优先用近期来源,并注明信息可能的时间。"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_structured_research_method() {
        let out = ResearchPlanTool
            .run(Path::new("/tmp"), &json!({"question": "Tauri vs Electron 选型"}))
            .unwrap();
        assert!(out.contains("Tauri vs Electron 选型"));
        assert!(out.contains("spawn_subagents"));
        assert!(out.contains("来源"));
        assert!(out.contains("Generator-Verifier") || out.contains("自验证"));
    }

    #[test]
    fn rejects_missing_question() {
        assert!(ResearchPlanTool.run(Path::new("/tmp"), &json!({})).is_err());
    }
}
