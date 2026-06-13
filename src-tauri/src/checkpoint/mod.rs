//! 文件快照 / 回滚：每次写文件工具执行前，把"将被改动的文件的当前内容"存一份快照，
//! 会话里可一键恢复到某次改动之前。快照存 <app_data>/checkpoints/<session_id>/。
//!
//! 设计取舍：只快照"即将被该次操作改动的那些文件"（增量），不是整个工作区——
//! 对 coding agent 的单文件/少量文件改动足够，且省空间。新建的文件记录为"原本不存在"，
//! 回滚时删除它。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 单个文件在某 checkpoint 时刻的前态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    /// 相对工作区根目录的路径
    pub rel_path: String,
    /// 改动前的内容；None 表示该文件当时不存在（回滚 = 删除它）
    pub before: Option<String>,
}

/// 一次写操作前的检查点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    /// 关联的工具调用 id（前端把它挂在对应工具卡片上）
    pub tool_call_id: String,
    /// 人类可读摘要，如 "edit_file src/main.rs"
    pub label: String,
    pub workspace_root: String,
    pub files: Vec<FileSnapshot>,
    /// 标记本检查点是否已被回滚（避免重复）
    #[serde(default)]
    pub reverted: bool,
}

fn session_dir(root: &Path, session_id: i64) -> PathBuf {
    root.join("checkpoints").join(session_id.to_string())
}

fn checkpoint_path(root: &Path, session_id: i64, checkpoint_id: &str) -> PathBuf {
    session_dir(root, session_id).join(format!("{checkpoint_id}.json"))
}

/// 在写操作执行前调用：对涉及的文件拍快照并落盘
pub fn capture(
    app_data: &Path,
    session_id: i64,
    checkpoint_id: &str,
    tool_call_id: &str,
    label: &str,
    workspace_root: &Path,
    rel_paths: &[String],
) -> Result<(), String> {
    let files: Vec<FileSnapshot> = rel_paths
        .iter()
        .map(|rel| {
            let abs = workspace_root.join(rel);
            let before = std::fs::read_to_string(&abs).ok();
            FileSnapshot { rel_path: rel.clone(), before }
        })
        .collect();

    let checkpoint = Checkpoint {
        id: checkpoint_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
        label: label.to_string(),
        workspace_root: workspace_root.display().to_string(),
        files,
        reverted: false,
    };

    let dir = session_dir(app_data, session_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建快照目录失败: {e}"))?;
    let json = serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?;
    std::fs::write(checkpoint_path(app_data, session_id, checkpoint_id), json)
        .map_err(|e| format!("写快照失败: {e}"))
}

fn load(app_data: &Path, session_id: i64, checkpoint_id: &str) -> Result<Checkpoint, String> {
    let content = std::fs::read_to_string(checkpoint_path(app_data, session_id, checkpoint_id))
        .map_err(|e| format!("快照不存在: {e}"))?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

/// 把快照里记录的文件恢复到改动前的状态
pub fn revert(app_data: &Path, session_id: i64, checkpoint_id: &str) -> Result<String, String> {
    let mut checkpoint = load(app_data, session_id, checkpoint_id)?;
    if checkpoint.reverted {
        return Err("该检查点已经回滚过了".into());
    }
    let root = PathBuf::from(&checkpoint.workspace_root);

    let mut restored = 0;
    let mut deleted = 0;
    for file in &checkpoint.files {
        let abs = root.join(&file.rel_path);
        match &file.before {
            Some(content) => {
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&abs, content).map_err(|e| format!("恢复 {} 失败: {e}", file.rel_path))?;
                restored += 1;
            }
            None => {
                // 改动前不存在 → 删除现在这个新文件
                if abs.exists() {
                    std::fs::remove_file(&abs)
                        .map_err(|e| format!("删除 {} 失败: {e}", file.rel_path))?;
                    deleted += 1;
                }
            }
        }
    }

    checkpoint.reverted = true;
    let json = serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?;
    let _ = std::fs::write(checkpoint_path(app_data, session_id, checkpoint_id), json);

    Ok(format!("已回滚「{}」：恢复 {restored} 个文件，删除 {deleted} 个新建文件", checkpoint.label))
}

/// 删除会话时清理其所有快照
pub fn remove_session(app_data: &Path, session_id: i64) {
    let _ = std::fs::remove_dir_all(session_dir(app_data, session_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_and_revert_modified_file() {
        let app_data = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("a.txt"), "原始内容\n").unwrap();

        capture(app_data.path(), 1, "cp1", "tc1", "edit_file a.txt", ws.path(), &["a.txt".into()])
            .unwrap();
        // 模拟改动
        std::fs::write(ws.path().join("a.txt"), "被改坏了\n").unwrap();

        let msg = revert(app_data.path(), 1, "cp1").unwrap();
        assert!(msg.contains("恢复 1"));
        assert_eq!(std::fs::read_to_string(ws.path().join("a.txt")).unwrap(), "原始内容\n");

        // 重复回滚被拒
        assert!(revert(app_data.path(), 1, "cp1").is_err());
    }

    #[test]
    fn revert_deletes_newly_created_file() {
        let app_data = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        // 文件原本不存在
        capture(app_data.path(), 1, "cp1", "tc1", "write_file new.txt", ws.path(), &["new.txt".into()])
            .unwrap();
        std::fs::write(ws.path().join("new.txt"), "新建的\n").unwrap();

        let msg = revert(app_data.path(), 1, "cp1").unwrap();
        assert!(msg.contains("删除 1"));
        assert!(!ws.path().join("new.txt").exists());
    }

    #[test]
    fn remove_session_clears_checkpoints() {
        let app_data = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        capture(app_data.path(), 7, "cp1", "tc1", "x", ws.path(), &[]).unwrap();
        assert!(session_dir(app_data.path(), 7).exists());
        remove_session(app_data.path(), 7);
        assert!(!session_dir(app_data.path(), 7).exists());
    }
}
