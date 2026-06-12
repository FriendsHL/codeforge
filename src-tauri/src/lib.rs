mod agent;
mod commands;
mod config;
mod git;
mod llm;
mod mcp;
mod pty;
mod security;
mod session;
mod skills;
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
    /// 停止按钮的取消标志（一次只有一个活动回合）
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
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

            // MCP：先 manage 空状态保证 UI 可查询，连接在后台线程做（server 握手可能秒级）
            let config_path = data_dir.join("mcp.json");
            app.manage(commands::mcp::McpState {
                manager: std::sync::Mutex::new(Arc::new(mcp::McpManager::default())),
                config_path: config_path.clone(),
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let manager = Arc::new(mcp::McpManager::load(&config_path));
                *handle.state::<commands::mcp::McpState>().manager.lock().unwrap() = manager;
            });
            Ok(())
        })
        .manage(AppState {
            workspace: Mutex::new(None),
            tools: Arc::new(ToolRegistry::builtin()),
            permissions: Arc::new(PermissionManager::default()),
            watcher: Mutex::new(None),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::chat::approve_permission,
            commands::chat::stop_generation,
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
            commands::session::list_projects,
            commands::session::remove_project,
            commands::browser::probe_url,
            commands::browser::browser_show,
            commands::browser::browser_bounds,
            commands::browser::browser_close,
            commands::browser::browser_history,
            commands::mcp::mcp_status,
            commands::mcp::mcp_reload,
            commands::mcp::mcp_config_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
