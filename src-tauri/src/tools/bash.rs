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

/// 把"用 bash 读文件/列目录/搜内容"的滥用挡回去，引导到专用工具。
/// 只拦最明确的简单命令（无管道/重定向/逻辑符），避免误伤组合命令。
/// 返回 Some(引导语) 表示应拒绝。
fn redirect_to_proper_tool(command: &str) -> Option<String> {
    let cmd = command.trim();
    // 含管道/重定向/逻辑符/子命令的组合命令放行（可能是正当用途，如 find|xargs wc）
    if cmd.contains('|')
        || cmd.contains("&&")
        || cmd.contains("||")
        || cmd.contains(';')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('`')
        || cmd.contains("$(")
    {
        return None;
    }
    let first = cmd.split_whitespace().next().unwrap_or("");
    match first {
        "cat" | "head" | "tail" | "less" | "more" => Some(
            "请用 read_file 读取文件内容（支持 offset/limit 分页），不要用 bash 的 cat/head/tail。".into(),
        ),
        "ls" => Some("请用 list_dir 列目录，不要用 bash ls。".into()),
        "find" => Some("请用 glob（按 **/*.rs 这类模式找文件）或 list_dir，不要用 bash find。".into()),
        "grep" | "rg" | "rep" | "egrep" => {
            Some("请用 grep 工具（正则搜内容），不要用 bash grep/rg。".into())
        }
        _ => None,
    }
}

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
        // 挡回"用 bash 读文件/列目录/搜内容"的滥用（最常见的浪费来源）
        if let Some(hint) = redirect_to_proper_tool(command) {
            return Err(hint);
        }
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
    fn redirects_file_ops_to_proper_tools() {
        // 简单的读/列/搜命令 → 拦回
        assert!(redirect_to_proper_tool("cat src/main.rs").is_some());
        assert!(redirect_to_proper_tool("ls src").is_some());
        assert!(redirect_to_proper_tool("find . -name '*.rs'").is_some());
        assert!(redirect_to_proper_tool("grep foo src").is_some());
        assert!(redirect_to_proper_tool("head -20 a.txt").is_some());
        // 组合/管道/正当执行命令 → 放行
        assert!(redirect_to_proper_tool("cargo test").is_none());
        assert!(redirect_to_proper_tool("npm run build").is_none());
        assert!(redirect_to_proper_tool("find . -name '*.rs' | xargs wc -l").is_none());
        assert!(redirect_to_proper_tool("git status").is_none());
    }

    #[test]
    fn bash_rejects_cat_at_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let r = BashTool.run_streaming(dir.path(), &json!({"command": "cat foo.txt"}), &mut |_| {});
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("read_file"));
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
