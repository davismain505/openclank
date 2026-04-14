//! The `ToolExecutor` trait and its real implementation.
//!
//! Each supported tool maps to a concrete operation:
//!
//! | Tool        | What it does                                            |
//! |-------------|---------------------------------------------------------|
//! | `Bash`      | Run a shell command, return stdout and exit code        |
//! | `ReadFile`  | Read a file's contents                                  |
//! | `WriteFile` | Write/overwrite a file with new contents                |
//! | `EditFile`  | Replace a unique occurrence of `old` with `new`         |
//! | `Glob`      | List files matching a glob pattern                      |
//! | `Grep`      | Find lines matching a regex (ripgrep, falls back to grep)|
//!
//! Errors produced by the tools (command failures, missing files,
//! permission denied) are returned as `ToolExecResult { is_error: true }`
//! rather than `Result::Err`. Tool errors are normal outcomes that the
//! model should see and reason about, not runner failures.
//!
//! ## Bash stderr
//!
//! The bash tool captures stdout only, not stderr. LLMs that want to
//! see error output reliably use `2>&1` in their command strings
//! themselves. Merging stderr server-side would double up whenever the
//! model does that, and confuse commands that write progress to stderr
//! by intention.

use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::event::effects::ToolCall;
use crate::state::message::ToolName;

/// The output of a tool execution.
///
/// Corresponds to the `content` and `is_error` fields of a
/// [`ContentBlock::ToolResult`](crate::state::message::ContentBlock::ToolResult).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolExecResult {
    /// The text content to send back to the model. For commands this
    /// is stdout; for file reads it's the file contents; for errors
    /// it's a human-readable description.
    pub content: String,
    /// Whether this represents a failure (non-zero exit, missing file,
    /// permission error, malformed input, etc.). The model uses this
    /// to distinguish successful output from error output.
    pub is_error: bool,
}

impl ToolExecResult {
    /// Create a successful result with the given content.
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    /// Create an error result with the given message.
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            content: message.into(),
            is_error: true,
        }
    }
}

/// Abstraction over how tools are executed.
///
/// Implementors perform the real work (running subprocesses, reading
/// files) and return a [`ToolExecResult`]. The trait is async because
/// tool execution involves IO.
///
/// `Send + Sync` bounds let the runner share the executor across the
/// tokio tasks that drive individual tool executions.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute the given tool call and return its result.
    async fn execute(&self, call: &ToolCall) -> ToolExecResult;
}

/// The production tool executor. Runs real subprocesses and touches
/// the real filesystem.
#[derive(Debug, Default)]
pub struct RealToolExecutor;

impl RealToolExecutor {
    /// Construct a new executor. Currently stateless, but reserved
    /// for future configuration (allowed commands, working directory,
    /// environment overrides).
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolExecutor for RealToolExecutor {
    async fn execute(&self, call: &ToolCall) -> ToolExecResult {
        match call.name {
            ToolName::Bash => run_bash(&call.input).await,
            ToolName::ReadFile => run_read_file(&call.input).await,
            ToolName::WriteFile => run_write_file(&call.input).await,
            ToolName::EditFile => run_edit_file(&call.input).await,
            ToolName::Glob => run_glob(&call.input).await,
            ToolName::Grep => run_grep(&call.input).await,
        }
    }
}

// ─── Individual tool implementations ──────────────────────────────────

/// Run a shell command via `sh -c`. Captures stdout only — LLMs that
/// want stderr use `2>&1` in their command string. Flags failure if
/// the exit code is non-zero.
async fn run_bash(input: &Value) -> ToolExecResult {
    let Some(command) = input.get("command").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("bash: missing 'command' argument");
    };

