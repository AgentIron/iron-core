use crate::capability::CapabilityRegistry;
use crate::config::{ApprovalStrategy, Config, EmbeddedPythonConfig};

pub struct RuntimeContextRenderer;

impl RuntimeContextRenderer {
    pub fn render(
        config: &Config,
        capabilities: Option<&CapabilityRegistry>,
        working_dir: &std::path::Path,
        workspace_roots: &[std::path::PathBuf],
        is_git_repo: bool,
        python_exec_available: bool,
    ) -> String {
        let mut ctx = String::new();
        ctx.push_str("<runtime_context>\n");

        ctx.push_str(&format!("Date: {}\n", chrono_date_or_fallback()));
        ctx.push_str(&format!("Platform: {}\n", std::env::consts::OS));
        ctx.push_str(&format!("Working directory: {}\n", working_dir.display()));
        for root in workspace_roots {
            ctx.push_str(&format!("Workspace root: {}\n", root.display()));
        }
        if is_git_repo {
            ctx.push_str("Git repository: yes\n");
        }

        ctx.push_str(&format!(
            "Approval strategy: {}\n",
            approval_strategy_label(config.default_approval_strategy)
        ));

        if !config.prompt_composition.protected_resources.is_empty() {
            ctx.push_str("\nProtected resources:\n");
            ctx.push_str("The following resources must never be read or modified, whether through direct tool use or scripted tool use (e.g. python_exec):\n");
            for res in &config.prompt_composition.protected_resources {
                ctx.push_str(&format!("- {}\n", res));
            }
        }

        if let Some(caps) = capabilities {
            if !caps.is_empty() {
                ctx.push_str("\nCapabilities:\n");
                for (_, desc) in caps.iter() {
                    let perm = if desc.requires_permission {
                        " (may require approval)"
                    } else {
                        ""
                    };
                    ctx.push_str(&format!("- {}{}: {}\n", desc.name, perm, desc.description));
                }
            }
        }

        if config.embedded_python.enabled && python_exec_available {
            ctx.push_str(&render_python_context(&config.embedded_python));
        }

        ctx.push_str("</runtime_context>");
        ctx
    }
}

fn approval_strategy_label(strategy: ApprovalStrategy) -> &'static str {
    match strategy {
        ApprovalStrategy::Always => "always",
        ApprovalStrategy::Never => "never",
        ApprovalStrategy::PerTool => "per-tool",
    }
}

fn render_python_context(config: &EmbeddedPythonConfig) -> String {
    let mut s = String::new();
    s.push_str("\nEmbedded Python (python_exec):\n");
    s.push_str("Status: enabled\n");
    s.push_str("Preferred uses: deterministic computation, tool orchestration, safe parallelization of independent tasks.\n");
    s.push_str("Tool access: the script receives `tools`, a namespace derived from the visible runtime tool catalog. Prefer `await tools.<tool>(payload)` for Python-safe aliases, use `await tools.call(name, payload)` for raw-name fallback, and keep `iron_call(name, args)` only as a low-level escape hatch.\n");
    s.push_str("Restrictions:\n");
    s.push_str("- No package installation (pip is unavailable)\n");
    s.push_str("- Do not assume arbitrary third-party libraries are available\n");
    s.push_str("- Do not rely on classes unless explicitly supported\n");
    s.push_str("- Do not assume full CPython compatibility\n");
    s.push_str(&format!(
        "Limits: timeout {}s, source {} bytes, result {} bytes, child calls {}\n",
        config.max_script_timeout_secs,
        config.max_source_bytes,
        config.max_result_bytes,
        config.max_child_calls,
    ));
    s
}

fn chrono_date_or_fallback() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => {
            let days_since_epoch = d.as_secs() / 86400;
            let year = 1970 + (days_since_epoch / 365) as i32;
            format!("{} (approx)", year)
        }
        Err(_) => "unknown".to_string(),
    }
}
