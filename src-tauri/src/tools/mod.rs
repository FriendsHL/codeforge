pub mod agent_roles;
pub mod bash;
pub mod browser;
pub mod diagnostics;
pub mod fs;
pub mod git;
pub mod mcp_adapter;
pub mod registry;
pub mod remember;
pub mod research;
pub mod search;
pub mod todo;
pub mod web;
pub mod write;

use std::path::{Path, PathBuf};

/// 把工具入参里的相对路径解析到工作区内，拒绝越界访问
pub fn resolve_in_workspace(workspace: &Path, rel: &str) -> Result<PathBuf, String> {
    let joined = if rel.is_empty() || rel == "." {
        workspace.to_path_buf()
    } else {
        workspace.join(rel)
    };
    let canonical = joined
        .canonicalize()
        .map_err(|e| format!("路径不存在或不可访问: {rel} ({e})"))?;
    if !canonical.starts_with(workspace) {
        return Err(format!("路径越界，禁止访问工作区之外: {rel}"));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_escape() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace = workspace.path().canonicalize().unwrap();
        let err = resolve_in_workspace(&workspace, "../..").unwrap_err();
        assert!(err.contains("越界"));
    }

    #[test]
    fn resolves_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        std::fs::write(workspace.join("a.txt"), "hi").unwrap();
        let resolved = resolve_in_workspace(&workspace, "a.txt").unwrap();
        assert!(resolved.ends_with("a.txt"));
    }
}
