use tauri::State;

use crate::session::{ProjectMeta, SessionMeta, SessionStore};

pub struct SessionState(pub SessionStore);

#[tauri::command]
pub fn list_sessions(state: State<'_, SessionState>) -> Result<Vec<SessionMeta>, String> {
    state.0.list()
}

#[tauri::command]
pub fn create_session(
    title: String,
    workspace_root: Option<String>,
    state: State<'_, SessionState>,
) -> Result<SessionMeta, String> {
    let title = if title.trim().is_empty() { "新会话".to_string() } else { title };
    state.0.create(&title, workspace_root.as_deref())
}

#[tauri::command]
pub fn list_projects(state: State<'_, SessionState>) -> Result<Vec<ProjectMeta>, String> {
    state.0.list_projects()
}

#[tauri::command]
pub fn remove_project(root: String, state: State<'_, SessionState>) -> Result<(), String> {
    state.0.remove_project(&root)
}

#[tauri::command]
pub fn rename_session(id: i64, title: String, state: State<'_, SessionState>) -> Result<(), String> {
    state.0.rename(id, &title)
}

#[tauri::command]
pub fn delete_session(id: i64, state: State<'_, SessionState>) -> Result<(), String> {
    state.0.delete(id)
}

#[tauri::command]
pub fn load_session_items(id: i64, state: State<'_, SessionState>) -> Result<String, String> {
    state.0.load_items(id)
}

#[tauri::command]
pub fn save_session_items(
    id: i64,
    items: String,
    state: State<'_, SessionState>,
) -> Result<(), String> {
    state.0.save_items(id, &items)
}