    let output = match Command::new("sh").arg("-c").arg(command).output().await {
        Ok(o) => o,
        Err(e) => return ToolExecResult::err(format!("bash: failed to spawn: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if output.status.success() {
        ToolExecResult::ok(stdout)
    } else {
        let code = output.status.code().unwrap_or(-1);
        ToolExecResult::err(format!("exit code {code}\n{stdout}"))
    }
}

/// Read a file's contents.
async fn run_read_file(input: &Value) -> ToolExecResult {
    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("read_file: missing 'path' argument");
    };
    let path = Path::new(path_str);

    match tokio::fs::File::open(path).await {
        Ok(mut file) => {
            let mut contents = String::new();
            if let Err(e) = file.read_to_string(&mut contents).await {
                return ToolExecResult::err(format!("read_file: {e}"));
            }
            ToolExecResult::ok(contents)
        }
        Err(e) => ToolExecResult::err(format!("read_file: {e}")),
    }
}

/// Write a file, creating it if needed and overwriting existing content.
async fn run_write_file(input: &Value) -> ToolExecResult {
    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("write_file: missing 'path' argument");
    };
    let Some(content) = input.get("content").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("write_file: missing 'content' argument");
    };

    match tokio::fs::write(path_str, content).await {
        Ok(()) => ToolExecResult::ok(format!("wrote {} bytes to {path_str}", content.len())),
        Err(e) => ToolExecResult::err(format!("write_file: {e}")),
    }
}

/// Replace a unique occurrence of `old` with `new` in a file.
/// Returns an error if `old` is not found or if multiple matches exist
/// (forcing the model to provide more context).
async fn run_edit_file(input: &Value) -> ToolExecResult {
    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("edit_file: missing 'path' argument");
    };
    let Some(old) = input.get("old").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("edit_file: missing 'old' argument");
    };
    let Some(new) = input.get("new").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("edit_file: missing 'new' argument");
    };

    let contents = match tokio::fs::read_to_string(path_str).await {
        Ok(s) => s,
        Err(e) => return ToolExecResult::err(format!("edit_file: {e}")),
    };

    let occurrences = contents.matches(old).count();
    if occurrences == 0 {
        return ToolExecResult::err(format!(
            "edit_file: 'old' text not found in {path_str}"
        ));
    }
    if occurrences > 1 {
        return ToolExecResult::err(format!(
            "edit_file: 'old' text matches {occurrences} times in {path_str}; provide more context to make it unique"
        ));
    }

    let updated = contents.replacen(old, new, 1);
    match tokio::fs::write(path_str, &updated).await {
        Ok(()) => ToolExecResult::ok(format!("edited {path_str}")),
        Err(e) => ToolExecResult::err(format!("edit_file: {e}")),
    }
}

/// List files matching a glob pattern. Shells out to `ls -d` so the
/// shell expands the pattern.
async fn run_glob(input: &Value) -> ToolExecResult {
    let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("glob: missing 'pattern' argument");
    };

    let cmd = format!("ls -d -- {pattern}");
    let output = match Command::new("sh").arg("-c").arg(&cmd).output().await {
        Ok(o) => o,
        Err(e) => return ToolExecResult::err(format!("glob: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        ToolExecResult::ok(format!("(no matches for {pattern})"))
    } else {
        ToolExecResult::ok(stdout)
    }
}

/// Search for a regex pattern in files. Tries `rg` (ripgrep) first
/// for speed and better defaults, falls back to `grep` if ripgrep
/// isn't installed.
async fn run_grep(input: &Value) -> ToolExecResult {
    let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
        return ToolExecResult::err("grep: missing 'pattern' argument");
    };
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    // Try ripgrep first. It's faster, respects .gitignore by default,
    // and has nicer output. If it's not on PATH, spawn fails with
    // NotFound and we fall back to grep.
    match Command::new("rg")
        .arg("--line-number")
        .arg("--")
        .arg(pattern)
        .arg(path)
        .output()
        .await
    {
        Ok(output) => map_grep_exit(output, pattern, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Fall back to system grep.
            match Command::new("grep")
                .arg("-rn")
                .arg("--")
                .arg(pattern)
                .arg(path)
                .output()
                .await
            {
                Ok(output) => map_grep_exit(output, pattern, path),
                Err(e) => ToolExecResult::err(format!("grep: {e}")),
            }
        }
        Err(e) => ToolExecResult::err(format!("grep: {e}")),
    }
}

