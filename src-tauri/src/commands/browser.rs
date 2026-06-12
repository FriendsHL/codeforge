use std::time::Duration;

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
