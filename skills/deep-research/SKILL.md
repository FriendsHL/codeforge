---
name: deep-research
description: 当用户要求"调研/深度调查/技术选型/竞品对比/可行性分析"等需要严谨多源信息并整理成报告的任务时使用。它规定一套"拆角度→并行检索→自验证→带引用结论→产出独立 HTML 报告"的完整流程。普通单点查询用 web_search 即可，不必用本技能。
---

# 深度调研 + HTML 报告

目标：把一个开放问题做成**可信、有据可查、可分享**的调研报告，最终产物是一个**自包含的单文件 HTML**（内联样式，浏览器直接打开）。

## 流程（务必逐步执行，不要跳步）

### 1. 拆解角度
先调 `research_plan` 拿到方法论。把核心问题拆成 2~4 个**互相独立**的子角度，例如：
- 官方文档 / 现状与定义
- 对立观点 / 已知批评 / 失败案例
- 最新进展（注明时间，时效性话题尤其重要）
- 实际落地案例 / 数据

### 2. 并行检索
用 `spawn_subagents` 给每个角度派一个子 agent，各自 `web_search` + `web_fetch` 深入。**强制要求**每个子 agent 的汇报里，每条关键事实都附带来源 URL 与（若有）日期。角度之间彼此独立，避免串行干等。

### 3. 综合去重
汇总各角度发现，合并重复事实，**显式标注分歧点**（谁说 A、谁说 B、各自来源）。

### 4. 自验证（Generator-Verifier）
对你打算写进结论的**每一条关键判断**，主动反问：
- 有没有相反证据？
- 来源可靠吗（官方/一手 > 二手/博客）？
- 是否过时？
必要时再 `web_search` 一轮**反驳性查询**。证据不足的判断必须标注「存疑」，不要硬下结论。

### 5. 产出 HTML 报告
用 `write_file` 写一个独立 HTML 文件到工作区（建议放 `reports/` 下，文件名 `report-<主题简写>.html`）。**严格使用下面的模板骨架**，把内容填进去：

- 报告标题 + 一句话结论摘要（TL;DR）
- 「核心结论」区：每条结论后用上标 `<sup>[n]</sup>` 挂引用编号；确证与存疑分开标注（存疑的加 `.uncertain` 类）
- 「分项分析」区：按第 1 步的角度分小节
- 「分歧与风险」区：把第 3 步的分歧点列出来
- 「参考来源」区：编号列表，每条 `[n] 标题 — URL（日期）`

写完后：
1. 用 `read_file` 抽查生成的 HTML 是否完整、引用编号对得上；
2. 用 `browser_open` 打开 `file://<HTML 绝对路径>` 让用户直接看到成品；
3. 在聊天里给一句话总结 + 报告文件路径。

## HTML 模板骨架

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{标题}}</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 15px/1.7 -apple-system, "PingFang SC", system-ui, sans-serif;
         max-width: 820px; margin: 0 auto; padding: 40px 24px; color: #1f2328; }
  h1 { font-size: 26px; border-bottom: 2px solid #eaecef; padding-bottom: 12px; }
  h2 { font-size: 19px; margin-top: 34px; border-left: 4px solid #2f81f7; padding-left: 10px; }
  .tldr { background: #f6f8fa; border: 1px solid #d0d7de; border-radius: 8px;
          padding: 14px 18px; margin: 18px 0; }
  .uncertain { color: #9a6700; }
  .uncertain::before { content: "⚠ 存疑："; font-weight: 600; }
  sup { color: #2f81f7; font-weight: 600; }
  .sources { font-size: 13px; color: #57606a; }
  .sources li { margin: 6px 0; word-break: break-all; }
  .meta { color: #8b949e; font-size: 13px; }
  @media (prefers-color-scheme: dark) {
    body { color: #e6edf3; background: #0d1117; }
    .tldr { background: #161b22; border-color: #30363d; }
    h1 { border-color: #30363d; }
  }
</style>
</head>
<body>
  <h1>{{标题}}</h1>
  <p class="meta">调研日期 {{日期}} · 由 codeForge 生成</p>
  <div class="tldr"><strong>TL;DR：</strong>{{一句话结论}}</div>

  <h2>核心结论</h2>
  <ul>
    <li>{{确证结论}}<sup>[1]</sup></li>
    <li class="uncertain">{{存疑结论}}<sup>[2]</sup></li>
  </ul>

  <h2>分项分析</h2>
  <h3>{{角度一}}</h3>
  <p>{{内容}}<sup>[1]</sup></p>

  <h2>分歧与风险</h2>
  <ul><li>{{分歧点：A 说…（来源），B 说…（来源）}}</li></ul>

  <h2>参考来源</h2>
  <ol class="sources">
    <li>{{标题}} — <a href="{{URL}}">{{URL}}</a>（{{日期}}）</li>
  </ol>
</body>
</html>
```

## 纪律
- 绝不编造来源或 URL；没找到就如实说「未找到可靠来源」。
- 引用编号要和「参考来源」一一对应，写完核对一遍。
- 时效性强的话题，优先近期来源并注明信息时间。
- 报告语言跟随用户（默认中文）。