/// Map grep-style exit codes to a `ToolExecResult`.
///
/// Both ripgrep and grep use the same convention: exit 0 = matches
/// found, exit 1 = no matches (not an error), anything else = error.
fn map_grep_exit(
    output: std::process::Output,
    pattern: &str,
    path: &str,
) -> ToolExecResult {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    match output.status.code() {
        Some(0) => ToolExecResult::ok(stdout),
        Some(1) => ToolExecResult::ok(format!("(no matches for {pattern} in {path})")),
        Some(code) => ToolExecResult::err(format!("grep: exit code {code}")),
        None => ToolExecResult::err("grep: terminated by signal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::message::ToolUseId;

    fn call(name: ToolName, input: Value) -> ToolCall {
        ToolCall {
            id: ToolUseId::new("toolu_test01234567890").unwrap(),
            name,
            input,
        }
    }

    #[tokio::test]
    async fn bash_echo() {
        let exec = RealToolExecutor::new();
        let result = exec
            .execute(&call(ToolName::Bash, serde_json::json!({"command": "echo hi"})))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("hi"));
    }

    #[tokio::test]
    async fn bash_nonzero_exit_is_error() {
        let exec = RealToolExecutor::new();
        let result = exec
            .execute(&call(ToolName::Bash, serde_json::json!({"command": "false"})))
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn bash_captures_only_stdout() {
        // Write to both stdout and stderr. Our result should contain
        // only the stdout piece (LLMs use 2>&1 if they want both).
        let exec = RealToolExecutor::new();
        let result = exec
            .execute(&call(
                ToolName::Bash,
                serde_json::json!({"command": "echo to_stdout; echo to_stderr 1>&2"}),
            ))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("to_stdout"));
        assert!(
            !result.content.contains("to_stderr"),
            "stderr leaked into output: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn bash_missing_command() {
        let exec = RealToolExecutor::new();
        let result = exec.execute(&call(ToolName::Bash, serde_json::json!({}))).await;
        assert!(result.is_error);
        assert!(result.content.contains("missing"));
    }

    #[tokio::test]
    async fn read_write_file_roundtrip() {
        let exec = RealToolExecutor::new();
        let tmp = std::env::temp_dir().join(format!("openclank_test_{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();

        let write = exec
            .execute(&call(
                ToolName::WriteFile,
                serde_json::json!({"path": tmp_str, "content": "hello\nworld"}),
            ))
            .await;
        assert!(!write.is_error, "write failed: {}", write.content);

        let read = exec
            .execute(&call(
                ToolName::ReadFile,
                serde_json::json!({"path": tmp_str}),
            ))
            .await;
        assert!(!read.is_error);
        assert_eq!(read.content, "hello\nworld");

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn read_nonexistent_file_is_error() {
        let exec = RealToolExecutor::new();
        let result = exec
            .execute(&call(
                ToolName::ReadFile,
                serde_json::json!({"path": "/definitely/does/not/exist"}),
            ))
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_match() {
        let exec = RealToolExecutor::new();
        let tmp = std::env::temp_dir().join(format!("openclank_edit_{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();

        tokio::fs::write(&tmp, "the quick brown fox").await.unwrap();

        let result = exec
            .execute(&call(
                ToolName::EditFile,
                serde_json::json!({"path": tmp_str, "old": "quick", "new": "slow"}),
            ))
            .await;
        assert!(!result.is_error, "edit failed: {}", result.content);

        let contents = tokio::fs::read_to_string(&tmp).await.unwrap();
        assert_eq!(contents, "the slow brown fox");

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn edit_file_refuses_ambiguous_match() {
        let exec = RealToolExecutor::new();
        let tmp = std::env::temp_dir().join(format!("openclank_amb_{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();

        tokio::fs::write(&tmp, "foo foo foo").await.unwrap();

        let result = exec
            .execute(&call(
                ToolName::EditFile,
                serde_json::json!({"path": tmp_str, "old": "foo", "new": "bar"}),
            ))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("3 times"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }

    #[tokio::test]
    async fn edit_file_no_match() {
        let exec = RealToolExecutor::new();
        let tmp = std::env::temp_dir().join(format!("openclank_nomatch_{}", std::process::id()));
        let tmp_str = tmp.to_string_lossy().to_string();

        tokio::fs::write(&tmp, "hello").await.unwrap();

        let result = exec
            .execute(&call(
                ToolName::EditFile,
                serde_json::json!({"path": tmp_str, "old": "missing", "new": "found"}),
            ))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));

        let _ = tokio::fs::remove_file(&tmp).await;
    }
}
