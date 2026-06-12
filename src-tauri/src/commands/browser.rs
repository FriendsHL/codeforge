//! 内嵌浏览器：Tauri 原生子 WebView（不是 iframe）。
//! 选这条路的原因：release 页面跑在 tauri:// 自定义协议里，WKWebView 拒绝内嵌
//! http iframe；且 iframe 受 X-Frame-Options 限制。原生子 webview 两个问题都没有。

use std::time::Duration;

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};

const BROWSER_LABEL: &str = "embedded-browser";

fn parse_url(url: &str) -> Result<tauri::Url, String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("URL 无效: {e}"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("只支持 http/https".into());
    }
    Ok(parsed)
}

/// 在指定区域显示浏览器（不存在则创建子 webview，存在则导航 + 调整位置）
#[tauri::command]
pub fn browser_show(
    app: AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let parsed = parse_url(&url)?;

    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview.navigate(parsed).map_err(|e| e.to_string())?;
        webview
            .set_position(LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
        webview
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = app.get_window("main").ok_or("主窗口不存在")?;
    window
        .add_child(
            WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(parsed)),
            LogicalPosition::new(x, y),
            LogicalSize::new(width, height),
        )
        .map_err(|e| format!("创建浏览器失败: {e}"))?;
    Ok(())
}

/// 面板移动/缩放时同步子 webview 的位置
#[tauri::command]
pub fn browser_bounds(app: AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview
            .set_position(LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
        webview
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_close(app: AppHandle) -> Result<(), String> {
    if let Some(webview) = app.get_webview(BROWSER_LABEL) {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 浏览器面板导航前的连通性探测（区分「服务没起」和「页面空白」）
#[tauri::command]
pub async fn probe_url(url: String) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .danger_accept_invalid_certs(true) // 本地 dev server 常用自签证书
        .build()
    else {
        return false;
    };
    client.get(&url).send().await.is_ok()
}
