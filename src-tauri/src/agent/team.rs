//! 异步团队编排（createTeam）：主 agent 把任务派给后台子 agent，立即拿到 task_id，
//! 之后用 team_status 查进度/收结果。任务注册表存 AppState，跨回合存活。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub title: String,
    pub role: Option<String>,
    pub status: TaskStatus,
    /// 完成=书面汇报，失败=错误信息，运行中=None
    pub result: Option<String>,
}

/// 后台任务注册表（线程安全，存 AppState）
#[derive(Default)]
pub struct TeamRegistry {
    tasks: Mutex<HashMap<String, TaskRecord>>,
    counter: AtomicUsize,
}

impl TeamRegistry {
    pub fn next_id(&self) -> String {
        format!("t{}", self.counter.fetch_add(1, Ordering::SeqCst))
    }

    pub fn start(&self, id: &str, title: &str, role: Option<String>) {
        self.tasks.lock().unwrap().insert(
            id.to_string(),
            TaskRecord {
                id: id.to_string(),
                title: title.to_string(),
                role,
                status: TaskStatus::Running,
                result: None,
            },
        );
    }

    pub fn finish(&self, id: &str, result: Result<String, String>) {
        if let Some(rec) = self.tasks.lock().unwrap().get_mut(id) {
            match result {
                Ok(r) => {
                    rec.status = TaskStatus::Done;
                    rec.result = Some(r);
                }
                Err(e) => {
                    rec.status = TaskStatus::Failed;
                    rec.result = Some(e);
                }
            }
        }
    }

    /// 全部任务，按 id 稳定排序
    pub fn snapshot(&self) -> Vec<TaskRecord> {
        let mut v: Vec<TaskRecord> = self.tasks.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// 取指定 id 的任务（保持入参顺序）
    pub fn get(&self, ids: &[String]) -> Vec<TaskRecord> {
        let m = self.tasks.lock().unwrap();
        ids.iter().filter_map(|i| m.get(i).cloned()).collect()
    }

    pub fn running_count(&self) -> usize {
        self.tasks
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_running_to_done() {
        let reg = TeamRegistry::default();
        let id = reg.next_id();
        assert_eq!(id, "t0");
        reg.start(&id, "查文档", Some("research".into()));
        assert_eq!(reg.running_count(), 1);
        assert_eq!(reg.snapshot()[0].status, TaskStatus::Running);

        reg.finish(&id, Ok("结论：可行".into()));
        assert_eq!(reg.running_count(), 0);
        let snap = reg.snapshot();
        assert_eq!(snap[0].status, TaskStatus::Done);
        assert_eq!(snap[0].result.as_deref(), Some("结论：可行"));
    }

    #[test]
    fn failure_recorded() {
        let reg = TeamRegistry::default();
        let id = reg.next_id();
        reg.start(&id, "x", None);
        reg.finish(&id, Err("超时".into()));
        let snap = reg.snapshot();
        assert_eq!(snap[0].status, TaskStatus::Failed);
        assert_eq!(snap[0].result.as_deref(), Some("超时"));
    }

    #[test]
    fn ids_increment_and_get_by_ids() {
        let reg = TeamRegistry::default();
        let a = reg.next_id();
        let b = reg.next_id();
        assert_ne!(a, b);
        reg.start(&a, "A", None);
        reg.start(&b, "B", None);
        let got = reg.get(&[b.clone()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "B");
    }
}
