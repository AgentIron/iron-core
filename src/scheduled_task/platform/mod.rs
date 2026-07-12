//! Platform-specific host scheduler adapters.
//!
//! Each adapter is gated by its target platform. The cron expression
//! compilation and content rendering logic is platform-independent so it
//! can be tested on any host.

pub mod crontab;
pub mod cron_adapter;
pub mod launchd;
pub mod task_scheduler;
