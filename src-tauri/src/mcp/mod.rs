//! 最小 MCP client（stdio 传输，newline-delimited JSON-RPC 2.0）。
//! 只实现工具相关的三步：initialize → tools/list → tools/call。
//! 配置文件：<app_data>/mcp.json，格式：
//! {"servers": {"名字": {"command": "npx", "args": ["-y", "..."], "env": {}}}}

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpConnection {
    pub server_name: String,
    pub tools: Vec<McpToolDef>,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    responses: Mutex<Receiver<Value>>,
    next_id: AtomicI64,
}

impl McpConnection {
    pub fn connect(server_name: &str, config: &McpServerConfig) -> Result<Self, String> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("启动 MCP server 失败 ({}): {e}", config.command))?;

        let stdin = child.stdin.take().ok_or("无法获取 stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取 stdout")?;

        // 读线程：只把「响应」（带 id 且非请求）转给调用方；server 发来的通知/请求忽略
        let (tx, rx) = mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if value.get("id").is_some() && value.get("method").is_none() {
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut conn = Self {
            server_name: server_name.to_string(),
            tools: Vec::new(),
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            responses: Mutex::new(rx),
            next_id: AtomicI64::new(1),
        };

        conn.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "codeforge", "version": "1.0.0"},
            }),
            HANDSHAKE_TIMEOUT,
        )?;
        conn.notify("notifications/initialized")?;

        let listed = conn.request("tools/list", json!({}), HANDSHAKE_TIMEOUT)?;
        conn.tools = listed["tools"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        Some(McpToolDef {
                            name: t["name"].as_str()?.to_string(),
                            description: t["description"].as_str().unwrap_or("").to_string(),
                            input_schema: t
                                .get("inputSchema")
                                .cloned()
                                .unwrap_or(json!({"type": "object", "properties": {}})),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(conn)
    }

    fn write_line(&self, value: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap();
        let line = serde_json::to_string(value).map_err(|e| e.to_string())?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
            .map_err(|e| format!("写入 MCP server 失败: {e}"))
    }

    fn notify(&self, method: &str) -> Result<(), String> {
        self.write_line(&json!({"jsonrpc": "2.0", "method": method}))
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.write_line(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;

        let rx = self.responses.lock().unwrap();
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("MCP {method} 超时（{}s）", timeout.as_secs()))?;
            match rx.recv_timeout(remaining) {
                Ok(response) if response["id"] == json!(id) => {
                    if let Some(error) = response.get("error") {
                        let message = error["message"].as_str().unwrap_or("未知错误");
                        return Err(format!("MCP 错误: {message}"));
                    }
                    return Ok(response["result"].clone());
                }
                Ok(_) => continue, // 串行调用下基本不会发生；保险跳过
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("MCP {method} 超时（{}s）", timeout.as_secs()));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("MCP server 已退出".into());
                }
            }
        }
    }

    pub fn call_tool(&self, tool: &str, arguments: Value) -> Result<String, String> {
        let result = self.request(
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
            CALL_TIMEOUT,
        )?;
        let text = result["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result["isError"].as_bool().unwrap_or(false) {
            return Err(if text.is_empty() { "工具执行失败".into() } else { text });
        }
        Ok(if text.is_empty() {
            serde_json::to_string(&result).unwrap_or_default()
        } else {
            text
        })
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

/// 一个 server 的连接结果（连不上也要进列表给 UI 显示原因）
pub struct McpServerStatus {
    pub name: String,
    pub connection: Result<Arc<McpConnection>, String>,
}

#[derive(Default)]
pub struct McpManager {
    pub servers: Vec<McpServerStatus>,
}

impl McpManager {
    pub fn load(config_path: &Path) -> Self {
        let config: McpConfig = std::fs::read_to_string(config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut servers: Vec<McpServerStatus> = config
            .servers
            .iter()
            .map(|(name, server_config)| McpServerStatus {
                name: name.clone(),
                connection: McpConnection::connect(name, server_config).map(Arc::new),
            })
            .collect();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Self { servers }
    }

    pub fn connections(&self) -> Vec<Arc<McpConnection>> {
        self.servers
            .iter()
            .filter_map(|s| s.connection.as_ref().ok().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用 python3 起一个假 MCP server，跑完整 initialize/list/call 流程
    #[test]
    fn talks_to_fake_mcp_server() {
        let script = r#"
import sys, json
for line in sys.stdin:
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method", "")
    if method == "initialize":
        out = {"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"fake"}}}
    elif method == "tools/list":
        out = {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"echo","description":"回显输入","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]}}
    elif method == "tools/call":
        text = msg["params"]["arguments"]["text"]
        out = {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"echo: "+text}]}}
    elif mid is None:
        continue
    else:
        out = {"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"unknown"}}
    sys.stdout.write(json.dumps(out)+"\n")
    sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fake_mcp.py");
        std::fs::write(&script_path, script).unwrap();

        let config = McpServerConfig {
            command: "python3".into(),
            args: vec![script_path.to_string_lossy().to_string()],
            env: HashMap::new(),
        };
        let conn = McpConnection::connect("fake", &config).unwrap();

        assert_eq!(conn.tools.len(), 1);
        assert_eq!(conn.tools[0].name, "echo");
        assert_eq!(conn.tools[0].description, "回显输入");

        let out = conn.call_tool("echo", json!({"text": "你好 MCP"})).unwrap();
        assert_eq!(out, "echo: 你好 MCP");
    }

    #[test]
    fn manager_reports_failed_servers() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mcp.json");
        std::fs::write(
            &config_path,
            r#"{"servers": {"broken": {"command": "/nonexistent/binary"}}}"#,
        )
        .unwrap();
        let manager = McpManager::load(&config_path);
        assert_eq!(manager.servers.len(), 1);
        assert!(manager.servers[0].connection.is_err());
        assert!(manager.connections().is_empty());
    }

    #[test]
    fn manager_tolerates_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let manager = McpManager::load(&dir.path().join("nope.json"));
        assert!(manager.servers.is_empty());
    }
}
