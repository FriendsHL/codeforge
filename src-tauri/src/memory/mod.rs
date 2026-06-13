//! 记忆系统 阶段1：CLAUDE.md 式持久记忆，跨会话存活。
//! 两级：项目级 <workspace>/.codeforge/MEMORY.md（随项目走、可进 git）
//!       全局级 ~/.codeforge/MEMORY.md（用户偏好等，跨项目）
//! 启动时整段注入 system prompt；agent 用 remember 工具主动追加。
//! 阶段2 再加：六信号评分晋升、向量/FTS 检索、后台提炼。

use std::path::{Path, PathBuf};

const MAX_INJECT_CHARS: usize = 12_000; // 注入 prompt 的记忆总量上限，防爆

fn project_memory_path(workspace: &Path) -> PathBuf {
    workspace.join(".codeforge/MEMORY.md")
}

fn global_memory_path() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codeforge/MEMORY.md"))
}

fn read_trimmed(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let t = content.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// system prompt 的「长期记忆」段；无记忆返回 None
pub fn prompt_section(workspace: Option<&Path>) -> Option<String> {
    let mut blocks = Vec::new();
    if let Some(global) = global_memory_path() {
        if let Some(c) = read_trimmed(&global) {
            blocks.push(format!("[全局记忆 ~/.codeforge/MEMORY.md]\n{c}"));
        }
    }
    if let Some(workspace) = workspace {
        if let Some(c) = read_trimmed(&project_memory_path(workspace)) {
            blocks.push(format!("[项目记忆 .codeforge/MEMORY.md]\n{c}"));
        }
    }
    if blocks.is_empty() {
        return None;
    }
    let mut body = blocks.join("\n\n");
    if body.chars().count() > MAX_INJECT_CHARS {
        body = body.chars().take(MAX_INJECT_CHARS).collect::<String>()
            + "\n…[记忆过长已截断，完整内容见 MEMORY.md]";
    }
    Some(format!(
        "## 长期记忆（跨会话持久）\n\
涉及用户偏好、项目约定、已踩过的坑时，先参考下面的记忆；学到值得长期记住的新事实，用 remember 工具记下来。\n\n{body}"
    ))
}

/// 追加一条记忆。scope: "project" | "global"。返回写入的文件路径。
pub fn append(
    workspace: Option<&Path>,
    scope: &str,
    category: &str,
    content: &str,
) -> Result<String, String> {
    let path = match scope {
        "global" => global_memory_path().ok_or("无法定位 HOME 目录")?,
        _ => {
            let ws = workspace.ok_or("项目记忆需要先打开工作区；或用 scope=global")?;
            project_memory_path(ws)
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建记忆目录失败: {e}"))?;
    }
    // 简洁的 markdown 条目：分类小标题下追加一条
    let entry = format!("\n## {}\n- {}\n", category.trim(), content.trim());
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let header = if existing.trim().is_empty() {
        "# codeForge 记忆\n\n> 跨会话持久的事实/偏好/约定。可手工编辑。\n"
    } else {
        ""
    };
    std::fs::write(&path, format!("{header}{existing}{entry}"))
        .map_err(|e| format!("写记忆失败: {e}"))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_project_memory() {
        let ws = tempfile::tempdir().unwrap();
        let p = append(Some(ws.path()), "project", "项目约定", "后端用 Rust，前端 React").unwrap();
        assert!(p.ends_with(".codeforge/MEMORY.md"));

        let section = prompt_section(Some(ws.path())).unwrap();
        assert!(section.contains("长期记忆"));
        assert!(section.contains("后端用 Rust"));
        assert!(section.contains("项目约定"));
    }

    #[test]
    fn no_memory_no_section() {
        let ws = tempfile::tempdir().unwrap();
        // 隔离 HOME，避免读到真实全局记忆
        std::env::set_var("HOME", ws.path());
        assert!(prompt_section(Some(ws.path())).is_none());
    }

    #[test]
    fn project_scope_requires_workspace() {
        assert!(append(None, "project", "x", "y").is_err());
    }

    #[test]
    fn multiple_appends_accumulate() {
        let ws = tempfile::tempdir().unwrap();
        append(Some(ws.path()), "project", "偏好", "用中文回答").unwrap();
        append(Some(ws.path()), "project", "约定", "测试必须先跑通").unwrap();
        let section = prompt_section(Some(ws.path())).unwrap();
        assert!(section.contains("用中文回答"));
        assert!(section.contains("测试必须先跑通"));
        // 文件有 header
        let content = std::fs::read_to_string(ws.path().join(".codeforge/MEMORY.md")).unwrap();
        assert!(content.contains("# codeForge 记忆"));
    }
}
