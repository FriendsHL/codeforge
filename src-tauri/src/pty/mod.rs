//! PTY 进程管理：spawn 命令、流式输出、超时 kill

use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// 防失控的输出硬上限（流式给 UI 的总量）
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

pub struct CommandResult {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}

/// 在工作区根目录下经 PTY 执行 shell 命令，输出实时回调 on_chunk
pub fn run_command(
    workspace: &Path,
    command: &str,
    timeout: Duration,
    mut on_chunk: impl FnMut(&str),
) -> Result<CommandResult, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("创建 PTY 失败: {e}"))?;

    let mut cmd = CommandBuilder::new("/bin/zsh");
    cmd.arg("-c");
    cmd.arg(command);
    cmd.cwd(workspace);

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("启动命令失败: {e}"))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("读取输出失败: {e}"))?;

    // 子进程退出后 master 读到 EOF，reader 线程结束 → 通道断开
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut timed_out = false;

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(bytes) => {
                let chunk = String::from_utf8_lossy(&bytes).to_string();
                if output.len() < MAX_OUTPUT_BYTES {
                    on_chunk(&chunk);
                    output.push_str(&chunk);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if Instant::now() > deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
    }
    // kill 后清空残余输出
    while let Ok(bytes) = rx.try_recv() {
        let chunk = String::from_utf8_lossy(&bytes).to_string();
        if output.len() < MAX_OUTPUT_BYTES {
            output.push_str(&chunk);
        }
    }

    let exit_code = child
        .wait()
        .map(|status| status.exit_code() as i32)
        .unwrap_or(-1);
    drop(pair.master);
    let _ = reader_thread.join();

    Ok(CommandResult { exit_code, output, timed_out })
}

/// 去掉 ANSI 转义序列（回填给模型的文本不需要颜色码）
pub fn strip_ansi(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\r").unwrap();
    re.replace_all(text, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_command_and_captures_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut streamed = String::new();
        let result = run_command(
            dir.path(),
            "echo hello-pty && exit 3",
            Duration::from_secs(10),
            |chunk| streamed.push_str(chunk),
        )
        .unwrap();
        assert!(result.output.contains("hello-pty"));
        assert!(streamed.contains("hello-pty"));
        assert_eq!(result.exit_code, 3);
        assert!(!result.timed_out);
    }

    #[test]
    fn kills_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let start = Instant::now();
        let result = run_command(dir.path(), "sleep 30", Duration::from_secs(1), |_| {}).unwrap();
        assert!(result.timed_out);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn strips_ansi_codes() {
        let colored = "\x1b[1m\x1b[32mok\x1b[0m\r\ndone";
        assert_eq!(strip_ansi(colored), "ok\ndone");
    }
}
