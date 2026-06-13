//! Agent 角色系统：预设「角色」= 专属 system prompt + 工具白名单。
//! 两种用法：① 会话驱动（用户在顶栏选一个角色，整轮对话用它的人设+工具）；
//!          ② 子 agent 预设（主 agent 用 spawn_subagents 派发带角色的子任务）。
//!
//! 角色来源（同名时用户覆盖内置）：
//!   - 内置 4 个（research/product/dev/review），编译进二进制
//!   - <workspace>/.codeforge/agents/<name>/AGENT.md  项目级
//!   - ~/.codeforge/agents/<name>/AGENT.md            全局
//! 文件格式同 skill：YAML frontmatter(name/description/tools) + 正文(system prompt)。

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AgentRole {
    pub name: String,
    pub description: String,
    /// 角色专属 system prompt（拼到基础 prompt 之前）
    pub system_prompt: String,
    /// 允许使用的工具名白名单；空 = 不限制（全部可用）
    pub tools: Vec<String>,
    /// 来源：「项目」/「全局」/「内置」，用于 list_agents 透明展示
    pub source: &'static str,
}

impl AgentRole {
    /// 该工具是否被本角色允许（空白名单=全允许）
    pub fn allows(&self, tool_name: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == tool_name)
    }
}

const BUILTIN: &[(&str, &str)] = &[
    ("research", include_str!("builtin/research.md")),
    ("product", include_str!("builtin/product.md")),
    ("dev", include_str!("builtin/dev.md")),
    ("review", include_str!("builtin/review.md")),
    ("test", include_str!("builtin/test.md")),
];

/// 解析 AGENT.md：frontmatter(name/description/tools) + 正文。tools 为 "*"/空表示不限。
fn parse(content: &str, source: &'static str) -> Option<AgentRole> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = String::new();
    let mut tools: Vec<String> = Vec::new();
    let mut body_start = 0usize;
    // 先逐行吃 frontmatter，记下正文起点
    let mut consumed = 1; // 已读掉首行 ---
    for line in content.lines().skip(1) {
        consumed += 1;
        let t = line.trim();
        if t == "---" {
            body_start = consumed;
            break;
        }
        if let Some(v) = t.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = t.strip_prefix("description:") {
            description = v.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(v) = t.strip_prefix("tools:") {
            let v = v.trim();
            if v != "*" && !v.is_empty() {
                tools = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
        }
    }
    let system_prompt = content.lines().skip(body_start).collect::<Vec<_>>().join("\n").trim().to_string();
    Some(AgentRole { name: name?, description, system_prompt, tools, source })
}

fn global_agents_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codeforge/agents"))
}

fn scan_dir(root: &Path, source: &'static str, out: &mut Vec<AgentRole>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let md = entry.path().join("AGENT.md");
        if !md.is_file() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&md) {
            if let Some(role) = parse(&content, source) {
                if !out.iter().any(|r| r.name == role.name) {
                    out.push(role);
                }
            }
        }
    }
}

/// 所有可用角色：用户文件优先（项目 > 全局 > 内置），同名覆盖。
pub fn discover(workspace: Option<&Path>) -> Vec<AgentRole> {
    discover_in(workspace, global_agents_dir().as_deref())
}

fn discover_in(workspace: Option<&Path>, global: Option<&Path>) -> Vec<AgentRole> {
    let mut roles = Vec::new();
    if let Some(ws) = workspace {
        scan_dir(&ws.join(".codeforge/agents"), "项目", &mut roles);
    }
    if let Some(g) = global {
        scan_dir(g, "全局", &mut roles);
    }
    // 内置兜底（同名不覆盖用户的）
    for (name, content) in BUILTIN {
        if roles.iter().any(|r| &r.name == name) {
            continue;
        }
        if let Some(role) = parse(content, "内置") {
            roles.push(role);
        }
    }
    roles
}

/// 角色配置文件路径：<dir>/<name>/AGENT.md
fn role_file(scope: &str, workspace: Option<&Path>, name: &str) -> Result<PathBuf, String> {
    let base = match scope {
        "project" => workspace
            .ok_or("项目级角色需要先打开工作区；或用 scope=global")?
            .join(".codeforge/agents"),
        _ => global_agents_dir().ok_or("无法定位 HOME 目录")?,
    };
    Ok(base.join(name).join("AGENT.md"))
}

/// 把角色名规范成安全的目录名（防路径穿越）
fn sanitize_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty()
        || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("角色名只能是字母/数字/-/_，且非空".into());
    }
    Ok(n.to_string())
}

/// 创建或覆盖一个角色（写 AGENT.md，即时生效、跨会话存活）。scope: project|global。
pub fn save_role(
    scope: &str,
    workspace: Option<&Path>,
    name: &str,
    description: &str,
    tools: &[String],
    system_prompt: &str,
) -> Result<String, String> {
    let name = sanitize_name(name)?;
    let path = role_file(scope, workspace, &name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建角色目录失败: {e}"))?;
    }
    let tools_line = if tools.is_empty() { "*".to_string() } else { tools.join(", ") };
    let content = format!(
        "---\nname: {name}\ndescription: {}\ntools: {tools_line}\n---\n{}\n",
        description.trim(),
        system_prompt.trim()
    );
    std::fs::write(&path, content).map_err(|e| format!("写角色失败: {e}"))?;
    Ok(path.display().to_string())
}

