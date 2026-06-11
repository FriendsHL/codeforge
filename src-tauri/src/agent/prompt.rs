//! system prompt 组装。刻意做成分段拼接——v2 的 skill 内容会作为新的段注入。

use std::path::Path;

pub fn build_system_prompt(workspace: Option<&Path>) -> String {
    let mut sections: Vec<String> = Vec::new();

    sections.push(
        "你是 codeForge，一个运行在用户本机的 coding agent。用中文回答。".into(),
    );

    if let Some(workspace) = workspace {
        sections.push(format!("当前工作区根目录：{}", workspace.display()));
        sections.push(
            "\
工作方式：
- 回答代码库相关问题前，先用工具收集证据：list_dir 看结构、glob 找文件、grep 搜内容、read_file 读片段。
- 项目是 git 仓库时，可用 git_status / git_diff 了解用户当前正在改什么，git_log 了解最近的提交脉络；涉及「最近改动」「为什么变成这样」的问题优先看 git。
- 所有路径都使用相对工作区根目录的相对路径。
- 工具结果可能带 [truncated] 截断标记，需要更多内容就带 offset 继续读或缩小搜索范围。
- 引用代码时给出 `文件路径:行号`。
- 基于读到的真实代码回答，不要编造没有看过的内容；证据不足时直说。"
                .into(),
        );
    }

    sections.join("\n\n")
}
