//! 会话持久化：SQLite 单表。items 列存整个 ChatItem JSON 数组——
//! v1 取「整存整取」的简单方案（对话规模下性能无虞），M5 后如需检索再拆表。

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: i64,
    pub title: String,
    pub updated_at: String,
}

pub struct SessionStore {
    conn: Mutex<Connection>,
}

impl SessionStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("打开数据库失败: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                items TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );",
        )
        .map_err(|e| format!("初始化表失败: {e}"))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn list(&self) -> Result<Vec<SessionMeta>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, title, updated_at FROM sessions ORDER BY updated_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn create(&self, title: &str) -> Result<SessionMeta, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO sessions (title) VALUES (?1)", [title])
            .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        let updated_at: String = conn
            .query_row("SELECT updated_at FROM sessions WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(SessionMeta { id, title: title.to_string(), updated_at })
    }

    pub fn rename(&self, id: i64, title: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET title = ?1 WHERE id = ?2", rusqlite::params![title, id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM sessions WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_items(&self, id: i64) -> Result<String, String> {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT items FROM sessions WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| format!("会话不存在: {e}"))
    }

    pub fn save_items(&self, id: i64, items_json: &str) -> Result<(), String> {
        // 防御：必须是合法 JSON 数组，避免坏数据破坏会话
        let parsed: serde_json::Value =
            serde_json::from_str(items_json).map_err(|e| format!("items 不是合法 JSON: {e}"))?;
        if !parsed.is_array() {
            return Err("items 必须是 JSON 数组".into());
        }
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET items = ?1, updated_at = datetime('now', 'localtime') WHERE id = ?2",
                rusqlite::params![items_json, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("t.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn create_list_rename_delete() {
        let (_dir, store) = store();
        let a = store.create("会话 A").unwrap();
        let _b = store.create("会话 B").unwrap();
        assert_eq!(store.list().unwrap().len(), 2);

        store.rename(a.id, "改名了").unwrap();
        assert!(store.list().unwrap().iter().any(|s| s.title == "改名了"));

        store.delete(a.id).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn save_and_load_items_roundtrip() {
        let (_dir, store) = store();
        let s = store.create("t").unwrap();
        let items = r#"[{"kind":"msg","role":"user","content":"你好"}]"#;
        store.save_items(s.id, items).unwrap();
        assert_eq!(store.load_items(s.id).unwrap(), items);
    }

    #[test]
    fn save_rejects_invalid_json() {
        let (_dir, store) = store();
        let s = store.create("t").unwrap();
        assert!(store.save_items(s.id, "not json").is_err());
        assert!(store.save_items(s.id, r#"{"kind":"msg"}"#).is_err());
    }
}
