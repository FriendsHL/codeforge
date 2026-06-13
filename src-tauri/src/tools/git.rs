//! agent 的 git 只读工具：git_status / git_diff / git_log

use std::path::Path;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::git;
use crate::llm::types::ToolSpec;

const MAX_DIFF_CHARS: usize = 40_000;
const MAX_LOG_COUNT: u64 = 100;

fn require_repo(workspace: &Path) -> Result<(), String> {
    if git::is_repo(workspace) {
        Ok(())
    } else {
        Err("当前工作区不是 git 仓库".into())
    }
}

fn cap(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        text
    } else {
        let shown: String = text.chars().take(limit).collect();
        format!("{shown}\n…[truncated: diff 过大，建议指定 path 缩小范围]")
    }
}

pub struct GitStatusTool;

impl Tool for GitStatusTool {
    fn parallel_safe(&self) -> bool {
        true
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "git_status".into(),
            description: "查看 git 仓库当前状态：所在分支 + 改动文件列表（M 修改/A 新增/D 删除/R 重命名/? 未跟踪）。".into(),
            input_schema: json!({"type": "object", "properties": {}, "required": []}),
        }
    }

    fn run(&self, workspace: &Path, _input: &Value) -> Result<String, String> {
        require_repo(workspace)?;
        let branch = git::current_branch(workspace)?;
        let entries = git::status_entries(workspace)?;
        if entries.is_empty() {
            return Ok(format!("分支: {branch}\n工作区干净，无改动"));
        }
        let list = entries
            .iter()
            .map(|e| format!("{} {}", e.status, e.path))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!("分支: {branch}\n改动文件:\n{list}"))
    }
}

pub struct GitDiffTool;

impl Tool for GitDiffTool {
    fn parallel_safe(&self) -> bool {
        true
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "git_diff".into(),
            description: "查看未提交的代码改动（unified diff）。可用 path 限定单个文件/目录，staged=true 看已暂存部分。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "限定某个文件或目录（可选）"},
                    "staged": {"type": "boolean", "description": "true 看已暂存（git add 过）的改动，默认 false"}
                },
                "required": []
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        require_repo(workspace)?;
        let mut args: Vec<&str> = vec!["diff"];
        if input["staged"].as_bool().unwrap_or(false) {
            args.push("--staged");
        }
        let path = input["path"].as_str();
        if let Some(path) = path {
            git::validate_git_path(path)?;
            args.push("--");
            args.push(path);
        }
        let diff = git::run_git(workspace, &args)?;
        if diff.trim().is_empty() {
            return Ok("无改动（提示：未跟踪的新文件不会出现在 diff 里，可先用 git_status 或 read_file 查看）".into());
        }
        Ok(cap(diff, MAX_DIFF_CHARS))
    }
}

pub struct GitLogTool;

impl Tool for GitLogTool {
    fn parallel_safe(&self) -> bool {
        true
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "git_log".into(),
            description: "查看最近的提交历史（单行格式：hash + 标题）。可用 path 只看涉及某文件的提交。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "count": {"type": "integer", "description": "条数，默认 20，最大 100"},
                    "path": {"type": "string", "description": "只看涉及该文件/目录的提交（可选）"}
                },
                "required": []
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        require_repo(workspace)?;
        let count = input["count"].as_u64().unwrap_or(20).clamp(1, MAX_LOG_COUNT);
        let count_arg = count.to_string();
        let mut args: Vec<&str> = vec!["log", "--oneline", "--decorate", "-n", &count_arg];
        let path = input["path"].as_str();
        if let Some(path) = path {
            git::validate_git_path(path)?;
            args.push("--");
            args.push(path);
        }
        let log = git::run_git(workspace, &args)?;
        if log.trim().is_empty() {
            return Ok("没有匹配的提交".into());
        }
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run_git;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        run_git(ws, &["init", "-b", "main"]).unwrap();
        run_git(ws, &["config", "user.email", "t@t.dev"]).unwrap();
        run_git(ws, &["config", "user.name", "t"]).unwrap();
        std::fs::write(ws.join("a.txt"), "hello\n").unwrap();
        run_git(ws, &["add", "."]).unwrap();
        run_git(ws, &["commit", "-m", "init commit"]).unwrap();
        dir
    }

    #[test]
    fn git_status_tool_reports_branch_and_files() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        let out = GitStatusTool.run(dir.path(), &json!({})).unwrap();
        assert!(out.contains("分支: main"));
        assert!(out.contains("M a.txt"));
    }

    #[test]
    fn git_diff_tool_shows_unified_diff() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        let out = GitDiffTool.run(dir.path(), &json!({"path": "a.txt"})).unwrap();
        assert!(out.contains("-hello"));
        assert!(out.contains("+changed"));
    }

    #[test]
    fn git_log_tool_lists_commits() {
        let dir = init_repo();
        let out = GitLogTool.run(dir.path(), &json!({"count": 5})).unwrap();
        assert!(out.contains("init commit"));
    }

    #[test]
    fn tools_reject_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(GitStatusTool.run(dir.path(), &json!({})).is_err());
    }
}
