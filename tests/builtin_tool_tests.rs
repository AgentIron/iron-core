use iron_core::builtin::{
    register_builtin_tools, BuiltinErrorCode, BuiltinToolConfig, BuiltinToolError,
    BuiltinToolPolicy, NetworkPolicy, ShellAvailability,
};
use iron_core::tool::{Tool, ToolRegistry};
use std::path::PathBuf;
use tempfile::TempDir;

fn temp_config() -> (TempDir, BuiltinToolConfig) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let config = BuiltinToolConfig::new(vec![root]);
    (tmp, config)
}

fn mark_file_as_read(config: &BuiltinToolConfig, path: &std::path::Path) {
    let tool = iron_core::builtin::file_ops::ReadTool::new(config.clone());
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(tool.execute(
        "prep-read",
        serde_json::json!({ "path": path.to_str().unwrap() }),
    ))
    .unwrap();
}

#[test]
fn policy_validates_absolute_allowed_root() {
    let tmp = TempDir::new().unwrap();
    let config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()]);
    assert!(config.validate().is_ok());
}

#[test]
fn policy_rejects_relative_allowed_root() {
    let config = BuiltinToolConfig::new(vec![PathBuf::from("relative/path")]);
    assert!(config.validate().is_err());
}

#[test]
fn policy_rejects_empty_allowed_roots() {
    let config = BuiltinToolConfig {
        allowed_roots: vec![],
        ..BuiltinToolConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn path_validation_accepts_file_within_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let policy = BuiltinToolPolicy::default();
    let file_path = root.join("test.txt");
    std::fs::write(&file_path, "hello").unwrap();
    let result = policy.validate_path(&file_path, &[root]);
    assert!(result.is_ok());
}

#[test]
fn path_validation_rejects_file_outside_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let policy = BuiltinToolPolicy::default();
    let outside = PathBuf::from("/etc/passwd");
    let result = policy.validate_path(&outside, &[root]);
    assert!(result.is_err());
}

#[test]
fn builtin_error_code_to_json() {
    let err = BuiltinToolError::out_of_scope("test path");
    let json = err.to_json();
    assert_eq!(json["error"]["code"], "path_out_of_scope");
    assert_eq!(json["error"]["message"], "test path");
}

#[test]
fn builtin_error_codes_are_stable() {
    assert_eq!(
        BuiltinErrorCode::PathOutOfScope.as_str(),
        "path_out_of_scope"
    );
    assert_eq!(BuiltinErrorCode::EditMismatch.as_str(), "edit_mismatch");
    assert_eq!(BuiltinErrorCode::EditAmbiguous.as_str(), "edit_ambiguous");
    assert_eq!(BuiltinErrorCode::BinaryContent.as_str(), "binary_content");
    assert_eq!(BuiltinErrorCode::Timeout.as_str(), "timeout");
    assert_eq!(BuiltinErrorCode::NetworkDenied.as_str(), "network_denied");
}

#[test]
fn disabled_tool_is_not_registered() {
    let tmp = TempDir::new().unwrap();
    let config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()])
        .with_disabled_tools(vec!["read".to_string(), "bash".to_string()]);
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry, &config);
    assert!(registry.get("read").is_none());
    assert!(registry.get("write").is_some());
}

#[test]
fn shell_availability_detect_runs_without_panic() {
    let avail = ShellAvailability::detect();
    match avail {
        ShellAvailability::Bash => assert!(avail.tool_name() == Some("bash")),
        ShellAvailability::PowerShell => assert!(avail.tool_name() == Some("powershell")),
        ShellAvailability::None => assert!(avail.tool_name().is_none()),
    }
}

#[test]
fn shell_not_advertised_when_none() {
    let tmp = TempDir::new().unwrap();
    let config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()])
        .with_shell_availability(ShellAvailability::None);
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry, &config);
    assert!(registry.get("bash").is_none());
    assert!(registry.get("powershell").is_none());
}

#[test]
fn bash_advertised_over_powershell() {
    let tmp = TempDir::new().unwrap();
    let config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()])
        .with_shell_availability(ShellAvailability::Bash);
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry, &config);
    assert!(registry.get("bash").is_some());
    assert!(registry.get("powershell").is_none());
}

