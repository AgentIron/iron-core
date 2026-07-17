//! Platform-specific host scheduler adapters.
//!
//! Each adapter is gated by its target platform. The cron expression
//! compilation and content rendering logic is platform-independent so it
//! can be tested on any host.

#[cfg(target_os = "linux")]
pub mod cron_adapter;
#[cfg(target_os = "linux")]
pub mod crontab;
#[cfg(target_os = "macos")]
pub mod launchd;
#[cfg(target_os = "windows")]
pub mod task_scheduler;
