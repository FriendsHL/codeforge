use keyring::Entry;

const SERVICE: &str = "com.codeforge.desktop";
pub const ANTHROPIC_KEY: &str = "anthropic_api_key";

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|e| e.to_string())
}

pub fn set_key(account: &str, key: &str) -> Result<(), String> {
    entry(account)?.set_password(key).map_err(|e| e.to_string())
}

pub fn get_key(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete_key(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// 兼容旧调用
pub fn set_api_key(key: &str) -> Result<(), String> {
    set_key(ANTHROPIC_KEY, key)
}

pub fn get_api_key() -> Result<Option<String>, String> {
    get_key(ANTHROPIC_KEY)
}

pub fn delete_api_key() -> Result<(), String> {
    delete_key(ANTHROPIC_KEY)
}
