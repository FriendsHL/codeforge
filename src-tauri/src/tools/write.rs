//! 写工具：write_file / edit_file。plan() 预演 diff 供审批，run() 真正落盘。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use similar::TextDiff;

use super::registry::{Tool, ApprovalPlan};
use crate::llm::types::ToolSpec;

/// 写路径校验：不能用 canonicalize（新文件还不存在），改为校验父目录 + 路径成分
fn resolve_write_path(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.is_empty() || rel.starts_with('/') || rel.starts_with('~') {
        return Err("path 必须是相对工作区根目录的相对路径".into());
    }
    if rel.split('/').any(|seg| seg == "..") {
        return Err("path 不允许包含 ..".into());
    }
    Ok(workspace.join(rel))
}

fn unified_diff(path: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

pub struct WriteFileTool;

/// 计算 write_file 的目标内容与现状
fn write_file_parts(workspace: &Path, input: &Value) -> Result<(PathBuf, String, String, String), String> {
    let rel = input["path"].as_str().ok_or("缺少 path 参数")?;
    let content = input["content"].as_str().ok_or("缺少 content 参数")?;
    let path = resolve_write_path(workspace, rel)?;
    let old = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| format!("读取原文件失败: {e}"))?
    } else {
        String::new()
    };
    Ok((path, rel.to_string(), old, content.to_string()))
}

impl Tool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "创建新文件，或完全覆盖一个已有文件的内容。修改已有文件的局部时应优先用 edit_file。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对工作区根目录的文件路径"},
                    "content": {"type": "string", "description": "文件完整内容"}
                },
                "required": ["path", "content"]
            }),
        }
    }

    fn plan(&self, workspace: &Path, input: &Value) -> Result<Option<ApprovalPlan>, String> {
        let (_, rel, old, new) = write_file_parts(workspace, input)?;
        Ok(Some(ApprovalPlan { diff: unified_diff(&rel, &old, &new), summary: rel }))
    }

    fn affected_paths(&self, input: &Value) -> Vec<String> {
        input["path"].as_str().map(|p| vec![p.to_string()]).unwrap_or_default()
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let (path, rel, old, new) = write_file_parts(workspace, input)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        std::fs::write(&path, &new).map_err(|e| format!("写入失败: {e}"))?;
        Ok(if old.is_empty() {
            format!("已创建 {rel}（{} 行）", new.lines().count())
        } else {
            format!("已覆盖 {rel}（{} 行）", new.lines().count())
        })
    }
}

pub struct EditFileTool;

/// 计算 edit_file 的替换结果
fn edit_file_parts(workspace: &Path, input: &Value) -> Result<(PathBuf, String, String, String), String> {
    let rel = input["path"].as_str().ok_or("缺少 path 参数")?;
    let old_string = input["old_string"].as_str().ok_or("缺少 old_string 参数")?;
    let new_string = input["new_string"].as_str().ok_or("缺少 new_string 参数")?;
    let replace_all = input["replace_all"].as_bool().unwrap_or(false);
    if old_string.is_empty() {
        return Err("old_string 不能为空".into());
    }
    if old_string == new_string {
        return Err("old_string 与 new_string 相同，无需修改".into());
    }

    let path = resolve_write_path(workspace, rel)?;
    let old = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;

    let count = old.matches(old_string).count();
    if count == 0 {
        return Err("找不到 old_string，请用 read_file 确认文件当前内容（注意空格和缩进需完全一致）".into());
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old_string 出现了 {count} 次，无法唯一定位。请提供更长的上下文，或设 replace_all=true 全部替换"
        ));
    }

    let new = if replace_all {
        old.replace(old_string, new_string)
    } else {
        old.replacen(old_string, new_string, 1)
    };
    Ok((path, rel.to_string(), old, new))
}

impl Tool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file".into(),
            description: "对已有文件做精确字符串替换。old_string 必须与文件内容逐字符一致（含空格缩进）且唯一；不唯一时提供更长上下文或 replace_all=true。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "相对工作区根目录的文件路径"},
                    "old_string": {"type": "string", "description": "要替换的原文（需逐字符精确匹配且唯一）"},
                    "new_string": {"type": "string", "description": "替换后的新文本"},
                    "replace_all": {"type": "boolean", "description": "true 时替换所有出现，默认 false"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    fn plan(&self, workspace: &Path, input: &Value) -> Result<Option<ApprovalPlan>, String> {
        let (_, rel, old, new) = edit_file_parts(workspace, input)?;
        Ok(Some(ApprovalPlan { diff: unified_diff(&rel, &old, &new), summary: rel }))
    }

    fn affected_paths(&self, input: &Value) -> Vec<String> {
        input["path"].as_str().map(|p| vec![p.to_string()]).unwrap_or_default()
    }

    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let (path, rel, _, new) = edit_file_parts(workspace, input)?;
        std::fs::write(&path, &new).map_err(|e| format!("写入失败: {e}"))?;
        Ok(format!("已修改 {rel}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_file_plan_shows_new_file_diff() {
        let dir = tempfile::tempdir().unwrap();
        let plan = WriteFileTool
            .plan(dir.path(), &json!({"path": "a.txt", "content": "hello\n"}))
            .unwrap()
            .unwrap();
        assert_eq!(plan.summary, "a.txt");
        assert!(plan.diff.contains("+hello"));
    }

    #[test]
    fn write_file_creates_nested_file() {
        let dir = tempfile::tempdir().unwrap();
        WriteFileTool
            .run(dir.path(), &json!({"path": "src/deep/a.txt", "content": "x\n"}))
            .unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("src/deep/a.txt")).unwrap(), "x\n");
    }

    #[test]
    fn edit_file_replaces_unique_string() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {\n    old();\n}\n").unwrap();
        let plan = EditFileTool
            .plan(dir.path(), &json!({"path": "a.rs", "old_string": "old()", "new_string": "new()"}))
            .unwrap()
            .unwrap();
        assert!(plan.diff.contains("-    old();"));
        assert!(plan.diff.contains("+    new();"));

        EditFileTool
            .run(dir.path(), &json!({"path": "a.rs", "old_string": "old()", "new_string": "new()"}))
            .unwrap();
        assert!(std::fs::read_to_string(dir.path().join("a.rs")).unwrap().contains("new()"));
    }

    #[test]
    fn edit_file_rejects_ambiguous_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x\nx\n").unwrap();
        let err = EditFileTool
            .plan(dir.path(), &json!({"path": "a.txt", "old_string": "x", "new_string": "y"}))
            .unwrap_err();
        assert!(err.contains("无法唯一定位"));
    }

    #[test]
    fn write_path_rejects_escape() {
        let dir = tempfile::tempdir().unwrap();
        assert!(WriteFileTool
            .plan(dir.path(), &json!({"path": "../evil.txt", "content": ""}))
            .is_err());
        assert!(WriteFileTool
            .plan(dir.path(), &json!({"path": "/etc/evil", "content": ""}))
            .is_err());
    }
}
