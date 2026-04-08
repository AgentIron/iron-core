#![cfg(feature = "embedded-python")]

use iron_core::embedded_python::convert::{json_to_monty, monty_to_json};
use iron_core::embedded_python::PythonExecTool;
use iron_core::embedded_python::{
    ChildCallStatus, ScriptEngine, ScriptErrorKind, ScriptExecStatus, ScriptInput, ScriptRun,
};
use iron_core::tool::Tool;
use iron_core::EmbeddedPythonConfig;
use monty::MontyObject;
use serde_json::json;
use std::sync::Arc;

fn default_config() -> EmbeddedPythonConfig {
    EmbeddedPythonConfig::default()
}

fn make_engine() -> ScriptEngine {
    ScriptEngine::new(&default_config())
}

#[test]
fn test_simple_script_returns_result() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "2 + 2".into(),
        input: json!({}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!(4)));
    assert!(output.error.is_none());
}

#[test]
fn test_script_receives_input() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "input['x'] + input['y']".into(),
        input: json!({"x": 10, "y": 32}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!(42)));
}

#[test]
fn test_script_syntax_error() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "def foo(".into(),
        input: json!({}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert!(output.error.is_some());
    assert_eq!(
        output.error.as_ref().unwrap().kind,
        ScriptErrorKind::Runtime
    );
}

#[test]
fn test_script_runtime_error() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "1 / 0".into(),
        input: json!({}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert!(output.error.is_some());
}

#[test]
fn test_script_source_too_large() {
    let mut config = default_config();
    config.max_source_bytes = 10;
    let engine = ScriptEngine::new(&config);

    let input = ScriptInput {
        script: "'this script is way more than ten bytes'".into(),
        input: json!({}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    let err = output.error.unwrap();
    assert_eq!(err.kind, ScriptErrorKind::SourceTooLarge);
}

#[test]
fn test_script_cancellation() {
    let cancel_token = Arc::new(std::sync::atomic::AtomicBool::new(true));

    let input = ScriptInput {
        script: "42".into(),
        input: json!({}),
    };
    let run = ScriptRun::new(input, &default_config(), cancel_token);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Cancelled);
    assert!(output.error.is_some());
    assert_eq!(output.error.unwrap().kind, ScriptErrorKind::Cancelled);
}

#[test]
fn test_mixed_success_failure_child_calls() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "a = await iron_call('good_tool', {})\nb = await iron_call('bad_tool', {})\n[a, b]"
            .into(),
        input: json!({}),
    };

    let executor = |_call_id: &str, name: &str, _args: serde_json::Value| {
        if name == "good_tool" {
            (ChildCallStatus::Completed, Some(json!("ok")))
        } else {
            (ChildCallStatus::Failed, Some(json!({"error": "failed"})))
        }
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(output.child_outcomes.len(), 2);
    assert_eq!(output.child_outcomes[0].status, ChildCallStatus::Completed);
    assert_eq!(output.child_outcomes[1].status, ChildCallStatus::Failed);
}

#[test]
fn test_denied_child_call() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "await iron_call('denied_tool', {})".into(),
        input: json!({}),
    };

    let executor =
        |_call_id: &str, _name: &str, _args: serde_json::Value| (ChildCallStatus::Denied, None);

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(output.child_outcomes.len(), 1);
    assert_eq!(output.child_outcomes[0].status, ChildCallStatus::Denied);
}

#[test]
fn test_unhandled_child_failure_propagates() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "await iron_call('bad_tool', {})".into(),
        input: json!({}),
    };

    let executor = |_call_id: &str, _name: &str, _args: serde_json::Value| {
        (ChildCallStatus::Failed, Some(json!({"error": "boom"})))
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(output.child_outcomes.len(), 1);
    assert_eq!(output.child_outcomes[0].status, ChildCallStatus::Failed);
}

#[test]
fn test_python_exec_tool_definition() {
    let tool = PythonExecTool::new();
    let def = tool.definition();

    assert_eq!(def.name, "python_exec");
    assert!(tool.requires_approval());
    assert!(def.description.contains("Monty"));
}

#[test]
fn test_iron_call_with_computation() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "val = await iron_call('get_data', {})\nval".into(),
        input: json!({}),
    };

    let executor = |_call_id: &str, _name: &str, _args: serde_json::Value| {
        (
            ChildCallStatus::Completed,
            Some(json!({"items": [1, 2, 3]})),
        )
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!({"items": [1, 2, 3]})));
}

#[test]
fn test_no_tool_executor_reports_failure() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "await iron_call('some_tool', {})".into(),
        input: json!({}),
    };

    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(output.child_outcomes.len(), 1);
    assert_eq!(output.child_outcomes[0].status, ChildCallStatus::Failed);
}

#[test]
fn test_python_exec_tool_missing_script_arg() {
    let tool = PythonExecTool::new();
    let result = tool.execute("call-1", json!({}));

    let output = futures::executor::block_on(result);
    assert!(output.is_err());
}

