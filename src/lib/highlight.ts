import Prism from "prismjs";
import "prismjs/themes/prism.css";
// core 自带 markup/css/clike/javascript；其余按需注册（注意依赖顺序）
import "prismjs/components/prism-typescript";
import "prismjs/components/prism-jsx";
import "prismjs/components/prism-tsx";
import "prismjs/components/prism-java";
import "prismjs/components/prism-rust";
import "prismjs/components/prism-json";
import "prismjs/components/prism-yaml";
import "prismjs/components/prism-toml";
import "prismjs/components/prism-bash";
import "prismjs/components/prism-python";
import "prismjs/components/prism-go";
import "prismjs/components/prism-sql";
import "prismjs/components/prism-markdown";
import "prismjs/components/prism-properties";

const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  java: "java",
  rs: "rust",
  json: "json",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  sh: "bash",
  zsh: "bash",
  bash: "bash",
  py: "python",
  go: "go",
  sql: "sql",
  md: "markdown",
  markdown: "markdown",
  properties: "properties",
  html: "markup",
  xml: "markup",
  svg: "markup",
  css: "css",
};

export function languageForPath(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return EXT_TO_LANG[ext] ?? null;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** 高亮一段代码为 HTML；语言未知时仅做 HTML 转义 */
export function highlightCode(code: string, lang: string | null): string {
  if (lang && Prism.languages[lang]) {
    try {
      return Prism.highlight(code, Prism.languages[lang], lang);
    } catch {
      return escapeHtml(code);
    }
  }
  return escapeHtml(code);
}
