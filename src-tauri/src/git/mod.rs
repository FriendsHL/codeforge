//! git 能力：封装 git CLI（只读）。选 CLI 而非 libgit2：本机必有 git、输出稳定、省重依赖。

use std::path::Path;
use std::process::Command;

pub fn run_git(workspace: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 git: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("git {} 执行失败", args.join(" "))
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn is_repo(workspace: &Path) -> bool {
    workspace.join(".git").exists()
}

pub fn current_branch(workspace: &Path) -> Result<String, String> {
    run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeEntry {
    /// 相对工作区根目录的路径（rename 取新路径）
    pub path: String,
    /// 单字符状态：M 修改 / A 新增 / D 删除 / R 重命名 / ? 未跟踪
    pub status: String,
}

pub fn status_entries(workspace: &Path) -> Result<Vec<ChangeEntry>, String> {
    Ok(parse_porcelain(&run_git(workspace, &["status", "--porcelain"])?))
}

fn parse_porcelain(output: &str) -> Vec<ChangeEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let x = line.as_bytes()[0] as char; // index 区状态
        let y = line.as_bytes()[1] as char; // 工作区状态
        let raw_path = &line[3..];
        // rename 格式: "R  old -> new"，取新路径
        let path = raw_path
            .split_once(" -> ")
            .map(|(_, new)| new)
            .unwrap_or(raw_path)
            .trim_matches('"')
            .to_string();
        let status = if x == '?' || y == '?' {
            '?'
        } else if x == 'R' || y == 'R' {
            'R'
        } else if y != ' ' {
            y
        } else {
            x
        };
        entries.push(ChangeEntry { path, status: status.to_string() });
    }
    entries
}

/// 工具入参里的 git 路径校验：只接受不含 .. 的相对路径
/// （不能用 canonicalize——被删除的文件路径已不存在）
pub fn validate_git_path(path: &str) -> Result<(), String> {
    if path.starts_with('/') || path.starts_with('~') {
        return Err("git 路径必须是相对工作区根目录的相对路径".into());
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err("git 路径不允许包含 ..".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        run_git(ws, &["init", "-b", "main"]).unwrap();
        run_git(ws, &["config", "user.email", "t@t.dev"]).unwrap();
        run_git(ws, &["config", "user.name", "t"]).unwrap();
        std::fs::write(ws.join("a.txt"), "hello\n").unwrap();
        run_git(ws, &["add", "."]).unwrap();
        run_git(ws, &["commit", "-m", "init"]).unwrap();
        dir
    }

    #[test]
    fn reports_branch_and_changes() {
        let dir = init_repo();
        let ws = dir.path();
        assert_eq!(current_branch(ws).unwrap(), "main");

        std::fs::write(ws.join("a.txt"), "changed\n").unwrap(); // M
        std::fs::write(ws.join("new.txt"), "new\n").unwrap(); // ?

        let entries = status_entries(ws).unwrap();
        assert!(entries.contains(&ChangeEntry { path: "a.txt".into(), status: "M".into() }));
        assert!(entries.contains(&ChangeEntry { path: "new.txt".into(), status: "?".into() }));
    }

    #[test]
    fn parses_rename_and_staged_states() {
        let parsed = parse_porcelain("R  old.rs -> new.rs\nA  added.rs\n D deleted.rs\nMM both.rs\n");
        assert_eq!(parsed[0], ChangeEntry { path: "new.rs".into(), status: "R".into() });
        assert_eq!(parsed[1], ChangeEntry { path: "added.rs".into(), status: "A".into() });
        assert_eq!(parsed[2], ChangeEntry { path: "deleted.rs".into(), status: "D".into() });
        assert_eq!(parsed[3], ChangeEntry { path: "both.rs".into(), status: "M".into() });
    }

    #[test]
    fn validates_git_paths() {
        assert!(validate_git_path("src/main.rs").is_ok());
        assert!(validate_git_path("/etc/passwd").is_err());
        assert!(validate_git_path("../outside").is_err());
        assert!(validate_git_path("a/../../b").is_err());
    }
}