#[test]
fn standard_tools_registered_by_default() {
    let tmp = TempDir::new().unwrap();
    let config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()]);
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry, &config);
    assert!(registry.get("read").is_some());
    assert!(registry.get("write").is_some());
    assert!(registry.get("edit").is_some());
    assert!(registry.get("glob").is_some());
    assert!(registry.get("grep").is_some());
    assert!(registry.get("webfetch").is_some());
}

#[test]
fn read_tool_rejects_out_of_scope_path() {
    let (_tmp, config) = temp_config();
    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-1",
        serde_json::json!({
            "path": "/etc/shadow"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn read_tool_reads_file_content() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("hello.txt");
    std::fs::write(&file_path, "line one\nline two\nline three\n").unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-2",
            serde_json::json!({
                "path": file_path.to_str().unwrap()
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("line one"));
    assert!(content.contains("line three"));
    assert!(result.get("meta").is_some());
}

#[test]
fn read_tool_detects_binary() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("binary.bin");
    std::fs::write(&file_path, [0u8, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3]).unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-3",
            serde_json::json!({
                "path": file_path.to_str().unwrap()
            }),
        ))
        .unwrap();

    assert!(result.get("is_binary").unwrap().as_bool().unwrap());
}

#[test]
fn read_tool_supports_offset_and_limit() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("lines.txt");
    std::fs::write(&file_path, "a\nb\nc\nd\ne\n").unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-4",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "offset": 2,
                "limit": 2
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("2: b"));
    assert!(content.contains("3: c"));
    assert!(!content.contains("1: a"));
    assert!(!content.contains("4: d"));
}

#[test]
fn read_tool_reports_continuation_metadata() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("long.txt");
    let content: Vec<String> = (0..100).map(|i| format!("line {}", i)).collect();
    std::fs::write(&file_path, content.join("\n")).unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-5",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "limit": 10
            }),
        ))
        .unwrap();

    let meta = result.get("meta").unwrap();
    assert!(meta.get("truncated").unwrap().as_bool().unwrap());
    assert!(meta.get("continuation_offset").is_some());
}

#[test]
fn write_tool_creates_file() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("new_file.txt");

    let tool = iron_core::builtin::file_ops::WriteTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-6",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "hello world"
            }),
        ))
        .unwrap();

    let internal = result.get("internal").unwrap();
    assert!(internal.get("created").unwrap().as_bool().unwrap());
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello world");
}

#[test]
fn write_tool_creates_missing_parent_directories() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("a/b/c/deep.txt");

    let tool = iron_core::builtin::file_ops::WriteTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-7",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "deep"
            }),
        ))
        .unwrap();

    let internal = result.get("internal").unwrap();
    assert!(internal.get("created").unwrap().as_bool().unwrap());
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "deep");
}

#[test]
fn write_tool_rejects_out_of_scope() {
    let (_tmp, config) = temp_config();
    let tool = iron_core::builtin::file_ops::WriteTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-8",
        serde_json::json!({
            "path": "/tmp/outside.txt",
            "content": "nope"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn edit_tool_applies_exact_replacement() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("edit_me.txt");
    std::fs::write(&file_path, "foo bar baz").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-9",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_string": "bar",
                "new_string": "BAR"
            }),
        ))
        .unwrap();

    assert_eq!(result["internal"]["replacements"].as_u64().unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "foo BAR baz");
}

