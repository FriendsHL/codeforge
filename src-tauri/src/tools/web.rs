//! web 工具：web_fetch（抓网页转 Markdown）/ web_search（Tavily 优先，DuckDuckGo 兜底）

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::llm::types::ToolSpec;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const DEFAULT_FETCH_CHARS: usize = 20_000;
const MAX_FETCH_CHARS: usize = 60_000;

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("Mozilla/5.0 (Macintosh) codeforge/1.0")
        .build()
        .map_err(|e| e.to_string())
}

/// HTML → Markdown，并跳过 script/style/nav 等对阅读无意义的标签
fn html_to_markdown(html: &str) -> String {
    let md = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "nav", "footer", "noscript", "svg", "head"])
        .build()
        .convert(html)
        .unwrap_or_else(|_| html.to_string());
    // 折叠 3+ 连续空行为 2 行，markdown 转换常留大量空行
    let mut out = String::with_capacity(md.len());
    let mut blank = 0;
    for line in md.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank <= 2 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn cap_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let shown: String = text.chars().take(limit).collect();
    format!("{shown}\n…[truncated: 内容过长，可调大 max_chars 或换更精确的 URL]")
}

pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "抓取一个 URL 的内容并转成 Markdown（HTML 会保留标题/链接/列表/代码块结构）。适合看文档、issue、报错解释等。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "完整 URL（http/https）"},
                    "max_chars": {"type": "integer", "description": "返回字符上限，默认 20000，最大 60000"}
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
        let max_chars = (input["max_chars"].as_u64().unwrap_or(DEFAULT_FETCH_CHARS as u64)
            as usize)
            .min(MAX_FETCH_CHARS);

        let response = http_client()?
            .get(url)
            .send()
            .map_err(|e| format!("请求失败: {e}"))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let bytes = response.bytes().map_err(|e| format!("读取响应失败: {e}"))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(format!("响应超过 {} MB，拒绝处理", MAX_BODY_BYTES / 1024 / 1024));
        }

        let raw = String::from_utf8_lossy(&bytes);
        let text = if content_type.contains("text/html") {
            html_to_markdown(&raw)
        } else {
            raw.to_string()
        };
        Ok(format!("[{status}] {url}\n\n{}", cap_chars(text.trim(), max_chars)))
    }
}

pub struct WebSearchTool;

#[derive(Debug)]
struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

fn search_tavily(query: &str, api_key: &str) -> Result<Vec<SearchHit>, String> {
    let response = http_client()?
        .post("https://api.tavily.com/search")
        .json(&json!({"api_key": api_key, "query": query, "max_results": 8}))
        .send()
        .map_err(|e| format!("Tavily 请求失败: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Tavily 返回 {}", response.status()));
    }
    let body: Value = response.json().map_err(|e| e.to_string())?;
    let hits = body["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|r| SearchHit {
                    title: r["title"].as_str().unwrap_or("").to_string(),
                    url: r["url"].as_str().unwrap_or("").to_string(),
                    snippet: r["content"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(hits)
}

/// DuckDuckGo HTML 版解析（零 key 兜底）。结果链接形如
/// //duckduckgo.com/l/?uddg=<urlencoded>&rut=…，需解出 uddg。
fn search_duckduckgo(query: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );
    let html = http_client()?
        .get(&url)
        .send()
        .map_err(|e| format!("DuckDuckGo 请求失败: {e}"))?
        .text()
        .map_err(|e| e.to_string())?;
    Ok(parse_ddg_html(&html))
}

fn parse_ddg_html(html: &str) -> Vec<SearchHit> {
    let link_re = regex::Regex::new(
        r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let snippet_re =
        regex::Regex::new(r#"(?s)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#).unwrap();
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    let strip = |s: &str| {
        tag_re
            .replace_all(s, "")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#x27;", "'")
            .replace("&quot;", "\"")
            .trim()
            .to_string()
    };

    let snippets: Vec<String> = snippet_re
        .captures_iter(html)
        .map(|c| strip(&c[1]))
        .collect();

    link_re
        .captures_iter(html)
        .take(8)
        .enumerate()
        .map(|(i, c)| {
            let raw_href = &c[1];
            // 解出 uddg 真实地址
            let url = raw_href
                .split("uddg=")
                .nth(1)
                .and_then(|rest| rest.split('&').next())
                .and_then(|enc| urlencoding::decode(enc).ok())
                .map(|s| s.to_string())
                .unwrap_or_else(|| raw_href.to_string());
            SearchHit {
                title: strip(&c[2]),
                url,
                snippet: snippets.get(i).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

impl Tool for WebSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "联网搜索（查文档、报错信息、库的用法等），返回标题 + 链接 + 摘要，可配合 web_fetch 深入阅读。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"}
                },
                "required": ["query"]
            }),
        }
    }

    fn needs_workspace(&self) -> bool {
        false
    }

    fn run(&self, _workspace: &Path, input: &Value) -> Result<String, String> {
        let query = input["query"].as_str().ok_or("缺少 query 参数")?;

        // Tavily（有 key 时质量更好）→ DuckDuckGo HTML 兜底
        let hits = match std::env::var("TAVILY_API_KEY") {
            Ok(key) if !key.is_empty() => {
                search_tavily(query, &key).or_else(|_| search_duckduckgo(query))?
            }
            _ => search_duckduckgo(query)?,
        };

        if hits.is_empty() {
            return Ok(format!("没有找到「{query}」的结果"));
        }
        Ok(hits
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{}. {}\n   {}\n   {}", i + 1, h.title, h.url, h.snippet))
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ddg_result_html() {
        let html = r##"
        <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftauri.app%2F&amp;rut=abc">Tauri <b>2</b></a>
        <a class="result__snippet" href="#">Build <b>small</b> apps</a>
        "##;
        let hits = parse_ddg_html(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://tauri.app/");
        assert_eq!(hits[0].title, "Tauri 2");
        assert_eq!(hits[0].snippet, "Build small apps");
    }

    #[test]
    fn converts_html_to_markdown() {
        let html = r#"<html><head><style>x{}</style></head><body>
        <h1>标题</h1><p>一段 <a href="https://x.com">链接</a> 文字。</p>
        <ul><li>项一</li><li>项二</li></ul>
        <pre><code>fn main() {}</code></pre>
        <script>alert(1)</script>
        </body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("# 标题"));
        assert!(md.contains("[链接](https://x.com)"));
        assert!(md.contains("项一"));
        assert!(!md.contains("alert(1)"), "script 应被跳过");
        assert!(!md.contains("x{}"), "style 应被跳过");
    }

    #[test]
    fn fetch_rejects_non_http_url() {
        let err = WebFetchTool
            .run(Path::new("/tmp"), &json!({"url": "file:///etc/passwd"}))
            .unwrap_err();
        assert!(err.contains("http"));
    }

    /// 真实网络测试：cargo test live_web -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_web_fetch_example() {
        let out = WebFetchTool
            .run(Path::new("/tmp"), &json!({"url": "https://example.com"}))
            .unwrap();
        assert!(out.contains("Example Domain"));
    }

    #[test]
    #[ignore]
    fn live_web_search_ddg() {
        let out = WebSearchTool
            .run(Path::new("/tmp"), &json!({"query": "tauri 2 documentation"}))
            .unwrap();
        println!("{out}");
        assert!(out.contains("http"));
    }
}
