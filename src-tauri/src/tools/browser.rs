//! browser_open 工具：agent 在界面的内嵌浏览器面板里给用户展示页面

use std::path::Path;

use serde_json::{json, Value};
use tauri::Emitter;

use super::registry::Tool;
use crate::llm::types::ToolSpec;

pub struct BrowserOpenTool {
    pub app: tauri::AppHandle,
}

impl Tool for BrowserOpenTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "browser_open".into(),
            description: "在用户界面的内嵌浏览器面板中打开一个 URL（演示页面、预览本地 dev server 的效果）。注意：本工具只负责给用户看，不返回页面内容；需要读取网页内容时用 web_fetch。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "要打开的完整 URL（http/https）"}
                },
                "required": ["url"]
            }),
        }
    }

    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, _workspace: &Path, input: &Value) -> Result<String, String> {
        let url = input["url"].as_str().ok_or("缺少 url 参数")?;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("url 必须以 http:// 或 https:// 开头".into());
        }
        // 前端收到事件后打开浏览器面板并导航（面板挂载时会定位子 webview）
        self.app
            .emit("browser-open", url.to_string())
            .map_err(|e| format!("通知界面失败: {e}"))?;
        Ok(format!("已在用户的浏览器面板中打开 {url}"))
    }
}