/// 删除一个角色配置文件（仅删用户文件；内置角色随后会以兜底身份重新出现）
pub fn delete_role(scope: &str, workspace: Option<&Path>, name: &str) -> Result<(), String> {
    let name = sanitize_name(name)?;
    let path = role_file(scope, workspace, &name)?;
    if !path.is_file() {
        return Err(format!("没找到 {scope} 级角色文件：{name}"));
    }
    std::fs::remove_file(&path).map_err(|e| format!("删除失败: {e}"))?;
    Ok(())
}

/// 首次运行把内置角色落成真实文件到 ~/.codeforge/agents，之后用户删/改不再覆盖。
/// 用 .seeded 哨兵记录已播种，尊重用户后续的删除。
pub fn seed_builtins() {
    let Some(dir) = global_agents_dir() else { return };
    let sentinel = dir.join(".seeded");
    if sentinel.exists() {
        return;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (name, content) in BUILTIN {
        let path = dir.join(name).join("AGENT.md");
        if path.exists() {
            continue;
        }
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let _ = std::fs::write(&path, content);
    }
    let _ = std::fs::write(&sentinel, "codeForge 已把内置角色落地为可编辑文件，删此文件可重新播种\n");
}

/// 按名取角色
pub fn resolve(workspace: Option<&Path>, name: &str) -> Option<AgentRole> {
    discover(workspace).into_iter().find(|r| r.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_roles_parse() {
        let roles = discover_in(None, Some(Path::new("/nonexistent")));
        assert_eq!(roles.len(), 5); // research/product/dev/review/test
        let dev = roles.iter().find(|r| r.name == "dev").unwrap();
        assert!(dev.tools.is_empty(), "dev tools=* 应不限制");
        assert!(dev.allows("edit_file"));
        assert_eq!(dev.source, "内置");

        let review = roles.iter().find(|r| r.name == "review").unwrap();
        assert!(review.allows("git_diff"));
        assert!(!review.allows("edit_file"), "review 不应允许改代码");
        assert!(!review.system_prompt.is_empty());

        let test = roles.iter().find(|r| r.name == "test").unwrap();
        assert!(test.allows("bash"), "test 角色要能跑测试");
    }

    #[test]
    fn user_file_overrides_builtin() {
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join(".codeforge/agents/dev");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("AGENT.md"),
            "---\nname: dev\ndescription: 我的定制 dev\ntools: read_file\n---\n自定义提示",
        )
        .unwrap();

        let dev = resolve_in(Some(ws.path()), "dev").unwrap();
        assert_eq!(dev.description, "我的定制 dev");
        assert_eq!(dev.tools, vec!["read_file".to_string()]);
        assert!(!dev.allows("bash"));
    }

    fn resolve_in(workspace: Option<&Path>, name: &str) -> Option<AgentRole> {
        discover_in(workspace, Some(Path::new("/nonexistent")))
            .into_iter()
            .find(|r| r.name == name)
    }

    #[test]
    fn parses_tools_list_and_star() {
        let star = parse("---\nname: a\ndescription: d\ntools: *\n---\nbody", "内置").unwrap();
        assert!(star.tools.is_empty());
        let list = parse("---\nname: b\ndescription: d\ntools: read_file, grep\n---\nbody", "项目").unwrap();
        assert_eq!(list.tools, vec!["read_file".to_string(), "grep".to_string()]);
        assert_eq!(list.source, "项目");
    }

    #[test]
    fn save_then_resolve_roundtrip() {
        let ws = tempfile::tempdir().unwrap();
        let path = save_role(
            "project",
            Some(ws.path()),
            "my-auditor",
            "安全审计角色",
            &["read_file".to_string(), "grep".to_string()],
            "你只做安全审计。",
        )
        .unwrap();
        assert!(path.ends_with("my-auditor/AGENT.md"));

        let role = discover_in(Some(ws.path()), Some(Path::new("/nonexistent")))
            .into_iter()
            .find(|r| r.name == "my-auditor")
            .expect("应能发现刚保存的角色");
        assert_eq!(role.description, "安全审计角色");
        assert_eq!(role.tools, vec!["read_file".to_string(), "grep".to_string()]);
        assert!(role.allows("grep") && !role.allows("bash"));
        assert!(role.system_prompt.contains("安全审计"));
    }

    #[test]
    fn save_overrides_builtin_then_delete_restores() {
        let ws = tempfile::tempdir().unwrap();
        // 覆盖内置 review
        save_role("project", Some(ws.path()), "review", "我的 review", &[], "随便").unwrap();
        let overridden = discover_in(Some(ws.path()), Some(Path::new("/nonexistent")))
            .into_iter()
            .find(|r| r.name == "review")
            .unwrap();
        assert_eq!(overridden.source, "项目");
        assert_eq!(overridden.description, "我的 review");

        // 删除后内置 review 兜底回归
        delete_role("project", Some(ws.path()), "review").unwrap();
        let restored = discover_in(Some(ws.path()), Some(Path::new("/nonexistent")))
            .into_iter()
            .find(|r| r.name == "review")
            .unwrap();
        assert_eq!(restored.source, "内置");
    }

    #[test]
    fn sanitize_rejects_path_traversal() {
        assert!(save_role("global", None, "../evil", "d", &[], "p").is_err());
        assert!(save_role("global", None, "a/b", "d", &[], "p").is_err());
    }
}
