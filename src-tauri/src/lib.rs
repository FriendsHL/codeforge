mod agent;
mod commands;
mod config;
mod git;
mod llm;
mod pty;
mod security;
mod session;
mod tools;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use security::PermissionManager;
use tools::registry::ToolRegistry;

pub struct AppState {
    pub workspace: Mutex<Option<PathBuf>>,
    pub tools: Arc<ToolRegistry>,
    pub permissions: Arc<PermissionManager>,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // SQLite 放 app data 目录（~/Library/Application Support/com.codeforge.desktop）
            use tauri::Manager;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = session::SessionStore::open(&data_dir.join("codeforge.db"))
                .map_err(std::io::Error::other)?;
            app.manage(commands::session::SessionState(store));
            Ok(())
        })
        .manage(AppState {
            workspace: Mutex::new(None),
            tools: Arc::new(ToolRegistry::builtin()),
            permissions: Arc::new(PermissionManager::default()),
            watcher: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::chat::approve_permission,
            commands::settings::set_api_key,
            commands::settings::has_api_key,
            commands::settings::set_provider_key,
            commands::settings::provider_key_status,
            commands::workspace::set_workspace,
            commands::workspace::read_dir_tree,
            commands::workspace::read_file_preview,
            commands::git::git_overview,
            commands::git::git_file_diff,
            commands::session::list_sessions,
            commands::session::create_session,
            commands::session::rename_session,
            commands::session::delete_session,
            commands::session::load_session_items,
            commands::session::save_session_items,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