#[test]
fn edit_tool_rejects_missing_text() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("edit_missing.txt");
    std::fs::write(&file_path, "foo bar baz").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-10",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "old_string": "not found",
            "new_string": "replacement"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn edit_tool_rejects_ambiguous_match() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("edit_ambig.txt");
    std::fs::write(&file_path, "abc abc").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-11",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "old_string": "abc",
            "new_string": "XYZ"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn glob_tool_finds_matching_files() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("foo.rs"), "").unwrap();
    std::fs::write(tmp.path().join("bar.rs"), "").unwrap();
    std::fs::write(tmp.path().join("baz.txt"), "").unwrap();

    let tool = iron_core::builtin::search::GlobTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-12",
            serde_json::json!({
                "pattern": "*.rs",
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let internal = result.get("internal").unwrap();
    let paths = internal.get("paths").unwrap().as_array().unwrap();
    assert_eq!(paths.len(), 2);
}

#[test]
fn glob_tool_respects_result_bounds() {
    let (tmp, mut config) = temp_config();
    config.max_glob_results = 2;
    for i in 0..5 {
        std::fs::write(tmp.path().join(format!("file_{}.txt", i)), "").unwrap();
    }

    let tool = iron_core::builtin::search::GlobTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-13",
            serde_json::json!({
                "pattern": "*.txt",
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let internal = result.get("internal").unwrap();
    assert!(internal.get("truncated").unwrap().as_bool().unwrap());
}

#[test]
fn grep_tool_finds_matches() {
    let (tmp, config) = temp_config();
    std::fs::write(
        tmp.path().join("hello.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("other.txt"), "no match here\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-14",
            serde_json::json!({
                "pattern": "hello",
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("hello"));
}

#[test]
fn grep_tool_applies_include_filter() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("code.rs"), "fn find() {}\n").unwrap();
    std::fs::write(tmp.path().join("notes.txt"), "find me\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-15",
            serde_json::json!({
                "pattern": "find",
                "path": tmp.path().to_str().unwrap(),
                "include": "*.rs"
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("code.rs"));
    assert!(!content.contains("notes.txt"));
}

#[test]
fn approval_scope_matching() {
    use iron_core::builtin::policy::{ApprovalScope, ApprovalScopeMatch};

    let scope = ApprovalScope::Command("cargo test".to_string());
    assert!(scope.matches(&ApprovalScopeMatch::Command("cargo test".to_string())));
    assert!(!scope.matches(&ApprovalScopeMatch::Command("cargo build".to_string())));

    let dir_scope = ApprovalScope::DirectoryPath(PathBuf::from("/home/user/project"));
    assert!(
        dir_scope.matches(&ApprovalScopeMatch::FilePath(PathBuf::from(
            "/home/user/project/src/main.rs"
        )))
    );
    assert!(!dir_scope.matches(&ApprovalScopeMatch::FilePath(PathBuf::from("/etc/passwd"))));

    let domain_scope = ApprovalScope::Domain("example.com".to_string());
    assert!(domain_scope.matches(&ApprovalScopeMatch::Domain("example.com".to_string())));
    assert!(domain_scope.matches(&ApprovalScopeMatch::Domain("sub.example.com".to_string())));
    assert!(!domain_scope.matches(&ApprovalScopeMatch::Domain("other.com".to_string())));

    let all_scope = ApprovalScope::All;
    assert!(all_scope.matches(&ApprovalScopeMatch::Command("anything".to_string())));
}

#[test]
fn network_policy_deny_blocks_fetch() {
    let (_tmp, mut config) = temp_config();
    config.policy.network = NetworkPolicy::DenyAll;

    let tool = iron_core::builtin::web::WebFetchTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-net",
        serde_json::json!({
            "url": "https://example.com"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn meta_empty_always_includes_object() {
    let meta = iron_core::builtin::helpers::BuiltinMeta::empty();
    assert!(meta.is_object());
}

#[test]
fn meta_truncation_includes_fields() {
    let meta = iron_core::builtin::helpers::BuiltinMeta::with_truncation(1024);
    assert!(meta.get("truncated").unwrap().as_bool().unwrap());
    assert_eq!(meta.get("total_bytes").unwrap().as_u64().unwrap(), 1024);
}

#[test]
fn meta_continuation_includes_offset() {
    let meta = iron_core::builtin::helpers::BuiltinMeta::with_continuation(50, 1000);
    assert!(meta.get("truncated").unwrap().as_bool().unwrap());
    assert_eq!(
        meta.get("continuation_offset").unwrap().as_u64().unwrap(),
        50
    );
    assert_eq!(meta.get("total_bytes").unwrap().as_u64().unwrap(), 1000);
}

#[test]
fn smoke_test_register_all_builtin_tools_via_agent() {
    let config = iron_core::Config::default();
    let provider = iron_providers::OpenAiProvider::new(iron_providers::OpenAiConfig::new(
        "test-key".to_string(),
    ))
    .expect("test provider config should be valid");
    let agent = iron_core::IronAgent::new(config, provider);

    let tmp = TempDir::new().unwrap();
    let builtin_config = BuiltinToolConfig::new(vec![tmp.path().to_path_buf()]);
    agent.register_builtin_tools(&builtin_config);

    let registry = agent.runtime().tool_registry();
    assert!(registry.contains("read"));
    assert!(registry.contains("write"));
    assert!(registry.contains("edit"));
    assert!(registry.contains("multiedit"));
    assert!(registry.contains("glob"));
    assert!(registry.contains("grep"));
    assert!(registry.contains("webfetch"));
}

// ---------------------------------------------------------------------------
// New tests for efficiency improvements
// ---------------------------------------------------------------------------

#[test]
fn read_directory_returns_listing() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("file.txt"), "content").unwrap();
    std::fs::create_dir(tmp.path().join("subdir")).unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-dir-read",
            serde_json::json!({
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    assert!(result.get("is_directory").unwrap().as_bool().unwrap());
    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("file.txt"));
    assert!(content.contains("subdir/"));
    // Should NOT contain . or ..
    assert!(!content.contains("/.\n"));
    assert!(!content.contains("/..\n"));
}

#[test]
fn read_directory_includes_hidden_entries() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join(".hidden"), "secret").unwrap();
    std::fs::write(tmp.path().join("visible.txt"), "public").unwrap();

    let tool = iron_core::builtin::file_ops::ReadTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-dir-hidden",
            serde_json::json!({
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains(".hidden"));
    assert!(content.contains("visible.txt"));
}

#[test]
fn edit_replace_all_replaces_every_occurrence() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("multi.txt");
    std::fs::write(&file_path, "abc abc abc").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-replace-all",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "old_string": "abc",
                "new_string": "XYZ",
                "replace_all": true
            }),
        ))
        .unwrap();

    assert_eq!(result["internal"]["replacements"].as_u64().unwrap(), 3);
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "XYZ XYZ XYZ");
}

#[test]
fn edit_replace_all_fails_on_zero_matches() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("no_match.txt");
    std::fs::write(&file_path, "foo bar").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-replace-all-miss",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "old_string": "not_here",
            "new_string": "XYZ",
            "replace_all": true
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn edit_requires_prior_read() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("must_read.txt");
    std::fs::write(&file_path, "foo").unwrap();

    let tool = iron_core::builtin::file_ops::EditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-edit-read-required",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "old_string": "foo",
            "new_string": "bar"
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn multiedit_applies_multiple_edits_atomically() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("multi_edit.txt");
    std::fs::write(&file_path, "one two three").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::MultieditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-multiedit",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "edits": [
                    { "old_string": "one", "new_string": "ONE" },
                    { "old_string": "two", "new_string": "TWO" },
                    { "old_string": "three", "new_string": "THREE" }
                ]
            }),
        ))
        .unwrap();

    assert_eq!(result["internal"]["edits_applied"].as_u64().unwrap(), 3);
    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "ONE TWO THREE"
    );
}

