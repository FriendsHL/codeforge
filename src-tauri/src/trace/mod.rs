//! 本地 trace：OTel span 结构（遵循 GenAI semantic conventions 的属性命名），
//! 逐行追加写入 <app_data>/traces/<session_id>.jsonl。
//! 全部本地、无上报；删除会话时一并清理。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 非加密随机即可：纳秒时间 × 进程内计数器混合
fn gen_hex_id(bytes: usize) -> String {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = (now_unix_nanos() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ counter.rotate_left(17);
    let mixed2 = mixed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (counter << 32);
    let mut hex = format!("{mixed:016x}{mixed2:016x}");
    hex.truncate(bytes * 2);
    hex
}

pub struct TraceWriter {
    file: Mutex<File>,
    pub trace_id: String,
}

impl TraceWriter {
    /// 每次 send_message（一个用户回合）= 一条 trace
    pub fn open(traces_dir: &Path, session_id: i64) -> Result<Self, String> {
        std::fs::create_dir_all(traces_dir).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(traces_dir.join(format!("{session_id}.jsonl")))
            .map_err(|e| format!("打开 trace 文件失败: {e}"))?;
        Ok(Self { file: Mutex::new(file), trace_id: gen_hex_id(16) })
    }

    /// 预留 span id（先有 id 供子 span 引用，span 本身结束时再落盘）
    pub fn reserve_span_id(&self) -> String {
        gen_hex_id(8)
    }

    /// 写一条 span（OTLP JSON 风格的扁平记录）
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &self,
        span_id: &str,
        name: &str,
        parent_span_id: Option<&str>,
        start_unix_nano: u128,
        end_unix_nano: u128,
        attributes: Value,
        error: Option<&str>,
    ) {
        let record = json!({
            "trace_id": self.trace_id,
            "span_id": span_id,
            "parent_span_id": parent_span_id,
            "name": name,
            "kind": "SPAN_KIND_INTERNAL",
            "start_time_unix_nano": start_unix_nano.to_string(),
            "end_time_unix_nano": end_unix_nano.to_string(),
            "status": {
                "code": if error.is_some() { "STATUS_CODE_ERROR" } else { "STATUS_CODE_OK" },
                "message": error,
            },
            "attributes": attributes,
        });
        if let Ok(line) = serde_json::to_string(&record) {
            let mut file = self.file.lock().unwrap();
            let _ = writeln!(file, "{line}");
        }
    }

    /// 便捷形式：现场生成 id 并落盘
    pub fn span(
        &self,
        name: &str,
        parent_span_id: Option<&str>,
        start_unix_nano: u128,
        attributes: Value,
        error: Option<&str>,
    ) -> String {
        let span_id = self.reserve_span_id();
        self.emit(&span_id, name, parent_span_id, start_unix_nano, now_unix_nanos(), attributes, error);
        span_id
    }
}

/// 删除会话时清理对应 trace 文件
pub fn remove_trace(traces_dir: &Path, session_id: i64) {
    let _ = std::fs::remove_file(traces_dir.join(format!("{session_id}.jsonl")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_otel_style_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let writer = TraceWriter::open(dir.path(), 42).unwrap();
        assert_eq!(writer.trace_id.len(), 32);

        let root = writer.reserve_span_id();
        let child_start = now_unix_nanos();
        writer.span(
            "chat doubao-seed-2.0-pro",
            Some(&root),
            child_start,
            json!({"gen_ai.system": "ark", "gen_ai.usage.input_tokens": 1200}),
            None,
        );
        writer.emit(&root, "agent.run", None, child_start, now_unix_nanos(), json!({}), None);

        let content = std::fs::read_to_string(dir.path().join("42.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let child: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(child["parent_span_id"], root.as_str());
        assert_eq!(child["attributes"]["gen_ai.usage.input_tokens"], 1200);
        assert_eq!(child["status"]["code"], "STATUS_CODE_OK");
        assert_eq!(child["trace_id"], writer.trace_id.as_str());

        remove_trace(dir.path(), 42);
        assert!(!dir.path().join("42.jsonl").exists());
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(gen_hex_id(8)));
        }
    }
}
