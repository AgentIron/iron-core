use crate::builtin::config::BuiltinToolConfig;
use crate::error::RuntimeResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::Value;

pub struct BashTool {
    config: BuiltinToolConfig,
}

impl BashTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self { config }
    }
}

impl Tool for BashTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "bash",
            "Execute a bash command. The command runs in the specified working directory. Returns stdout, stderr, and exit code.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Working directory for the command (must be within allowed roots)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (optional, uses default if not specified)"
                    }
                },
                "required": ["command"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move { execute_bash(&config, arguments).await })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

async fn execute_bash(config: &BuiltinToolConfig, args: Value) -> RuntimeResult<Value> {
    use crate::builtin::error::BuiltinToolError;

    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'command' argument"))
        .map_err(crate::error::RuntimeError::from)?;

    let working_dir = args
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from);

    if let Some(ref dir) = working_dir {
        config
            .policy
            .validate_path(dir, &config.allowed_roots)
            .map_err(crate::error::RuntimeError::from)?;
    }

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .map(std::time::Duration::from_secs)
        .unwrap_or(config.shell_timeout);

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    } else if let Some(root) = config.allowed_roots.first() {
        cmd.current_dir(root);
    }

    let mut child = cmd.spawn().map_err(|e| {
        crate::error::RuntimeError::tool_execution(format!("failed to spawn bash: {}", e))
    })?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let result = tokio::select! {
        output = child.wait() => {
            match output {
                Ok(status) => {
                    let stdout = read_to_string(stdout_handle).await;
                    let stderr = read_to_string(stderr_handle).await;
                    let exit_code = status.code().unwrap_or(-1);

                    let (stdout_truncated, stdout_final) =
                        truncate_output(&stdout, config.max_output_bytes);
                    let (stderr_truncated, stderr_final) =
                        truncate_output(&stderr, config.max_output_bytes);

                    let meta = if stdout_truncated || stderr_truncated {
                        crate::builtin::helpers::BuiltinMeta::with_truncation(stdout.len() + stderr.len())
                    } else {
                        crate::builtin::helpers::BuiltinMeta::empty()
                    };

                    Ok(serde_json::json!({
                        "stdout": stdout_final,
                        "stderr": stderr_final,
                        "exit_code": exit_code,
                        "meta": meta,
                    }))
                }
                Err(e) => Err(crate::error::RuntimeError::tool_execution(format!(
                    "bash execution failed: {}",
                    e
                ))),
            }
        }
        _ = tokio::time::sleep(timeout_secs) => {
            let _ = child.kill().await;
            Ok(serde_json::json!({
                "stdout": "",
                "stderr": "command timed out",
                "exit_code": -1,
                "meta": crate::builtin::helpers::BuiltinMeta::empty(),
            }))
        }
    };

    result
}

async fn read_to_string(maybe_reader: Option<impl tokio::io::AsyncRead + Unpin>) -> String {
    match maybe_reader {
        Some(mut reader) => {
            let mut buf = Vec::new();
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        }
        None => String::new(),
    }
}

fn truncate_output(output: &str, max_bytes: usize) -> (bool, String) {
    if output.len() <= max_bytes {
        return (false, output.to_string());
    }
    (
        true,
        output[..output.floor_char_boundary(max_bytes)].to_string(),
    )
}

pub struct PowerShellTool {
    config: BuiltinToolConfig,
}

impl PowerShellTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self { config }
    }
}

impl Tool for PowerShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "powershell",
            "Execute a PowerShell command. The command runs in the specified working directory. Returns stdout, stderr, and exit code.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The PowerShell command to execute"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "Working directory for the command (must be within allowed roots)"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (optional, uses default if not specified)"
                    }
                },
                "required": ["command"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move { execute_powershell(&config, arguments).await })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

async fn execute_powershell(config: &BuiltinToolConfig, args: Value) -> RuntimeResult<Value> {
    use crate::builtin::error::BuiltinToolError;

    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'command' argument"))
        .map_err(crate::error::RuntimeError::from)?;

    let working_dir = args
        .get("working_dir")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from);

    if let Some(ref dir) = working_dir {
        config
            .policy
            .validate_path(dir, &config.allowed_roots)
            .map_err(crate::error::RuntimeError::from)?;
    }

    let timeout_secs = args
        .get("timeout")
        .and_then(|v| v.as_u64())
        .map(std::time::Duration::from_secs)
        .unwrap_or(config.shell_timeout);

    let shell_cmd = if which_exists("pwsh") {
        "pwsh"
    } else {
        "powershell"
    };

    let mut cmd = tokio::process::Command::new(shell_cmd);
    cmd.arg("-NoProfile")
        .arg("-Command")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    } else if let Some(root) = config.allowed_roots.first() {
        cmd.current_dir(root);
    }

    let output = tokio::time::timeout(timeout_secs, cmd.output()).await;

    match output {
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let exit_code = out.status.code().unwrap_or(-1);

            let (stdout_truncated, stdout_final) =
                truncate_output(&stdout, config.max_output_bytes);
            let (stderr_truncated, stderr_final) =
                truncate_output(&stderr, config.max_output_bytes);

            let meta = if stdout_truncated || stderr_truncated {
                crate::builtin::helpers::BuiltinMeta::with_truncation(stdout.len() + stderr.len())
            } else {
                crate::builtin::helpers::BuiltinMeta::empty()
            };

            Ok(serde_json::json!({
                "stdout": stdout_final,
                "stderr": stderr_final,
                "exit_code": exit_code,
                "meta": meta,
            }))
        }
        Ok(Err(e)) => Err(crate::error::RuntimeError::tool_execution(format!(
            "powershell execution failed: {}",
            e
        ))),
        Err(_) => Ok(serde_json::json!({
            "stdout": "",
            "stderr": "command timed out",
            "exit_code": -1,
            "meta": crate::builtin::helpers::BuiltinMeta::empty(),
        })),
    }
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
