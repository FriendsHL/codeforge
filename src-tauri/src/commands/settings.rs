use crate::config;

#[tauri::command]
pub fn set_api_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return config::delete_api_key();
    }
    config::set_api_key(key)
}

#[tauri::command]
pub fn has_api_key() -> Result<bool, String> {
    Ok(config::get_api_key()?.is_some())
}
