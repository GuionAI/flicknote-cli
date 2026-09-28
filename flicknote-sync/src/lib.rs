pub mod app;
pub mod browser;
mod connector;
pub mod fts_search;
pub mod ipc;
pub mod mcp;
mod ownership;
mod project_assignment_events;
pub mod recall;
mod remote;
mod runtime;
mod storage_maintenance;
mod upload;

pub use runtime::{DaemonRunError, run};

#[cfg(test)]
mod test_support;
