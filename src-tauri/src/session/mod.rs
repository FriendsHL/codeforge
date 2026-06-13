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
    /// 会话所属的项目根目录（纯聊天会话为 None）
    pub workspace_root: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    pub root: String,
    pub name: String,
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
            );
            CREATE TABLE IF NOT EXISTS projects (
                root TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                last_opened_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            );",
        )
        .map_err(|e| format!("初始化表失败: {e}"))?;
        // v1.2 迁移：会话关联项目（列已存在时报错可忽略）
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN workspace_root TEXT", []);
        // v4-8 迁移：会话级 token 统计（累计花费/缓存/上下文），JSON blob
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN stats TEXT", []);
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn list(&self) -> Result<Vec<SessionMeta>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, title, updated_at, workspace_root FROM sessions ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    workspace_root: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn create(&self, title: &str, workspace_root: Option<&str>) -> Result<SessionMeta, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (title, workspace_root) VALUES (?1, ?2)",
            rusqlite::params![title, workspace_root],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        let updated_at: String = conn
            .query_row("SELECT updated_at FROM sessions WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(SessionMeta {
            id,
            title: title.to_string(),
            updated_at,
            workspace_root: workspace_root.map(String::from),
        })
    }

    /// 记录/刷新最近打开的项目
    pub fn upsert_project(&self, root: &str, name: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (root, name) VALUES (?1, ?2)
                 ON CONFLICT(root) DO UPDATE SET last_opened_at = datetime('now', 'localtime')",
                rusqlite::params![root, name],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectMeta>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT root, name FROM projects ORDER BY last_opened_at DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectMeta { root: row.get(0)?, name: row.get(1)? })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn remove_project(&self, root: &str) -> Result<(), String> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM projects WHERE root = ?1", [root])
            .map_err(|e| e.to_string())?;
        Ok(())
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

    /// 读取会话级 token 统计（JSON），无记录时返回 "{}"
    pub fn load_stats(&self, id: i64) -> Result<String, String> {
        let stats: Option<String> = self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT stats FROM sessions WHERE id = ?1", [id], |r| r.get(0))
            .map_err(|e| format!("会话不存在: {e}"))?;
        Ok(stats.unwrap_or_else(|| "{}".into()))
    }

    /// 保存会话级 token 统计（JSON object）。不刷新 updated_at，避免把会话顶到列表最前。
    pub fn save_stats(&self, id: i64, stats_json: &str) -> Result<(), String> {
        let parsed: serde_json::Value =
            serde_json::from_str(stats_json).map_err(|e| format!("stats 不是合法 JSON: {e}"))?;
        if !parsed.is_object() {
            return Err("stats 必须是 JSON 对象".into());
        }
        self.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET stats = ?1 WHERE id = ?2",
                rusqlite::params![stats_json, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
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
        let a = store.create("会话 A", Some("/tmp/proj")).unwrap();
        let _b = store.create("会话 B", None).unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed
            .iter()
            .any(|s| s.workspace_root.as_deref() == Some("/tmp/proj")));

        store.rename(a.id, "改名了").unwrap();
        assert!(store.list().unwrap().iter().any(|s| s.title == "改名了"));

        store.delete(a.id).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn save_and_load_items_roundtrip() {
        let (_dir, store) = store();
        let s = store.create("t", None).unwrap();
        let items = r#"[{"kind":"msg","role":"user","content":"你好"}]"#;
        store.save_items(s.id, items).unwrap();
        assert_eq!(store.load_items(s.id).unwrap(), items);
    }

    #[test]
    fn save_and_load_stats_roundtrip() {
        let (_dir, store) = store();
        let s = store.create("t", None).unwrap();
        // 未保存时默认空对象
        assert_eq!(store.load_stats(s.id).unwrap(), "{}");
        let stats = r#"{"sessionInputTokens":1200,"sessionTokens":340,"sessionCacheTokens":800,"contextTokens":1500}"#;
        store.save_stats(s.id, stats).unwrap();
        assert_eq!(store.load_stats(s.id).unwrap(), stats);
    }

    #[test]
    fn save_stats_rejects_non_object() {
        let (_dir, store) = store();
        let s = store.create("t", None).unwrap();
        assert!(store.save_stats(s.id, "[]").is_err());
        assert!(store.save_stats(s.id, "not json").is_err());
    }

    #[test]
    fn save_rejects_invalid_json() {
        let (_dir, store) = store();
        let s = store.create("t", None).unwrap();
        assert!(store.save_items(s.id, "not json").is_err());
        assert!(store.save_items(s.id, r#"{"kind":"msg"}"#).is_err());
    }

    #[test]
    fn projects_upsert_and_list() {
        let (_dir, store) = store();
        store.upsert_project("/a", "a").unwrap();
        store.upsert_project("/b", "b").unwrap();
        store.upsert_project("/a", "a").unwrap(); // 重开 → 刷新时间，不重复
        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
        store.remove_project("/a").unwrap();
        assert_eq!(store.list_projects().unwrap().len(), 1);
    }
}
