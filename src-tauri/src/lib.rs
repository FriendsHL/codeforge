mod commands;
mod config;
mod llm;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::chat::send_message,
            commands::settings::set_api_key,
            commands::settings::has_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
