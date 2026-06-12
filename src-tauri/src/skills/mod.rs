//! 技能系统：SKILL.md 指令包，按需加载（progressive disclosure）。
//! 技能清单（name + description）注入 system prompt，正文由 agent 经 load_skill 工具按需读取。
//!
//! 扫描目录（同名时排前者优先）：
//!   1. <workspace>/.codeforge/skills/<name>/SKILL.md   项目级
//!   2. ~/.codeforge/skills/<name>/SKILL.md             全局（对接 skillForge：把技能目录
//!      软链或拷贝到这里即可，格式同为 SKILL.md + frontmatter）

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    /// SKILL.md 所在目录（正文里可引用同目录的脚本/资源）
    pub dir: PathBuf,
}

fn global_skills_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codeforge/skills"))
}

/// 解析 SKILL.md 的 YAML frontmatter（只取 name / description 两行，避免引入 yaml 依赖）
fn parse_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    Some((name?, description.unwrap_or_default()))
}

fn scan_dir(root: &Path, out: &mut Vec<SkillMeta>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&skill_md) else { continue };
        if let Some((name, description)) = parse_frontmatter(&content) {
            // 同名技能：先扫描到的（项目级）优先
            if !out.iter().any(|s| s.name == name) {
                out.push(SkillMeta { name, description, dir });
            }
        }
    }
}

pub fn discover(workspace: Option<&Path>) -> Vec<SkillMeta> {
    discover_in(workspace, global_skills_dir().as_deref())
}

fn discover_in(workspace: Option<&Path>, global: Option<&Path>) -> Vec<SkillMeta> {
    let mut skills = Vec::new();
    if let Some(workspace) = workspace {
        scan_dir(&workspace.join(".codeforge/skills"), &mut skills);
    }
    if let Some(global) = global {
        scan_dir(global, &mut skills);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

pub fn load(workspace: Option<&Path>, name: &str) -> Result<String, String> {
    let skill = discover(workspace)
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("技能不存在: {name}（用 prompt 里列出的技能名）"))?;
    let content = std::fs::read_to_string(skill.dir.join("SKILL.md"))
        .map_err(|e| format!("读取技能失败: {e}"))?;
    Ok(format!(
        "[技能目录: {}（正文中的相对路径以此为基准）]\n\n{content}",
        skill.dir.display()
    ))
}

/// system prompt 中的技能清单段；无技能时返回 None
pub fn prompt_section(workspace: Option<&Path>) -> Option<String> {
    let skills = discover(workspace);
    if skills.is_empty() {
        return None;
    }
    let list = skills
        .iter()
        .map(|s| format!("- {} ({}/SKILL.md): {}", s.name, s.dir.display(), s.description))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "可用技能（每项为 名称 (SKILL.md 路径): 说明）。任务与某技能匹配时，先用 load_skill 读取其完整指令（也可直接 read_file 读上面的 SKILL.md 路径）再开始干活：\n{list}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(root: &Path, name: &str, desc: &str) {
        let dir = root.join(".codeforge/skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n\n# {name} 的使用步骤\n1. 第一步\n"),
        )
        .unwrap();
    }

    #[test]
    fn discovers_and_loads_workspace_skills() {
        let dir = tempfile::tempdir().unwrap();
        make_skill(dir.path(), "deploy-check", "发布前检查清单");
        make_skill(dir.path(), "api-review", "API 设计评审");

        // 用隔离的全局目录（None），避免被宿主机 ~/.codeforge/skills 干扰
        let skills = discover_in(Some(dir.path()), None);
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "api-review"); // 按名排序

        let body = load(Some(dir.path()), "deploy-check").unwrap();
        assert!(body.contains("deploy-check 的使用步骤"));
        assert!(body.contains("技能目录:"));
    }

    #[test]
    fn workspace_skill_wins_name_clash_over_global() {
        let workspace = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        make_skill(workspace.path(), "deploy-check", "项目级");
        // 全局目录结构没有 .codeforge/skills 前缀，直接放技能目录
        let global_skill = global.path().join("deploy-check");
        std::fs::create_dir_all(&global_skill).unwrap();
        std::fs::write(
            global_skill.join("SKILL.md"),
            "---\nname: deploy-check\ndescription: 全局级\n---\nbody",
        )
        .unwrap();

        let skills = discover_in(Some(workspace.path()), Some(global.path()));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "项目级");
    }

    #[test]
    fn frontmatter_parsing_tolerates_quotes_and_extra_fields() {
        let md = "---\nname: \"x\"\nversion: 1\ndescription: 'y z'\n---\nbody";
        assert_eq!(parse_frontmatter(md), Some(("x".into(), "y z".into())));
        assert_eq!(parse_frontmatter("no frontmatter"), None);
    }
}
