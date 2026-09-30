#![doc = include_str!("../README.md")]
mod client;
pub mod dto;
pub mod editable_document;
mod error;
mod protocol;
pub mod source;

pub use client::{DaemonClient, send_request};
pub use error::ClientError;
pub use protocol::*;

#[cfg(test)]
mod tests;
