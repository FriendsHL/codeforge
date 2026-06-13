//! bash 工具：经 PTY 执行 shell 命令，输出实时流向前端终端

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::registry::{ApprovalPlan, Tool};
use crate::llm::types::ToolSpec;
use crate::pty;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// 回填给模型的输出上限（保头保尾，中间截断——报错通常在尾部）
const MAX_RESULT_CHARS: usize = 30_000;

pub struct BashTool;

fn truncate_middle(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MAX_RESULT_CHARS {
        return text.to_string();
    }
    let head: String = chars[..MAX_RESULT_CHARS / 3].iter().collect();
    let tail: String = chars[chars.len() - MAX_RESULT_CHARS * 2 / 3..].iter().collect();
    format!("{head}\n…[输出过长，中间已截断]…\n{tail}")
}

impl Tool for BashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".into(),
            description: "在工作区根目录执行 shell 命令（zsh -c），适合跑测试、构建、安装依赖、git 操作等。输出实时展示给用户。耗时长的命令记得调大 timeout_secs。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "要执行的 shell 命令"},
                    "timeout_secs": {"type": "integer", "description": "超时秒数，默认 120，最大 600"}
                },
                "required": ["command"]
            }),
        }
    }

    fn plan(&self, _workspace: &Path, input: &Value) -> Result<Option<ApprovalPlan>, String> {
        let command = input["command"].as_str().ok_or("缺少 command 参数")?;
        Ok(Some(ApprovalPlan {
            summary: command.to_string(),
            diff: String::new(),
            danger: crate::security::danger::detect(command),
        }))
    }

    fn is_mutating(&self) -> bool {
        true
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        self.run_streaming(workspace, input, &mut |_| {})
    }

    fn run_streaming(
        &self,
        workspace: &Path,
        input: &Value,
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let command = input["command"].as_str().ok_or("缺少 command 参数")?;
        let timeout = input["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(5, MAX_TIMEOUT_SECS);

        let result = pty::run_command(
            workspace,
            command,
            Duration::from_secs(timeout),
            |chunk| on_chunk(chunk),
        )?;

        let clean = truncate_middle(&pty::strip_ansi(&result.output));
        let status = if result.timed_out {
            format!("[超时（{timeout}s）已强制终止]")
        } else {
            format!("exit code: {}", result.exit_code)
        };
        Ok(format!("{status}\n{clean}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_tool_runs_and_reports_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut streamed = String::new();
        let out = BashTool
            .run_streaming(
                dir.path(),
                &json!({"command": "echo from-bash-tool"}),
                &mut |c| streamed.push_str(c),
            )
            .unwrap();
        assert!(out.contains("exit code: 0"));
        assert!(out.contains("from-bash-tool"));
        assert!(streamed.contains("from-bash-tool"));
    }

    #[test]
    fn bash_tool_plan_requires_approval_with_command() {
        let plan = BashTool
            .plan(Path::new("/tmp"), &json!({"command": "cargo test"}))
            .unwrap()
            .unwrap();
        assert_eq!(plan.summary, "cargo test");
        assert!(plan.diff.is_empty());
    }
}