#[test]
fn multiedit_is_atomic_on_failure() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("atomic.txt");
    let original = "alpha beta gamma";
    std::fs::write(&file_path, original).unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::MultieditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-multiedit-atomic",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "edits": [
                { "old_string": "alpha", "new_string": "ALPHA" },
                { "old_string": "not_found", "new_string": "FAIL" }
            ]
        }),
    ));
    assert!(result.is_err());
    // File should be unchanged (atomic rollback).
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), original);
}

#[test]
fn multiedit_supports_replace_all_per_item() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("per_item.txt");
    std::fs::write(&file_path, "foo foo bar bar").unwrap();
    mark_file_as_read(&config, &file_path);

    let tool = iron_core::builtin::file_ops::MultieditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _result = rt
        .block_on(tool.execute(
            "test-multiedit-replace-all",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "edits": [
                    { "old_string": "foo", "new_string": "FOO", "replace_all": true },
                    { "old_string": "bar", "new_string": "BAR", "replace_all": true }
                ]
            }),
        ))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(&file_path).unwrap(),
        "FOO FOO BAR BAR"
    );
}

#[test]
fn multiedit_requires_prior_read() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("must_read_multi.txt");
    std::fs::write(&file_path, "foo bar").unwrap();

    let tool = iron_core::builtin::file_ops::MultieditTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(
        "test-multiedit-read-required",
        serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "edits": [{"old_string": "foo", "new_string": "FOO"}]
        }),
    ));
    assert!(result.is_err());
}

