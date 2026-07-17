//! Sandboxed embedded-Python execution and its `python_exec` tool.
//!
//! The always-available types describe script inputs and outputs. Enabling the
//! `embedded-python` feature also exposes JSON conversion helpers and the
//! Monty-backed execution engine. Scripts are constrained by configured source
//! size, result size, duration, and child-tool-call limits and have no direct
//! host OS access.

#[cfg(feature = "embedded-python")]
/// Internal catalog used to expose registered tools to scripts.
pub(crate) mod catalog;
#[cfg(feature = "embedded-python")]
pub mod convert;
#[cfg(feature = "embedded-python")]
pub mod engine;

pub mod python_exec_tool;
pub mod types;

#[cfg(feature = "embedded-python")]
pub(crate) use catalog::{is_tools_namespace, ToolCatalog};
#[cfg(feature = "embedded-python")]
pub use convert::{json_to_monty, make_iron_exception, monty_to_json};
#[cfg(feature = "embedded-python")]
pub use engine::{ScriptEngine, ScriptRun, ToolExecutorFn};
#[cfg(feature = "embedded-python")]
pub use python_exec_tool::script_output_to_json;

pub use python_exec_tool::PythonExecTool;
pub use types::{
    ChildCallOutcome, ChildCallStatus, ScriptError, ScriptErrorKind, ScriptExecStatus, ScriptInput,
    ScriptOutput, ScriptResult,
};
