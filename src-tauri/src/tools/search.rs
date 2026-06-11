//! 搜索工具：glob（找文件）/ grep（搜内容）

use std::path::Path;

use serde_json::{json, Value};

use super::{registry::Tool, resolve_in_workspace};
use crate::llm::types::ToolSpec;

const MAX_GLOB_RESULTS: usize = 300;
const MAX_GREP_MATCHES: usize = 200;
const MAX_GREP_FILE_BYTES: u64 = 1024 * 1024;
const MAX_MATCH_LINE_CHARS: usize = 300;

pub struct GlobTool;

impl Tool for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".into(),
            description: "按 glob 模式查找文件（如 **/*.rs、src/**/*.ts），返回相对路径列表，遵循 .gitignore。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "glob 模式，相对工作区根目录"}
                },
                "required": ["pattern"]
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let pattern = input["pattern"].as_str().ok_or("缺少 pattern 参数")?;
        let matcher = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| format!("glob 模式无效: {e}"))?
            .compile_matcher();

        let mut results = Vec::new();
        let walker = ignore::WalkBuilder::new(workspace)
            .hidden(true)
            .git_ignore(true)
            .build();
        for entry in walker.flatten() {
            if results.len() >= MAX_GLOB_RESULTS {
                results.push(format!("…[truncated: 超过 {MAX_GLOB_RESULTS} 个结果]"));
                break;
            }
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if let Ok(rel) = entry.path().strip_prefix(workspace) {
                if matcher.is_match(rel) {
                    results.push(rel.to_string_lossy().to_string());
                }
            }
        }
        if results.is_empty() {
            return Ok(format!("没有匹配 {pattern} 的文件"));
        }
        Ok(results.join("\n"))
    }
}

pub struct GrepTool;

impl Tool for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "在工作区内按正则表达式搜索文件内容，返回 路径:行号:匹配行。可用 path 限定子目录、include 过滤文件名。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "正则表达式"},
                    "path": {"type": "string", "description": "限定搜索的子目录（可选）"},
                    "include": {"type": "string", "description": "文件名 glob 过滤，如 *.java（可选）"}
                },
                "required": ["pattern"]
            }),
        }
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let pattern = input["pattern"].as_str().ok_or("缺少 pattern 参数")?;
        let regex = regex::Regex::new(pattern).map_err(|e| format!("正则无效: {e}"))?;

        let root = match input["path"].as_str() {
            Some(rel) => resolve_in_workspace(workspace, rel)?,
            None => workspace.to_path_buf(),
        };
        let include = match input["include"].as_str() {
            Some(g) => Some(
                globset::GlobBuilder::new(g)
                    .build()
                    .map_err(|e| format!("include 模式无效: {e}"))?
                    .compile_matcher(),
            ),
            None => None,
        };

        let mut matches = Vec::new();
        let mut truncated = false;
        let walker = ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(true)
            .build();
        'outer: for entry in walker.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if let Some(matcher) = &include {
                if !matcher.is_match(entry.file_name()) {
                    continue;
                }
            }
            if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_GREP_FILE_BYTES {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue; // 二进制或非 UTF-8，跳过
            };
            let rel = entry
                .path()
                .strip_prefix(workspace)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();
            for (line_no, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    let shown: String = if line.chars().count() > MAX_MATCH_LINE_CHARS {
                        line.chars().take(MAX_MATCH_LINE_CHARS).collect::<String>() + "…"
                    } else {
                        line.to_string()
                    };
                    matches.push(format!("{rel}:{}:{}", line_no + 1, shown.trim_end()));
                    if matches.len() >= MAX_GREP_MATCHES {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }

        if matches.is_empty() {
            return Ok(format!("没有匹配 {pattern} 的内容"));
        }
        if truncated {
            matches.push(format!("…[truncated: 超过 {MAX_GREP_MATCHES} 条匹配，建议缩小范围]"));
        }
        Ok(matches.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/util")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("src/util/helper.rs"), "pub fn helper() {}\n").unwrap();
        std::fs::write(dir.path().join("notes.md"), "TODO: helper docs\n").unwrap();
        dir
    }

    #[test]
    fn glob_finds_nested_files() {
        let dir = workspace();
        let ws = dir.path().canonicalize().unwrap();
        let out = GlobTool.run(&ws, &json!({"pattern": "**/*.rs"})).unwrap();
        assert!(out.contains("src/main.rs"));
        assert!(out.contains("src/util/helper.rs"));
        assert!(!out.contains("notes.md"));
    }

    #[test]
    fn grep_reports_path_line_and_text() {
        let dir = workspace();
        let ws = dir.path().canonicalize().unwrap();
        let out = GrepTool.run(&ws, &json!({"pattern": "helper"})).unwrap();
        assert!(out.contains("src/util/helper.rs:1:pub fn helper() {}"));
        assert!(out.contains("notes.md:1:TODO: helper docs"));
    }

    #[test]
    fn grep_respects_include_filter() {
        let dir = workspace();
        let ws = dir.path().canonicalize().unwrap();
        let out = GrepTool
            .run(&ws, &json!({"pattern": "helper", "include": "*.md"}))
            .unwrap();
        assert!(out.contains("notes.md"));
        assert!(!out.contains("helper.rs"));
    }
}
