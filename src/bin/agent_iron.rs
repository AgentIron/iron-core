//! `agent-iron` headless CLI binary.
//!
//! Run-only task execution: `agent-iron run <task-id> [OPTIONS]`.
//! See `agent-iron run --help` for usage.

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to create Tokio runtime");

    let local = tokio::task::LocalSet::new();
    let code = local.block_on(&runtime, async { iron_core::cli::execute_run(&args).await });

    std::process::ExitCode::from(code as u8)
}
