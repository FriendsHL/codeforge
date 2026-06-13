//! diagnostics 工具：跑项目的快速类型/语法检查，把报错结构化返回。
//! LSP 的轻量替代——拿到"我改的代码有没有错"这个最高频价值，不维护 language server。
//! 按工作区里出现的工程标志自动选检查器(可被 input.checker 覆盖)。

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::registry::Tool;
use crate::llm::types::ToolSpec;
use crate::pty;

const TIMEOUT: Duration = Duration::from_secs(300); // Maven/Gradle 首次编译较慢，给足
const MAX_OUTPUT_CHARS: usize = 20_000;

struct Checker {
    name: &'static str,
    /// 工作区根目录下存在该文件则适用
    marker: &'static str,
    command: &'static str,
}

const CHECKERS: &[Checker] = &[
    Checker { name: "rust", marker: "Cargo.toml", command: "cargo check --message-format short 2>&1" },
    Checker { name: "typescript", marker: "tsconfig.json", command: "npx --no-install tsc --noEmit 2>&1 || true" },
    Checker { name: "python-ruff", marker: "pyproject.toml", command: "ruff check . 2>&1 || true" },
    Checker { name: "go", marker: "go.mod", command: "go vet ./... 2>&1 || true" },
    // Java：编译检查（test-compile 连测试代码一起编，覆盖更全）。-q 安静、-o 离线优先省时
    Checker { name: "java-maven", marker: "pom.xml", command: "mvn -q -o test-compile 2>&1 || mvn -q test-compile 2>&1 || true" },
    Checker { name: "java-gradle", marker: "build.gradle", command: "./gradlew -q compileTestJava 2>&1 || gradle -q compileTestJava 2>&1 || true" },
];

fn pick_checker<'a>(workspace: &Path, explicit: Option<&str>) -> Option<&'a Checker> {
    if let Some(name) = explicit {
        return CHECKERS.iter().find(|c| c.name == name);
    }
    CHECKERS.iter().find(|c| workspace.join(c.marker).exists())
}

pub struct DiagnosticsTool;

impl Tool for DiagnosticsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "diagnostics".into(),
            description: "对当前项目跑一次快速类型/语法检查（rust=cargo check、ts=tsc --noEmit、python=ruff、go=go vet、java=mvn/gradle 编译，按项目自动选），返回结构化报错。改完代码后用它确认没引入编译错误，比手动跑构建快。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "checker": {"type": "string", "enum": ["rust", "typescript", "python-ruff", "go", "java-maven", "java-gradle"], "description": "强制指定检查器（默认按项目自动选）"}
                },
                "required": []
            }),
        }
    }

    // 只读检查（不改代码），ask 模式下也不必审批；但它会跑命令，归类为非 mutating 的内部检查
    fn run(&self, workspace: &Path, input: &Value) -> Result<String, String> {
        let explicit = input["checker"].as_str();
        let checker = pick_checker(workspace, explicit).ok_or(
            "未识别项目类型（没找到 Cargo.toml/tsconfig.json/pyproject.toml/go.mod）；可用 checker 参数强制指定",
        )?;

        let result = pty::run_command(workspace, checker.command, TIMEOUT, |_| {})?;
        let clean = pty::strip_ansi(&result.output);
        let trimmed = clean.trim();

        let body = if trimmed.is_empty() {
            "（无输出）".to_string()
        } else if trimmed.chars().count() > MAX_OUTPUT_CHARS {
            let head: String = trimmed.chars().take(MAX_OUTPUT_CHARS).collect();
            format!("{head}\n…[诊断输出过长已截断]")
        } else {
            trimmed.to_string()
        };

        let verdict = if result.timed_out {
            format!("[{} 检查超时（{}s）]", checker.name, TIMEOUT.as_secs())
        } else if result.exit_code == 0 && trimmed.is_empty() {
            format!("[{} 检查通过，无诊断]", checker.name)
        } else {
            format!("[{} 检查 exit={}]", checker.name, result.exit_code)
        };
        Ok(format!("{verdict}\n{body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_checker_by_marker() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let c = pick_checker(dir.path(), None).unwrap();
        assert_eq!(c.name, "rust");
    }

    #[test]
    fn explicit_checker_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let c = pick_checker(dir.path(), Some("go")).unwrap();
        assert_eq!(c.name, "go");
    }

    #[test]
    fn picks_java_maven_by_pom() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pom.xml"), "<project/>").unwrap();
        assert_eq!(pick_checker(dir.path(), None).unwrap().name, "java-maven");
    }

    #[test]
    fn no_marker_no_checker() {
        let dir = tempfile::tempdir().unwrap();
        assert!(pick_checker(dir.path(), None).is_none());
    }
}