#[test]
fn grep_mode_files_with_matches() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("a.rs"), "fn find() {}\n").unwrap();
    std::fs::write(tmp.path().join("b.rs"), "fn other() {}\n").unwrap();
    std::fs::write(tmp.path().join("c.txt"), "find me\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-grep-files",
            serde_json::json!({
                "pattern": "find",
                "path": tmp.path().to_str().unwrap(),
                "mode": "files_with_matches"
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("a.rs"));
    assert!(content.contains("c.txt"));
    assert!(!content.contains("b.rs"));
}

#[test]
fn grep_mode_count_returns_totals() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("a.rs"), "find one\nfind two\n").unwrap();
    std::fs::write(tmp.path().join("b.rs"), "find three\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-grep-count",
            serde_json::json!({
                "pattern": "find",
                "path": tmp.path().to_str().unwrap(),
                "mode": "count"
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("3 matches total"));
    assert!(content.contains("a.rs: 2"));
    assert!(content.contains("b.rs: 1"));
}

#[test]
fn grep_case_insensitive() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("mixed.txt"), "Hello World\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-grep-ci",
            serde_json::json!({
                "pattern": "hello",
                "path": tmp.path().to_str().unwrap(),
                "case_insensitive": true
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("Hello"));
}

#[test]
fn write_output_is_compact_summary() {
    let (tmp, config) = temp_config();
    let file_path = tmp.path().join("summary.txt");

    let tool = iron_core::builtin::file_ops::WriteTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-write-summary",
            serde_json::json!({
                "path": file_path.to_str().unwrap(),
                "content": "test"
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.starts_with("Created"));
}

#[test]
fn glob_output_is_compact_paths() {
    let (tmp, config) = temp_config();
    std::fs::write(tmp.path().join("a.rs"), "").unwrap();
    std::fs::write(tmp.path().join("b.rs"), "").unwrap();

    let tool = iron_core::builtin::search::GlobTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-glob-compact",
            serde_json::json!({
                "pattern": "*.rs",
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let content = result.get("content").unwrap().as_str().unwrap();
    assert!(content.contains("a.rs"));
    assert!(content.contains("b.rs"));
    // Should NOT contain JSON keys like "paths" or "count"
    assert!(!content.contains("\"paths\""));
}

#[test]
fn grep_explicit_hidden_include_overrides_default_filters() {
    let (tmp, config) = temp_config();
    std::fs::create_dir_all(tmp.path().join(".hidden")).unwrap();
    std::fs::write(tmp.path().join(".hidden/secret.txt"), "needle\n").unwrap();

    let tool = iron_core::builtin::search::GrepTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-grep-hidden-override",
            serde_json::json!({
                "pattern": "needle",
                "path": tmp.path().to_str().unwrap(),
                "include": ".hidden/**"
            }),
        ))
        .unwrap();

    let content = result["content"].as_str().unwrap();
    assert!(content.contains(".hidden/secret.txt"));
}

#[test]
fn glob_explicit_hidden_pattern_overrides_default_filters() {
    let (tmp, config) = temp_config();
    std::fs::create_dir_all(tmp.path().join(".hidden")).unwrap();
    std::fs::write(tmp.path().join(".hidden/file.txt"), "x").unwrap();

    let tool = iron_core::builtin::search::GlobTool::new(config);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(tool.execute(
            "test-glob-hidden-override",
            serde_json::json!({
                "pattern": ".hidden/**",
                "path": tmp.path().to_str().unwrap()
            }),
        ))
        .unwrap();

    let content = result["content"].as_str().unwrap();
    assert!(content.contains(".hidden/file.txt"));
}
