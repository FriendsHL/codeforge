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
- 基于读到的真实代码回答，不要编造没有看过的内容；证据不足时直说。
- 修改代码：局部改动用 edit_file（old_string 需逐字符精确匹配），新建文件或整文件重写用 write_file。改前先 read_file 确认现状；每次改动会以 diff 形式请用户审批，被拒绝时先弄清用户意图再调整方案。
- 执行命令用 bash（跑测试、构建、安装依赖等），同样需用户审批。改完代码主动跑相关测试验证，失败就根据输出继续修，直到通过。避免无关的全局安装和网络下载。
- 遇到不熟悉的库 API、报错信息或需要最新文档时，用 web_search 搜索、web_fetch 阅读具体页面；优先官方文档。"
                .into(),
        );

        // M8 技能段：当年在 M2 留的「分段组装」接缝，今天用上了
        if let Some(skills_section) = crate::skills::prompt_section(Some(workspace)) {
            sections.push(skills_section);
        }
    } else {
        sections.push(
            "\
当前未打开项目目录（纯聊天模式）：
- 你仍可联网：web_search 搜索、web_fetch 阅读网页，遇到时效性问题主动用它们。
- 用户要操作某个代码库时，请提示其点顶栏「打开项目」。"
                .into(),
        );
        if let Some(skills_section) = crate::skills::prompt_section(None) {
            sections.push(skills_section);
        }
    }

    sections.join("\n\n")
}
