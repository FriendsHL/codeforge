//! 记忆系统。
//! 阶段1：CLAUDE.md 式持久记忆，跨会话存活。两级存储：
//!   项目级 <workspace>/.codeforge/MEMORY.md（随项目走、可进 git）
//!   全局级 ~/.codeforge/MEMORY.md（用户偏好等，跨项目）
//! 阶段2(v4-8)：在不引入第二数据源的前提下加三件事——
//!   ① 六信号质量评分（纯函数，读时计算，不改文件、不产生 git 噪声）
//!   ② remember 写入前查重，避免记忆膨胀
//!   ③ 相关性检索：记忆超预算时按「与当前问题的相关度 + 质量分 + 新近度」择优注入，
//!      而不是一刀切截断。
//! MEMORY.md 始终是唯一事实来源，用户可手工编辑，分值即时重算。
//! 后续可加：会话→记忆的后台提炼（dreaming）、向量检索。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_INJECT_CHARS: usize = 12_000; // 注入 prompt 的记忆总量上限，防爆
const DUP_THRESHOLD: f32 = 0.6; // 与已有条目相似度 ≥ 此值视为重复
const MIN_KEEP_SCORE: u32 = 25; // 质量分低于此值的记忆视为太弱，不写入
const CORE_SCORE: u32 = 70; // 质量分 ≥ 此值的「核心记忆」，即使与当前问题无关也优先保留

fn project_memory_path(workspace: &Path) -> PathBuf {
    workspace.join(".codeforge/MEMORY.md")
}

fn global_memory_path() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codeforge/MEMORY.md"))
}

fn read_trimmed(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let t = content.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// 一条记忆条目（从 MEMORY.md 解析出来）
#[derive(Debug, Clone)]
pub struct Entry {
    pub category: String,
    pub content: String,
    /// 文件内出现的序号（越大越新，append 把新条目追加在末尾）
    pub order: usize,
    /// 来源标签，用于注入时标明「全局 / 项目」
    pub source: &'static str,
}

/// 解析 MEMORY.md：`## 分类` 标题下的每个 `- 条目` 是一条记忆。
/// 容忍旧格式与手工编辑——非条目行直接忽略。
fn parse_entries(md: &str, source: &'static str, base_order: usize) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut category = "备忘".to_string();
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            category = rest.trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix("- ") {
            let content = rest.trim().to_string();
            if !content.is_empty() {
                entries.push(Entry {
                    category: category.clone(),
                    content,
                    order: base_order + entries.len(),
                    source,
                });
            }
        }
    }
    entries
}

/// 分词：ASCII 词（小写、长度≥2）+ 中文相邻双字（bigram）。
/// 不依赖分词器，对中英混排都能给出可用的重叠度。
fn tokenize(s: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let mut word = String::new();
    let mut cjk: Vec<char> = Vec::new();
    let flush_word = |word: &mut String, tokens: &mut HashSet<String>| {
        if word.chars().count() >= 2 {
            tokens.insert(word.to_lowercase());
        }
        word.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            word.push(ch);
            // 一段连续 ASCII 结束才 flush；这里先攒着
        } else if is_cjk(ch) {
            flush_word(&mut word, &mut tokens);
            cjk.push(ch);
        } else {
            flush_word(&mut word, &mut tokens);
            // 中文 bigram
            for pair in cjk.windows(2) {
                tokens.insert(pair.iter().collect());
            }
            if cjk.len() == 1 {
                tokens.insert(cjk[0].to_string());
            }
            cjk.clear();
        }
    }
    flush_word(&mut word, &mut tokens);
    for pair in cjk.windows(2) {
        tokens.insert(pair.iter().collect());
    }
    if cjk.len() == 1 {
        tokens.insert(cjk[0].to_string());
    }
    tokens
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

/// 两段文本的 Jaccard 相似度（基于 token 集合），0~1
fn similarity(a: &str, b: &str) -> f32 {
    let ta = tokenize(a);
    let tb = tokenize(b);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    inter / union
}

