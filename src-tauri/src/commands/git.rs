use serde::Serialize;
use tauri::State;

use crate::git;
use crate::AppState;

const MAX_UI_DIFF_CHARS: usize = 60_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChange {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOverview {
    pub branch: String,
    pub changes: Vec<GitChange>,
}

/// UI 一次拉全：分支 + 改动列表。非 git 仓库返回 None。
#[tauri::command]
pub fn git_overview(state: State<'_, AppState>) -> Result<Option<GitOverview>, String> {
    let Some(workspace) = state.workspace.lock().unwrap().clone() else {
        return Ok(None);
    };
    if !git::is_repo(&workspace) {
        return Ok(None);
    }
    let branch = git::current_branch(&workspace)?;
    let changes = git::status_entries(&workspace)?
        .into_iter()
        .map(|e| GitChange { path: e.path, status: e.status })
        .collect();
    Ok(Some(GitOverview { branch, changes }))
}

/// 改动列表里点单个文件看 diff（未跟踪文件由前端走文件预览，不进这里）
#[tauri::command]
pub fn git_file_diff(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let workspace = state
        .workspace
        .lock()
        .unwrap()
        .clone()
        .ok_or("未打开工作区")?;
    git::validate_git_path(&path)?;

    let mut diff = git::run_git(&workspace, &["diff", "--", &path])?;
    if diff.trim().is_empty() {
        // 改动可能已暂存
        diff = git::run_git(&workspace, &["diff", "--staged", "--", &path])?;
    }
    if diff.trim().is_empty() {
        return Ok("(无 diff 内容)".into());
    }
    if diff.chars().count() > MAX_UI_DIFF_CHARS {
        let shown: String = diff.chars().take(MAX_UI_DIFF_CHARS).collect();
        diff = format!("{shown}\n…[diff 过大已截断]");
    }
    Ok(diff)
}
