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

/// 后台 agent 的自我身份（让它的 report 工具知道"我是谁"）
#[derive(Debug, Clone)]
pub struct TeamTaskHandle {
    pub id: String,
    pub title: String,
}

/// 子 agent 发给主 agent（协调者）的消息
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub from_id: String,
    pub from_title: String,
    pub content: String,
}

/// 后台任务注册表 + 双向信箱（线程安全，存 AppState）
#[derive(Default)]
pub struct TeamRegistry {
    tasks: Mutex<HashMap<String, TaskRecord>>,
    counter: AtomicUsize,
    /// 子 agent → 主 agent 的消息队列（主 loop 每轮抽取注入）
    inbox: Mutex<Vec<AgentMessage>>,
    /// 主 agent → 各子 agent 的指令信箱（task_id → 指令队列；子 agent 每轮抽取）
    mailboxes: Mutex<HashMap<String, Vec<String>>>,
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

    /// 子 agent 投递一条给协调者的消息
    pub fn post_message(&self, from_id: &str, from_title: &str, content: &str) {
        self.inbox.lock().unwrap().push(AgentMessage {
            from_id: from_id.to_string(),
            from_title: from_title.to_string(),
            content: content.to_string(),
        });
    }

    /// 主 loop 抽取（清空）信箱
    pub fn drain_inbox(&self) -> Vec<AgentMessage> {
        std::mem::take(&mut *self.inbox.lock().unwrap())
    }

    /// 该任务是否在运行中（主 agent 下指令前判断有没有意义）
    pub fn is_running(&self, id: &str) -> bool {
        self.tasks
            .lock()
            .unwrap()
            .get(id)
            .map(|t| t.status == TaskStatus::Running)
            .unwrap_or(false)
    }

    /// 主 agent 给某个子 agent 投递一条指令
    pub fn post_to_agent(&self, id: &str, content: &str) {
        self.mailboxes
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_default()
            .push(content.to_string());
    }

    /// 子 agent 抽取（清空）自己的指令信箱
    pub fn drain_agent_mailbox(&self, id: &str) -> Vec<String> {
        self.mailboxes
            .lock()
            .unwrap()
            .get_mut(id)
            .map(std::mem::take)
            .unwrap_or_default()
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
    fn inbox_post_and_drain() {
        let reg = TeamRegistry::default();
        reg.post_message("t0", "查文档", "发现 X，需要你确认方向");
        reg.post_message("t1", "审代码", "第3处有空指针风险");
        let msgs = reg.drain_inbox();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].from_id, "t0");
        assert!(msgs[1].content.contains("空指针"));
        // 抽取后清空
        assert!(reg.drain_inbox().is_empty());
    }

    #[test]
    fn agent_mailbox_post_and_drain() {
        let reg = TeamRegistry::default();
        let id = reg.next_id();
        reg.start(&id, "开发X", None);
        assert!(reg.is_running(&id));
        reg.post_to_agent(&id, "改用方案B");
        reg.post_to_agent(&id, "记得加测试");
        let msgs = reg.drain_agent_mailbox(&id);
        assert_eq!(msgs, vec!["改用方案B".to_string(), "记得加测试".to_string()]);
        // 抽取后清空；未知任务返回空
        assert!(reg.drain_agent_mailbox(&id).is_empty());
        assert!(reg.drain_agent_mailbox("nope").is_empty());
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