#[test]
fn test_convert_roundtrip_complex() {
    let original = json!({
        "name": "test",
        "count": 42,
        "active": true,
        "items": [1, 2, 3],
        "nested": {"key": "value"},
        "nothing": null
    });
    let monty_val = json_to_monty(&original);
    let roundtripped = monty_to_json(&monty_val);
    assert_eq!(original, roundtripped);
}

#[test]
fn test_convert_null() {
    assert_eq!(json_to_monty(&json!(null)), MontyObject::None);
    assert_eq!(monty_to_json(&MontyObject::None), json!(null));
}

#[test]
fn test_convert_bool() {
    assert_eq!(json_to_monty(&json!(true)), MontyObject::Bool(true));
    assert_eq!(monty_to_json(&MontyObject::Bool(true)), json!(true));
}

#[test]
fn test_engine_is_enabled_default() {
    let config = EmbeddedPythonConfig::default();
    let engine = ScriptEngine::new(&config);
    assert!(!engine.is_enabled());
}

#[test]
fn test_engine_is_enabled_when_set() {
    let config = EmbeddedPythonConfig {
        enabled: true,
        ..Default::default()
    };
    let engine = ScriptEngine::new(&config);
    assert!(engine.is_enabled());
}

#[test]
fn test_local_compute_with_nested_input() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "input['items'][0] + input['items'][1]".into(),
        input: json!({"items": [10, 20, 30], "label": "sum"}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!(30)));
}

#[test]
fn test_local_compute_returns_none_for_null() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "input.get('missing', None)".into(),
        input: json!({}),
    };
    let run = engine.create_run(input);
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!(null)));
}

#[test]
fn test_sequential_tool_calls() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "a = await iron_call('step_one', {})\nb = await iron_call('step_two', {})\n[a, b]"
            .into(),
        input: json!({}),
    };

    let executor = |_call_id: &str, name: &str, _args: serde_json::Value| match name {
        "step_one" => (ChildCallStatus::Completed, Some(json!(1))),
        "step_two" => (ChildCallStatus::Completed, Some(json!(2))),
        _ => (
            ChildCallStatus::Failed,
            Some(json!({"error": "unknown tool"})),
        ),
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!([1, 2])));
    assert_eq!(output.child_outcomes.len(), 2);
}

#[test]
fn test_tool_call_with_input_dependent_args() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "await iron_call('lookup', {'key': input['query']})".into(),
        input: json!({"query": "test_value"}),
    };

    let executor = |_call_id: &str, _name: &str, args: serde_json::Value| {
        assert_eq!(args["key"], "test_value");
        (ChildCallStatus::Completed, Some(json!({"found": true})))
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Completed);
    assert_eq!(output.result, Some(json!({"found": true})));
}

#[test]
fn test_child_call_limit_exceeded() {
    let mut config = default_config();
    config.max_child_calls = 2;
    let engine = ScriptEngine::new(&config);

    let input = ScriptInput {
        script: "a = await iron_call('t1', {})\nb = await iron_call('t2', {})\nc = await iron_call('t3', {})\n[a, b, c]".into(),
        input: json!({}),
    };

    let executor = |_call_id: &str, _name: &str, _args: serde_json::Value| {
        (ChildCallStatus::Completed, Some(json!("ok")))
    };

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(
        output.error.as_ref().unwrap().kind,
        ScriptErrorKind::ChildCallLimitExceeded
    );
}

#[test]
fn test_cancelled_child_call() {
    let engine = make_engine();
    let input = ScriptInput {
        script: "await iron_call('cancelled_tool', {})".into(),
        input: json!({}),
    };

    let executor =
        |_call_id: &str, _name: &str, _args: serde_json::Value| (ChildCallStatus::Cancelled, None);

    let run = engine
        .create_run(input)
        .with_tool_executor(Arc::new(executor));
    let output = run.execute();

    assert_eq!(output.status, ScriptExecStatus::Failed);
    assert_eq!(output.child_outcomes.len(), 1);
    assert_eq!(output.child_outcomes[0].status, ChildCallStatus::Cancelled);
}

#[test]
fn test_python_exec_tool_executes_script() {
    let tool = PythonExecTool::new();
    let result = tool.execute("test-call-1", json!({"script": "2 + 2"}));

    let output = futures::executor::block_on(result).unwrap();
    assert_eq!(output["status"], "completed");
    assert_eq!(output["result"], 4);
}

#[test]
fn test_python_exec_tool_returns_error_on_bad_script() {
    let tool = PythonExecTool::new();
    let result = tool.execute("test-call-2", json!({"script": "1 / 0"}));

    let output = futures::executor::block_on(result).unwrap();
    assert_eq!(output["status"], "failed");
}

#[test]
fn test_portability_no_native_deps_in_embedded_path() {
    let has_rustpython = option_env!("CARGO_DEPENDENCY_RUSTPYTHON").is_some();
    assert!(!has_rustpython, "RustPython should not be present");
}