/// 六信号质量评分（0~100），纯函数。用于写入门槛与检索排序。
/// 信号：specificity 具体度 / durability 耐久度(分类) / actionability 可执行性
///       / conciseness 精炼度 / emphasis 用户强调 / uniqueness 唯一性。
/// 前五项靠内容+分类计算；uniqueness 由调用方按上下文另算后叠加。
pub fn quality_score(category: &str, content: &str) -> u32 {
    let c = content.trim();
    let chars = c.chars().count();
    let mut score: i32 = 40; // 基线

    // ① 具体度：含路径/扩展名/数字/反引号/CamelCase/:: 等具体符号
    let specific = c.contains('/')
        || c.contains('`')
        || c.contains("::")
        || c.chars().any(|ch| ch.is_ascii_digit())
        || has_camel_or_dotted(c);
    if specific {
        score += 15;
    }

    // ② 可执行性：含约束/指令性词
    const ACTION_WORDS: &[&str] = &[
        "必须", "不要", "总是", "避免", "优先", "禁止", "应", "需", "记得", "默认",
        "must", "always", "never", "avoid", "prefer", "should",
    ];
    let lc = c.to_lowercase();
    if ACTION_WORDS.iter().any(|w| lc.contains(w)) {
        score += 15;
    }

    // ③ 耐久度：分类决定这条记忆能管多久
    const DURABLE: &[&str] = &["用户偏好", "项目约定", "关键决策", "已知坑", "教训", "约定", "偏好"];
    if DURABLE.iter().any(|d| category.contains(d)) {
        score += 15;
    }

    // ④ 精炼度：太短=琐碎，太长=不像单条事实
    if (8..=120).contains(&chars) {
        score += 10;
    } else if chars > 300 {
        score -= 10;
    } else if chars < 4 {
        score -= 25;
    }

    // ⑤ 用户强调：给了非默认分类，说明经过分门别类
    if !category.trim().is_empty() && category != "备忘" {
        score += 5;
    }

    score.clamp(0, 100) as u32
}

fn has_camel_or_dotted(s: &str) -> bool {
    // foo.bar / FooBar 之类的标识符
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            return true; // CamelCase
        }
        prev_lower = ch.is_ascii_lowercase();
    }
    // 形如 a.b 的点分标识符（排除句号：要求点两侧都是字母数字）
    let bytes: Vec<char> = s.chars().collect();
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] == '.' && bytes[i - 1].is_ascii_alphanumeric() && bytes[i + 1].is_ascii_alphanumeric() {
            return true;
        }
    }
    false
}

/// 与查询的相关度：token 重叠数。无查询时返回 0。
fn relevance(query_tokens: &HashSet<String>, content: &str) -> u32 {
    if query_tokens.is_empty() {
        return 0;
    }
    let et = tokenize(content);
    et.intersection(query_tokens).count() as u32
}

/// 加载某一来源的所有条目
fn load_entries(workspace: Option<&Path>) -> Vec<Entry> {
    let mut all = Vec::new();
    if let Some(global) = global_memory_path() {
        if let Some(c) = read_trimmed(&global) {
            all.extend(parse_entries(&c, "全局", 0));
        }
    }
    if let Some(workspace) = workspace {
        if let Some(c) = read_trimmed(&project_memory_path(workspace)) {
            let base = all.len();
            all.extend(parse_entries(&c, "项目", base));
        }
    }
    all
}

/// 综合排序键：相关度优先，其次质量分，再次新近度。返回越大越该保留。
fn rank_key(e: &Entry, query_tokens: &HashSet<String>, max_order: usize) -> (u32, u32, usize) {
    let rel = relevance(query_tokens, &e.content);
    let q = quality_score(&e.category, &e.content);
    let recency = e.order; // 越大越新
    let _ = max_order;
    (rel, q, recency)
}

/// system prompt 的「长期记忆」段；无记忆返回 None。
/// query 为本轮用户问题（用于相关性检索）；为 None 时退化为按质量+新近度排序。
pub fn prompt_section(workspace: Option<&Path>, query: Option<&str>) -> Option<String> {
    let entries = load_entries(workspace);
    if entries.is_empty() {
        return None;
    }
    let query_tokens = query.map(tokenize).unwrap_or_default();
    let max_order = entries.iter().map(|e| e.order).max().unwrap_or(0);

    // 全部装得下就全注入（无回归）；装不下再择优
    let total: usize = entries.iter().map(|e| e.content.chars().count() + e.category.chars().count() + 8).sum();

    let selected: Vec<&Entry> = if total <= MAX_INJECT_CHARS {
        let mut v: Vec<&Entry> = entries.iter().collect();
        // 仍按排序键排序，让最相关/重要的排在前面
        v.sort_by(|a, b| rank_key(b, &query_tokens, max_order).cmp(&rank_key(a, &query_tokens, max_order)));
        v
    } else {
        select_within_budget(&entries, &query_tokens, max_order)
    };

    render_section(&selected, total > MAX_INJECT_CHARS)
}

