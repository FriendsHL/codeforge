use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::Watcher;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

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
pub fn set_workspace(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WorkspaceInfo, String> {
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
    *state.workspace.lock().unwrap() = Some(canonical.clone());
    state.permissions.reset(); // 换项目后重置"全部允许"
    start_watcher(&canonical, app, &state)?;
    Ok(info)
}

/// 监听工作区文件变化，节流后向前端发 workspace-fs-changed 事件
fn start_watcher(workspace: &Path, app: AppHandle, state: &State<'_, AppState>) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            // .git 内部的索引/锁文件变化极频繁，仅放行 HEAD（感知分支切换）
            let relevant = event.paths.iter().any(|p| {
                let s = p.to_string_lossy();
                !s.contains("/.git/") || s.ends_with("/.git/HEAD")
            });
            if relevant {
                let _ = tx.send(());
            }
        }
    })
    .map_err(|e| format!("启动文件监听失败: {e}"))?;

    watcher
        .watch(workspace, notify::RecursiveMode::Recursive)
        .map_err(|e| format!("监听目录失败: {e}"))?;
    *state.watcher.lock().unwrap() = Some(watcher); // 旧 watcher 随之 drop 停止

    std::thread::spawn(move || {
        // 节流：收到事件后静默 500ms 合并后续抖动，再通知前端
        while rx.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(500));
            while rx.try_recv().is_ok() {}
            if app.emit("workspace-fs-changed", ()).is_err() {
                break;
            }
        }
    });
    Ok(())
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
