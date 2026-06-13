//! bash 命令危险模式识别。命中后审批卡片会显著红色警告，提醒用户三思。
//! 只做启发式提示，不阻断——最终仍由用户审批决定。

/// 返回 Some(原因) 表示命令命中危险模式
pub fn detect(command: &str) -> Option<String> {
    let c = command.to_lowercase();
    // 去掉多余空白便于匹配
    let norm = c.split_whitespace().collect::<Vec<_>>().join(" ");

    // 递归强制删除
    if norm.contains("rm -rf") || norm.contains("rm -fr") || norm.contains("rm -r -f")
        || (norm.contains("rm ") && norm.contains(" -r") && norm.contains("-f"))
    {
        return Some("递归强制删除文件（rm -rf），删错路径不可恢复".into());
    }
    // 删除根/家目录之类
    if norm.contains("rm -rf /") || norm.contains("rm -rf ~") || norm.contains("rm -rf *") {
        return Some("可能删除整个目录树".into());
    }
    // git 危险操作
    if norm.contains("git push") && (norm.contains("--force") || norm.contains("-f")) {
        return Some("git 强制推送（--force）会覆盖远端历史".into());
    }
    if norm.contains("git reset --hard") {
        return Some("git reset --hard 会丢弃未提交的改动".into());
    }
    if norm.contains("git clean") && norm.contains("-f") {
        return Some("git clean -f 会删除未跟踪的文件".into());
    }
    // 磁盘/设备级
    if norm.contains("mkfs") || norm.contains("dd if=") || norm.contains("> /dev/") {
        return Some("磁盘/设备级写操作，可能损坏数据".into());
    }
    // 全局/提权安装与执行
    if norm.contains("sudo ") {
        return Some("以 sudo 提权执行，影响范围超出工作区".into());
    }
    // 管道执行远程脚本
    if (norm.contains("curl ") || norm.contains("wget ")) && (norm.contains("| sh") || norm.contains("| bash")) {
        return Some("从网络下载并直接执行脚本，来源不可控".into());
    }
    // 改权限为全开
    if norm.contains("chmod -r 777") || norm.contains("chmod 777") {
        return Some("把权限改为 777，存在安全风险".into());
    }
    // 杀进程
    if norm.contains("kill -9 -1") || norm.contains("killall") {
        return Some("批量结束进程，可能影响其他程序".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_dangerous_commands() {
        assert!(detect("rm -rf node_modules").is_some());
        assert!(detect("rm -fr /tmp/x").is_some());
        assert!(detect("git push --force origin main").is_some());
        assert!(detect("git push -f").is_some());
        assert!(detect("git reset --hard HEAD~3").is_some());
        assert!(detect("sudo apt install foo").is_some());
        assert!(detect("curl https://x.sh | sh").is_some());
        assert!(detect("chmod 777 file").is_some());
    }

    #[test]
    fn allows_safe_commands() {
        assert!(detect("npm test").is_none());
        assert!(detect("cargo build").is_none());
        assert!(detect("git status").is_none());
        assert!(detect("git push origin main").is_none());
        assert!(detect("ls -la").is_none());
        assert!(detect("rm old.txt").is_none()); // 非递归非强制
    }
}