/// 超预算时的择优：先保住核心记忆，再用剩余预算按排序键贪心填充。
fn select_within_budget<'a>(
    entries: &'a [Entry],
    query_tokens: &HashSet<String>,
    max_order: usize,
) -> Vec<&'a Entry> {
    let cost = |e: &Entry| e.content.chars().count() + e.category.chars().count() + 8;

    // 候选按排序键降序
    let mut ranked: Vec<&Entry> = entries.iter().collect();
    ranked.sort_by(|a, b| rank_key(b, query_tokens, max_order).cmp(&rank_key(a, query_tokens, max_order)));

    let mut selected: Vec<&Entry> = Vec::new();
    let mut used = 0usize;

    // 第一遍：核心记忆（高质量）优先占坑
    for e in &ranked {
        if quality_score(&e.category, &e.content) >= CORE_SCORE {
            let cst = cost(e);
            if used + cst <= MAX_INJECT_CHARS {
                selected.push(e);
                used += cst;
            }
        }
    }
    // 第二遍：剩余预算按排序键填充（跳过已选）
    for e in &ranked {
        if selected.iter().any(|s| std::ptr::eq(*s, *e)) {
            continue;
        }
        let cst = cost(e);
        if used + cst <= MAX_INJECT_CHARS {
            selected.push(e);
            used += cst;
        }
    }
    // 恢复成排序键顺序，保证展示稳定
    selected.sort_by(|a, b| rank_key(b, query_tokens, max_order).cmp(&rank_key(a, query_tokens, max_order)));
    selected
}

fn render_section(selected: &[&Entry], truncated: bool) -> Option<String> {
    if selected.is_empty() {
        return None;
    }
    let body = selected
        .iter()
        .map(|e| format!("- [{}/{}] {}", e.source, e.category, e.content))
        .collect::<Vec<_>>()
        .join("\n");
    let note = if truncated {
        "\n（记忆较多，已按与当前任务的相关性择优展示；完整记忆见 MEMORY.md）"
    } else {
        ""
    };
    Some(format!(
        "## 长期记忆（跨会话持久）\n\
涉及用户偏好、项目约定、已踩过的坑时，先参考下面的记忆；学到值得长期记住的新事实，用 remember 工具记下来。{note}\n\n{body}"
    ))
}

/// remember 的写入结果
pub enum RememberOutcome {
    Saved { path: String },
    Duplicate { existing: String },
    TooWeak { reason: String },
}

/// 带查重与质量门槛的记忆写入：阶段2 的核心，防止记忆膨胀与低质内容堆积。
pub fn remember(
    workspace: Option<&Path>,
    scope: &str,
    category: &str,
    content: &str,
) -> Result<RememberOutcome, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("content 不能为空".into());
    }

    // 在目标 scope 的文件里查重
    let target = match scope {
        "global" => global_memory_path(),
        _ => workspace.map(project_memory_path),
    };
    if let Some(path) = &target {
        if let Some(md) = read_trimmed(path) {
            for e in parse_entries(&md, "项目", 0) {
                if similarity(&e.content, content) >= DUP_THRESHOLD {
                    return Ok(RememberOutcome::Duplicate { existing: e.content });
                }
            }
        }
    }

    // 质量门槛：太笼统/太琐碎的不收
    let score = quality_score(category, content);
    if score < MIN_KEEP_SCORE {
        return Ok(RememberOutcome::TooWeak {
            reason: format!(
                "这条太笼统或太琐碎（质量分 {score}），没记。值得长期记的应是具体的偏好/约定/决策/踩坑，可补充具体路径、约束或原因后再记。"
            ),
        });
    }

    let path = append(workspace, scope, category, content)?;
    Ok(RememberOutcome::Saved { path })
}

