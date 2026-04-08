use crate::config::EmbeddedPythonConfig;
use crate::embedded_python::convert::{json_to_monty, make_iron_exception, monty_to_json};
use crate::embedded_python::types::{
    ChildCallOutcome, ChildCallStatus, ScriptError, ScriptExecStatus, ScriptInput, ScriptOutput,
};
use monty::{
    ExcType, ExtFunctionResult, LimitedTracker, MontyException, MontyObject, MontyRun,
    NameLookupResult, PrintWriter, ResourceLimits, RunProgress,
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type ToolExecutorFn =
    dyn Fn(&str, &str, Value) -> (ChildCallStatus, Option<Value>) + Send + Sync;

pub struct ScriptRun {
    pub input: ScriptInput,
    pub config: EmbeddedPythonConfig,
    cancel_token: Arc<AtomicBool>,
    child_outcomes: Arc<Mutex<Vec<ChildCallOutcome>>>,
    tool_executor: Option<Arc<ToolExecutorFn>>,
}

impl ScriptRun {
    pub fn new(
        input: ScriptInput,
        config: &EmbeddedPythonConfig,
        cancel_token: Arc<AtomicBool>,
    ) -> Self {
        Self {
            input,
            config: config.clone(),
            cancel_token,
            child_outcomes: Arc::new(Mutex::new(Vec::new())),
            tool_executor: None,
        }
    }

    pub fn with_tool_executor(mut self, executor: Arc<ToolExecutorFn>) -> Self {
        self.tool_executor = Some(executor);
        self
    }

    pub fn execute(&self) -> ScriptOutput {
        if self.cancel_token.load(Ordering::SeqCst) {
            return ScriptOutput::cancelled();
        }

        if self.input.script.len() > self.config.max_source_bytes {
            return ScriptOutput::failed(ScriptError::source_too_large(
                self.config.max_source_bytes,
            ));
        }

        let runner = match MontyRun::new(
            self.input.script.clone(),
            "script",
            vec!["input".to_string()],
        ) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{}", e);
                return ScriptOutput::failed(ScriptError::runtime(msg));
            }
        };

        let monty_input = json_to_monty(&self.input.input);

        let limits = ResourceLimits::new()
            .max_duration(Duration::from_secs(self.config.max_script_timeout_secs));
        let tracker = LimitedTracker::new(limits);

        let progress = match runner.start(vec![monty_input], tracker, PrintWriter::Disabled) {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("{}", e);
                return if msg.contains("TimeoutError") {
                    ScriptOutput::failed(ScriptError::timeout())
                } else {
                    ScriptOutput::failed(ScriptError::runtime(msg))
                };
            }
        };

        self.run_loop(progress)
    }

    fn run_loop(&self, initial: RunProgress<LimitedTracker>) -> ScriptOutput {
        let mut current = initial;
        let mut pending_tool_calls: Vec<(String, Value, String)> = Vec::new();
        let max_calls = self.config.max_child_calls;
        let mut call_count = 0usize;

        loop {
            if self.cancel_token.load(Ordering::SeqCst) {
                return ScriptOutput::cancelled();
            }

            if call_count > max_calls {
                return ScriptOutput::failed(ScriptError::child_call_limit(max_calls));
            }

            current = match current {
                RunProgress::Complete(value) => {
                    let result = monty_to_json(&value);
                    let result_size = serde_json::to_string(&result).map(|s| s.len()).unwrap_or(0);
                    if result_size > self.config.max_result_bytes {
                        return ScriptOutput::failed(ScriptError::result_too_large(
                            self.config.max_result_bytes,
                        ));
                    }
                    let child_outcomes = self.take_child_outcomes();
                    let mut output = ScriptOutput::completed(result, child_outcomes);
                    let has_child_failure = output
                        .child_outcomes
                        .iter()
                        .any(|o| o.status != ChildCallStatus::Completed);
                    if has_child_failure {
                        output.status = ScriptExecStatus::CompletedWithFailures;
                    }
                    return output;
                }

                RunProgress::NameLookup(lookup) => {
                    let result = if lookup.name == "iron_call" {
                        NameLookupResult::Value(MontyObject::Function {
                            name: "iron_call".to_string(),
                            docstring: Some(
                                "Call an Iron tool. Usage: await iron_call(name, args)".to_string(),
                            ),
                        })
                    } else {
                        NameLookupResult::Undefined
                    };
                    match lookup.resume(result, PrintWriter::Disabled) {
                        Ok(next) => next,
                        Err(e) => {
                            return ScriptOutput::failed(ScriptError::runtime(format!("{}", e)));
                        }
                    }
                }

                RunProgress::FunctionCall(call) => {
                    if call.function_name == "iron_call" {
                        call_count += 1;
                        let tool_name = call
                            .args
                            .first()
                            .and_then(|a| match a {
                                MontyObject::String(s) => Some(s.clone()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let tool_args = call.args.get(1).map(monty_to_json).unwrap_or(Value::Null);
                        let iron_call_id = uuid::Uuid::new_v4().to_string();

                        pending_tool_calls.push((tool_name, tool_args, iron_call_id));

                        match call.resume_pending(PrintWriter::Disabled) {
                            Ok(next) => next,
                            Err(e) => {
                                return ScriptOutput::failed(ScriptError::runtime(format!(
                                    "{}",
                                    e
                                )));
                            }
                        }
                    } else {
                        let exc = MontyException::new(
                            ExcType::NameError,
                            Some(format!("name '{}' is not defined", call.function_name)),
                        );
                        match call.resume(ExtFunctionResult::Error(exc), PrintWriter::Disabled) {
                            Ok(next) => next,
                            Err(e) => {
                                return ScriptOutput::failed(ScriptError::runtime(format!(
                                    "{}",
                                    e
                                )));
                            }
                        }
                    }
                }

                RunProgress::ResolveFutures(state) => {
                    let pending_ids = state.pending_call_ids();
                    let mut results = Vec::with_capacity(pending_ids.len());

                    for (idx, &call_id) in pending_ids.iter().enumerate() {
                        if idx < pending_tool_calls.len() {
                            let (tool_name, tool_args, iron_call_id) =
                                pending_tool_calls[idx].clone();
                            let (status, result) =
                                self.execute_single_tool(&iron_call_id, &tool_name, tool_args);

                            let ext_result = match status {
                                ChildCallStatus::Completed => {
                                    let obj = result
                                        .map(|v| json_to_monty(&v))
                                        .unwrap_or(MontyObject::None);
                                    ExtFunctionResult::Return(obj)
                                }
                                ChildCallStatus::Failed => {
                                    let msg = result
                                        .and_then(|v| {
                                            v.get("error")
                                                .and_then(|e| e.as_str().map(String::from))
                                        })
                                        .unwrap_or_else(|| "tool call failed".to_string());
                                    ExtFunctionResult::Error(make_iron_exception(
                                        "ToolFailedError",
                                        &msg,
                                    ))
                                }
                                ChildCallStatus::Denied => ExtFunctionResult::Error(
                                    make_iron_exception("ToolDeniedError", "tool call was denied"),
                                ),
                                ChildCallStatus::Cancelled => {
                                    ExtFunctionResult::Error(make_iron_exception(
                                        "ToolCancelledError",
                                        "tool call was cancelled",
                                    ))
                                }
                            };
                            results.push((call_id, ext_result));
                        }
                    }
                    pending_tool_calls.clear();

                    match state.resume(results, PrintWriter::Disabled) {
                        Ok(next) => next,
                        Err(e) => {
                            let child_outcomes = self.take_child_outcomes();
                            let mut output =
                                ScriptOutput::failed(ScriptError::runtime(format!("{}", e)));
                            output.child_outcomes = child_outcomes;
                            return output;
                        }
                    }
                }

                RunProgress::OsCall(call) => {
                    let exc = MontyException::new(
                        ExcType::RuntimeError,
                        Some("OS access not available in sandbox".to_string()),
                    );
                    match call.resume(ExtFunctionResult::Error(exc), PrintWriter::Disabled) {
                        Ok(next) => next,
                        Err(e) => {
                            return ScriptOutput::failed(ScriptError::runtime(format!("{}", e)));
                        }
                    }
                }
            };
        }
    }

    fn execute_single_tool(
        &self,
        call_id: &str,
        tool_name: &str,
        args: Value,
    ) -> (ChildCallStatus, Option<Value>) {
        let executor = match self.tool_executor.as_ref() {
            Some(e) => e,
            None => {
                let error = serde_json::json!({"error": "no tool executor configured"});
                self.child_outcomes.lock().unwrap().push(ChildCallOutcome {
                    call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    status: ChildCallStatus::Failed,
                    result: Some(error.clone()),
                });
                return (ChildCallStatus::Failed, Some(error));
            }
        };

        let (status, result) = executor(call_id, tool_name, args);
        self.child_outcomes.lock().unwrap().push(ChildCallOutcome {
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            status,
            result: result.clone(),
        });
        (status, result)
    }

    fn take_child_outcomes(&self) -> Vec<ChildCallOutcome> {
        std::mem::take(&mut *self.child_outcomes.lock().unwrap())
    }
}

pub struct ScriptEngine {
    pub config: EmbeddedPythonConfig,
}

impl ScriptEngine {
    pub fn new(config: &EmbeddedPythonConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn create_run(&self, input: ScriptInput) -> ScriptRun {
        let cancel_token = Arc::new(AtomicBool::new(false));
        ScriptRun::new(input, &self.config, cancel_token)
    }
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("enabled", &self.config.enabled)
            .finish()
    }
}
