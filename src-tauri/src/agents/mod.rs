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
];

/// 解析 AGENT.md：frontmatter(name/description/tools) + 正文。tools 为 "*"/空表示不限。
fn parse(content: &str) -> Option<AgentRole> {
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
    Some(AgentRole { name: name?, description, system_prompt, tools })
}

fn global_agents_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codeforge/agents"))
}

fn scan_dir(root: &Path, out: &mut Vec<AgentRole>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let md = entry.path().join("AGENT.md");
        if !md.is_file() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&md) {
            if let Some(role) = parse(&content) {
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
        scan_dir(&ws.join(".codeforge/agents"), &mut roles);
    }
    if let Some(g) = global {
        scan_dir(g, &mut roles);
    }
    // 内置兜底（同名不覆盖用户的）
    for (name, content) in BUILTIN {
        if roles.iter().any(|r| &r.name == name) {
            continue;
        }
        if let Some(role) = parse(content) {
            roles.push(role);
        }
    }
    roles
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
        assert_eq!(roles.len(), 4);
        let dev = roles.iter().find(|r| r.name == "dev").unwrap();
        assert!(dev.tools.is_empty(), "dev tools=* 应不限制");
        assert!(dev.allows("edit_file"));

        let review = roles.iter().find(|r| r.name == "review").unwrap();
        assert!(review.allows("git_diff"));
        assert!(!review.allows("edit_file"), "review 不应允许改代码");
        assert!(!review.system_prompt.is_empty());
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
        let star = parse("---\nname: a\ndescription: d\ntools: *\n---\nbody").unwrap();
        assert!(star.tools.is_empty());
        let list = parse("---\nname: b\ndescription: d\ntools: read_file, grep\n---\nbody").unwrap();
        assert_eq!(list.tools, vec!["read_file".to_string(), "grep".to_string()]);
    }
}
