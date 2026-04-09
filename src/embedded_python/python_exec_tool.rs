use crate::error::LoopResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::{json, Value};

#[cfg(feature = "embedded-python")]
use crate::embedded_python::types::{
    ChildCallStatus, ScriptErrorKind, ScriptExecStatus, ScriptInput, ScriptOutput,
};
#[cfg(feature = "embedded-python")]
use crate::embedded_python::{ScriptRun, ToolExecutorFn};
#[cfg(feature = "embedded-python")]
use std::sync::Arc;

pub struct PythonExecTool {
    definition: ToolDefinition,
    #[cfg(feature = "embedded-python")]
    tool_executor: Option<Arc<ToolExecutorFn>>,
}

impl Default for PythonExecTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonExecTool {
    pub fn new() -> Self {
        let definition = ToolDefinition::new(
            "python_exec",
            "Execute a Python script in the embedded Monty runtime. The script receives `input` and a `tools` namespace derived from the visible runtime tool catalog. Prefer `await tools.<tool_name>(payload)` or `await tools.call(name, payload)` for orchestration, and use `asyncio.gather(...)` for parallel calls. `iron_call(name, args)` remains available as a low-level fallback. The last expression is the result.",
            json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "Python source code to execute"
                    },
                    "input": {
                        "type": "object",
                        "description": "Structured input payload available as `input` in the script"
                    }
                },
                "required": ["script"]
            }),
        );
        Self {
            definition,
            #[cfg(feature = "embedded-python")]
            tool_executor: None,
        }
    }

    #[cfg(feature = "embedded-python")]
    pub fn with_tool_executor(mut self, executor: Arc<ToolExecutorFn>) -> Self {
        self.tool_executor = Some(executor);
        self
    }

    #[cfg(feature = "embedded-python")]
    fn execute_impl(&self, _call_id: &str, arguments: Value) -> LoopResult<Value> {
        let script = arguments
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::LoopError::tool_execution("missing 'script' argument"))?
            .to_string();

        let input = arguments.get("input").cloned().unwrap_or(json!({}));

        let config = crate::config::EmbeddedPythonConfig::default();
        let cancel_token = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut run = ScriptRun::new(ScriptInput { script, input }, &config, cancel_token);

        if let Some(ref executor) = self.tool_executor {
            run = run.with_tool_executor(executor.clone());
        }

        let output = run.execute();
        Ok(script_output_to_json(&output))
    }

    #[cfg(not(feature = "embedded-python"))]
    fn execute_impl(&self, _call_id: &str, _arguments: Value) -> LoopResult<Value> {
        Err(crate::error::LoopError::tool_execution(
            "embedded Python runtime is not enabled; rebuild with --features embedded-python",
        ))
    }
}

#[cfg(feature = "embedded-python")]
pub fn script_output_to_json(output: &ScriptOutput) -> Value {
    let status_str = match output.status {
        ScriptExecStatus::Completed => "completed",
        ScriptExecStatus::CompletedWithFailures => "completed_with_failures",
        ScriptExecStatus::Failed => "failed",
        ScriptExecStatus::Cancelled => "cancelled",
    };

    let mut result = json!({ "status": status_str });

    if let Some(ref val) = output.result {
        result["result"] = val.clone();
    }

    if let Some(ref err) = output.error {
        result["error"] = json!({
            "kind": match err.kind {
                ScriptErrorKind::Timeout => "timeout",
                ScriptErrorKind::SourceTooLarge => "source_too_large",
                ScriptErrorKind::ResultTooLarge => "result_too_large",
                ScriptErrorKind::Runtime => "runtime",
                ScriptErrorKind::ChildCallLimitExceeded => "child_call_limit_exceeded",
                ScriptErrorKind::Cancelled => "cancelled",
                ScriptErrorKind::SandboxViolation => "sandbox_violation",
            },
            "message": err.message,
        });
    }

    if !output.child_outcomes.is_empty() {
        let child_outcomes: Vec<Value> = output
            .child_outcomes
            .iter()
            .map(|o| {
                json!({
                    "call_id": o.call_id,
                    "tool_name": o.tool_name,
                    "status": match o.status {
                        ChildCallStatus::Completed => "completed",
                        ChildCallStatus::Failed => "failed",
                        ChildCallStatus::Denied => "denied",
                        ChildCallStatus::Cancelled => "cancelled",
                    },
                    "result": o.result,
                })
            })
            .collect();
        result["child_outcomes"] = json!(child_outcomes);
    }

    result
}

impl Tool for PythonExecTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(&self, call_id: &str, arguments: Value) -> ToolFuture {
        let result = self.execute_impl(call_id, arguments);
        Box::pin(async move { result })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

impl std::fmt::Debug for PythonExecTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PythonExecTool")
            .field("name", &self.definition.name)
            .finish()
    }
}
