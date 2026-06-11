//! 只读文件工具：read_file / list_dir

use std::path::Path;

use serde_json::{json, Value};

use super::{registry::Tool, resolve_in_workspace};
use crate::llm::types::ToolSpec;

const DEFAULT_READ_LINES: usize = 500;
const MAX_READ_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 500;

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "读取工作区内某个文本文件的内容（带行号）。文件大时分页：用 offset/limit 读取指定范围。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对工作区根目录的文件路径"},
                    "offset": {"type": "integer", "description": "起始行号（1-based，默认 1）"},
                    "limit": {"type": "integer", "description": "读取行数（默认 500，最大 2000）"}
                },
                "required": ["path"]
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let rel = input["path"].as_str().ok_or("缺少 path 参数")?;
        let path = resolve_in_workspace(workspace, rel)?;
        if !path.is_file() {
            return Err(format!("不是文件: {rel}"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("非文本文件（UTF-8 解码失败）: {rel}"))?;

        let offset = input["offset"].as_u64().unwrap_or(1).max(1) as usize;
        let limit = (input["limit"].as_u64().unwrap_or(DEFAULT_READ_LINES as u64) as usize)
            .min(MAX_READ_LINES);

        let total = text.lines().count();
        let mut out = String::new();
        for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
            let line: String = if line.chars().count() > MAX_LINE_CHARS {
                let truncated: String = line.chars().take(MAX_LINE_CHARS).collect();
                format!("{truncated} …[line truncated]")
            } else {
                line.to_string()
            };
            out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
        }
        if offset - 1 + limit < total {
            out.push_str(&format!(
                "…[truncated: 共 {total} 行，本次显示到第 {} 行，继续读请用 offset={}]\n",
                offset - 1 + limit,
                offset + limit
            ));
        }
        if out.is_empty() {
            out = "(空文件或 offset 超出范围)".into();
        }
        Ok(out)
    }
}

pub struct ListDirTool;

impl Tool for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "列出工作区内某个目录的直接子项（目录带 / 后缀），遵循 .gitignore。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对路径，省略表示工作区根目录"}
                },
                "required": []
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let rel = input["path"].as_str().unwrap_or(".");
        let path = resolve_in_workspace(workspace, rel)?;
        if !path.is_dir() {
            return Err(format!("不是目录: {rel}"));
        }

        let mut entries: Vec<(bool, String)> = Vec::new();
        let walker = ignore::WalkBuilder::new(&path)
            .max_depth(Some(1))
            .hidden(true)
            .git_ignore(true)
            .build();
        for entry in walker.flatten() {
            if entry.path() == path {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push((is_dir, name));
        }
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        if entries.is_empty() {
            return Ok("(空目录)".into());
        }
        Ok(entries
            .into_iter()
            .map(|(is_dir, name)| if is_dir { format!("{name}/") } else { name })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();
        dir
    }

    #[test]
    fn read_file_returns_numbered_lines() {
        let dir = workspace();
        let ws = dir.path().canonicalize().unwrap();
        let out = ReadFileTool
            .run(&ws, &json!({"path": "src/main.rs"}))
            .unwrap();
        assert!(out.contains("1\tfn main() {"));
        assert!(out.contains("2\t    println!"));
    }

    #[test]
    fn read_file_paginates() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().canonicalize().unwrap();
        let many: String = (1..=100).map(|i| format!("line{i}\n")).collect();
        std::fs::write(ws.join("big.txt"), many).unwrap();
        let out = ReadFileTool
            .run(&ws, &json!({"path": "big.txt", "offset": 10, "limit": 5}))
            .unwrap();
        assert!(out.contains("10\tline10"));
        assert!(out.contains("14\tline14"));
        assert!(!out.contains("15\tline15"));
        assert!(out.contains("truncated"));
    }

    #[test]
    fn list_dir_marks_directories() {
        let dir = workspace();
        let ws = dir.path().canonicalize().unwrap();
        let out = ListDirTool.run(&ws, &json!({})).unwrap();
        assert!(out.contains("src/"));
        assert!(out.contains("README.md"));
    }
}
