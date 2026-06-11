use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::tools::resolve_in_workspace;
use crate::AppState;

const MAX_PREVIEW_BYTES: usize = 200 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub root: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// 相对工作区根目录的路径（树节点 key）
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub content: String,
    pub truncated: bool,
}

#[tauri::command]
pub fn set_workspace(path: String, state: State<'_, AppState>) -> Result<WorkspaceInfo, String> {
    let canonical = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| format!("目录不可访问: {e}"))?;
    if !canonical.is_dir() {
        return Err("请选择一个目录".into());
    }
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| canonical.display().to_string());
    let info = WorkspaceInfo { root: canonical.display().to_string(), name };
    *state.workspace.lock().unwrap() = Some(canonical);
    Ok(info)
}

#[tauri::command]
pub fn read_dir_tree(path: String, state: State<'_, AppState>) -> Result<Vec<TreeNode>, String> {
    let workspace = state
        .workspace
        .lock()
        .unwrap()
        .clone()
        .ok_or("未打开工作区")?;
    let dir = resolve_in_workspace(&workspace, &path)?;

    let mut nodes = Vec::new();
    let walker = ignore::WalkBuilder::new(&dir)
        .max_depth(Some(1))
        .hidden(true)
        .git_ignore(true)
        .build();
    for entry in walker.flatten() {
        if entry.path() == dir {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let rel = entry
            .path()
            .strip_prefix(&workspace)
            .map_err(|e| e.to_string())?;
        nodes.push(TreeNode {
            path: rel.to_string_lossy().to_string(),
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir,
        });
    }
    nodes.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(nodes)
}

#[tauri::command]
pub fn read_file_preview(path: String, state: State<'_, AppState>) -> Result<FilePreview, String> {
    let workspace = state
        .workspace
        .lock()
        .unwrap()
        .clone()
        .ok_or("未打开工作区")?;
    let file = resolve_in_workspace(&workspace, &path)?;
    if !file.is_file() {
        return Err(format!("不是文件: {path}"));
    }
    let bytes = std::fs::read(&file).map_err(|e| format!("读取失败: {e}"))?;
    let truncated = bytes.len() > MAX_PREVIEW_BYTES;
    let slice = if truncated { &bytes[..MAX_PREVIEW_BYTES] } else { &bytes[..] };
    let content = String::from_utf8_lossy(slice).to_string();
    Ok(FilePreview { content, truncated })
}
