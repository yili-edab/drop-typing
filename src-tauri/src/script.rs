//! 语音指令脚本执行（M4 指令通道）。
//!
//! 动作别名的 `script` 字段支持两种写法：
//! - 已存在的脚本文件路径（绝对路径或 `~/` 开头）→ 按 shebang 直接执行，
//!   工作目录为脚本所在目录；文件需有执行权限（否则明确提示 chmod +x）；
//! - 一行 shell 命令 → 交给 `/bin/zsh -lc` 执行，工作目录为用户主目录。
//!
//! 执行是阻塞式的（调用方应放在后台线程）；不设超时、不展示 stdout，
//! 失败时返回退出码与截断的 stderr 摘要。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// 脚本执行失败信息（退出码 + stderr 摘要，或启动失败原因）。
#[derive(Debug)]
pub struct ScriptError {
    message: String,
}

impl ScriptError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScriptError {}

/// 阻塞式执行脚本：自动判断值是「文件路径」还是「一行命令」。
pub fn run(script: &str) -> Result<(), ScriptError> {
    let script = script.trim();
    if script.is_empty() {
        return Err(ScriptError::new("脚本内容为空"));
    }
    // 展开 ~ 后若为已存在文件 → 按 shebang 直接执行；否则按一行命令交给 zsh
    if let Some(path) = resolve_existing_file(script) {
        run_file(&path)
    } else {
        run_shell_line(script)
    }
}

/// `~/...` / `~` → 用户主目录绝对路径；其它原样返回。
fn expand_tilde(raw: &str) -> String {
    let Some(home) = std::env::var("HOME").ok() else {
        return raw.to_string();
    };
    if raw == "~" {
        home
    } else if let Some(rest) = raw.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        raw.to_string()
    }
}

/// 展开后是已存在的普通文件则返回其路径，否则返回 None（落到命令行分支）。
fn resolve_existing_file(raw: &str) -> Option<PathBuf> {
    let expanded = expand_tilde(raw);
    let path = Path::new(&expanded);
    path.is_file().then(|| path.to_path_buf())
}

fn run_file(path: &Path) -> Result<(), ScriptError> {
    let mut cmd = Command::new(path);
    if let Some(parent) = path.parent() {
        cmd.current_dir(parent);
    }
    let output = cmd.output().map_err(|e| {
        ScriptError::new(format!(
            "无法执行脚本 {}：{e}（若是缺少执行权限，请先运行 chmod +x 后重试）",
            path.display()
        ))
    })?;
    finish(output)
}

fn run_shell_line(line: &str) -> Result<(), ScriptError> {
    let cwd = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let output = Command::new("/bin/zsh")
        .args(["-lc", line])
        .current_dir(cwd)
        .output()
        .map_err(|e| ScriptError::new(format!("无法启动 /bin/zsh：{e}")))?;
    finish(output)
}

/// 统一收尾：退出码 0 → Ok；否则错误携带退出码 + 截断的 stderr 摘要。
fn finish(output: Output) -> Result<(), ScriptError> {
    if output.status.success() {
        return Ok(());
    }
    let code = match output.status.code() {
        Some(c) => c.to_string(),
        None => "信号终止".to_string(),
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        Err(ScriptError::new(format!("脚本执行失败（退出码 {code}）")))
    } else {
        let summary = truncate(stderr, 2000);
        Err(ScriptError::new(format!(
            "脚本执行失败（退出码 {code}）：{summary}"
        )))
    }
}

/// 截断到 max_chars 字符，超出则追加省略号说明。
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars).collect();
    format!("{head}…（已截断）")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时目录写一个可执行脚本，返回其绝对路径。
    fn write_exec_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn shell_line_success() {
        assert!(run("echo drop-typing-ok").is_ok());
    }

    #[test]
    fn shell_line_nonzero_exit_returns_error_with_stderr() {
        let err = run("echo 'boom' >&2; exit 7").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("退出码 7"), "应包含退出码：{msg}");
        assert!(msg.contains("boom"), "应包含 stderr 摘要：{msg}");
    }

    #[test]
    fn shell_line_unknown_command_reports_not_found() {
        let err = run("drop-typing-no-such-command-xyz").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("command not found") || msg.contains("未找到命令"),
            "应提示命令不存在：{msg}"
        );
    }

    #[test]
    fn file_path_executes_via_shebang() {
        let dir = std::env::temp_dir().join(format!(
            "drop-typing-script-test-{}-ok",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_exec_script(&dir, "ok.sh", "#!/bin/sh\nexit 0\n");
        assert!(run(path.to_str().unwrap()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_path_nonzero_exit_reports_code_and_stderr() {
        let dir = std::env::temp_dir().join(format!(
            "drop-typing-script-test-{}-fail",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_exec_script(&dir, "fail.sh", "#!/bin/sh\necho 'from-script' >&2\nexit 9\n");
        let err = run(path.to_str().unwrap()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("退出码 9"), "应包含退出码：{msg}");
        assert!(msg.contains("from-script"), "应包含 stderr：{msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tilde_expansion_uses_home() {
        let home = std::env::var("HOME").expect("测试环境应有 HOME");
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/backup.sh"), format!("{home}/backup.sh"));
        assert_eq!(expand_tilde("/abs/path.sh"), "/abs/path.sh");
    }

    #[test]
    fn non_file_value_falls_through_to_shell() {
        // 不存在的 ~/ 路径不是文件 → 落到 zsh 命令行分支并报“无此文件”
        let err = run("~/drop-typing-no-such-dir-xyz/foo.sh").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no such file") || msg.contains("No such file") || msg.contains("没有那个文件"),
            "应提示无此文件：{msg}"
        );
    }

    #[test]
    fn empty_script_rejected() {
        let err = run("   ").unwrap_err();
        assert!(err.to_string().contains("脚本内容为空"));
    }
}