/// 追加一条记忆（低层写入，不做查重/评分）。scope: "project" | "global"。返回写入路径。
pub fn append(
    workspace: Option<&Path>,
    scope: &str,
    category: &str,
    content: &str,
) -> Result<String, String> {
    let path = match scope {
        "global" => global_memory_path().ok_or("无法定位 HOME 目录")?,
        _ => {
            let ws = workspace.ok_or("项目记忆需要先打开工作区；或用 scope=global")?;
            project_memory_path(ws)
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建记忆目录失败: {e}"))?;
    }
    // 简洁的 markdown 条目：分类小标题下追加一条
    let entry = format!("\n## {}\n- {}\n", category.trim(), content.trim());
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let header = if existing.trim().is_empty() {
        "# codeForge 记忆\n\n> 跨会话持久的事实/偏好/约定。可手工编辑。\n"
    } else {
        ""
    };
    std::fs::write(&path, format!("{header}{existing}{entry}"))
        .map_err(|e| format!("写记忆失败: {e}"))?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_project_memory() {
        let ws = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", ws.path()); // 隔离全局记忆
        let p = append(Some(ws.path()), "project", "项目约定", "后端用 Rust，前端 React").unwrap();
        assert!(p.ends_with(".codeforge/MEMORY.md"));

        let section = prompt_section(Some(ws.path()), None).unwrap();
        assert!(section.contains("长期记忆"));
        assert!(section.contains("后端用 Rust"));
        assert!(section.contains("项目约定"));
    }

    #[test]
    fn no_memory_no_section() {
        let ws = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", ws.path());
        assert!(prompt_section(Some(ws.path()), None).is_none());
    }

    #[test]
    fn project_scope_requires_workspace() {
        assert!(append(None, "project", "x", "y").is_err());
    }

    #[test]
    fn parses_entries_with_categories() {
        let md = "# h\n\n## 偏好\n- 用中文回答\n\n## 约定\n- 测试必须先跑通\n";
        let entries = parse_entries(md, "项目", 0);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].category, "偏好");
        assert_eq!(entries[0].content, "用中文回答");
        assert_eq!(entries[1].category, "约定");
        assert!(entries[1].order > entries[0].order);
    }

    #[test]
    fn quality_score_rewards_specific_actionable() {
        // 具体路径 + 约束词 + 好分类 → 高分
        let high = quality_score("项目约定", "改 src/agent/loop_.rs 前必须先 read_file");
        // 笼统、无分类、短 → 低分
        let low = quality_score("备忘", "挺好");
        assert!(high > low);
        assert!(high >= MIN_KEEP_SCORE);
    }

    #[test]
    fn remember_rejects_weak_and_dedupes() {
        let ws = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", ws.path());

        // 太弱：极短无信息
        match remember(Some(ws.path()), "project", "备忘", "嗯").unwrap() {
            RememberOutcome::TooWeak { .. } => {}
            _ => panic!("应判为太弱"),
        }

        // 正常写入
        match remember(Some(ws.path()), "project", "项目约定", "改 loop_.rs 前必须先 read_file 确认现状").unwrap() {
            RememberOutcome::Saved { .. } => {}
            o => panic!("应保存, got {}", matches!(o, RememberOutcome::Saved { .. })),
        }

        // 近似重复：换个说法但语义重叠高 → 查重命中
        match remember(Some(ws.path()), "project", "项目约定", "改 loop_.rs 前必须先 read_file 看清现状").unwrap() {
            RememberOutcome::Duplicate { .. } => {}
            _ => panic!("应判为重复"),
        }
    }

    #[test]
    fn relevance_retrieval_prefers_query_match() {
        let q = tokenize("怎么配置 git 推送");
        let git_entry = "git push 前要先拉取，约定用 rebase";
        let unrelated = "前端组件统一放在 components 目录";
        assert!(relevance(&q, git_entry) > relevance(&q, unrelated));
    }

    #[test]
    fn multiple_appends_accumulate() {
        let ws = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", ws.path());
        append(Some(ws.path()), "project", "偏好", "用中文回答").unwrap();
        append(Some(ws.path()), "project", "约定", "测试必须先跑通").unwrap();
        let section = prompt_section(Some(ws.path()), None).unwrap();
        assert!(section.contains("用中文回答"));
        assert!(section.contains("测试必须先跑通"));
        let content = std::fs::read_to_string(ws.path().join(".codeforge/MEMORY.md")).unwrap();
        assert!(content.contains("# codeForge 记忆"));
    }
}
