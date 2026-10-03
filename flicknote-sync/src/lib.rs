pub mod app;
pub mod browser;
mod connector;
pub mod fts_search;
pub mod ipc;
pub mod mcp;
mod ownership;
pub mod pg;
pub mod private_mcp;
mod project_assignment_events;
pub mod recall;
mod remote;
mod runtime;
pub mod search;
mod storage_maintenance;
mod upload;

pub use runtime::{DaemonRunError, run};

#[cfg(test)]
mod test_support;

#[cfg(feature = "experimental-spike")]
pub mod spike;
