//! 写操作审批：loop 发起 ask 并挂起等待，前端调 approve_permission 决议

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::oneshot;

/// 审批等待上限：超时按拒绝处理，避免 loop 永久挂起
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct PermissionManager {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    allow_all: AtomicBool,
}

impl PermissionManager {
    /// 注册一个审批请求。必须在向前端发 PermissionAsk 事件**之前**调用，
    /// 否则极快到达的决议会找不到通道（oneshot 会缓存结果，先 resolve 后 wait 是安全的）。
    pub fn register(&self, request_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(request_id.to_string(), tx);
        rx
    }

    /// 等待用户决定，超时按拒绝处理
    pub async fn wait(&self, request_id: &str, rx: oneshot::Receiver<bool>) -> bool {
        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(approved)) => approved,
            _ => {
                self.pending.lock().unwrap().remove(request_id);
                false
            }
        }
    }

    /// 会话级"全部允许"开关已打开时无需弹审批
    pub fn is_allow_all(&self) -> bool {
        self.allow_all.load(Ordering::SeqCst)
    }

    pub fn resolve(&self, request_id: &str, approved: bool, allow_all: bool) -> Result<(), String> {
        if allow_all && approved {
            self.allow_all.store(true, Ordering::SeqCst);
        }
        let sender = self
            .pending
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or("审批请求不存在或已超时")?;
        sender.send(approved).map_err(|_| "审批通道已关闭".to_string())
    }

    /// 切换工作区时重置"全部允许"
    pub fn reset(&self) {
        self.allow_all.store(false, Ordering::SeqCst);
        self.pending.lock().unwrap().clear();
    }

    /// 用户点停止：所有挂起中的审批一律按拒绝决议，立刻解除 loop 的等待
    pub fn deny_all_pending(&self) {
        for (_, sender) in self.pending.lock().unwrap().drain() {
            let _ = sender.send(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approve_resolves_waiting_ask() {
        let pm = PermissionManager::default();
        let rx = pm.register("r1");
        pm.resolve("r1", true, false).unwrap();
        assert!(pm.wait("r1", rx).await);
    }

    #[tokio::test]
    async fn resolve_before_wait_is_safe() {
        // 竞态保护：决议先于 wait 到达，oneshot 缓存结果
        let pm = PermissionManager::default();
        let rx = pm.register("r1");
        pm.resolve("r1", true, false).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(pm.wait("r1", rx).await);
    }

    #[tokio::test]
    async fn allow_all_flag_sticks_until_reset() {
        let pm = PermissionManager::default();
        let rx = pm.register("r1");
        pm.resolve("r1", true, true).unwrap();
        assert!(pm.wait("r1", rx).await);
        assert!(pm.is_allow_all());
        pm.reset();
        assert!(!pm.is_allow_all());
    }

    #[tokio::test]
    async fn deny_resolves_false() {
        let pm = PermissionManager::default();
        let rx = pm.register("r1");
        pm.resolve("r1", false, false).unwrap();
        assert!(!pm.wait("r1", rx).await);
    }
}
