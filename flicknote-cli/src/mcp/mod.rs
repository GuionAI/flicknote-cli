mod dto;
mod error;
mod gateway_tools;
mod note_tools;
mod project_tools;
mod server;

pub(crate) use server::serve;
#[cfg(test)]
pub(crate) use server::{EXPECTED_TOOLS, FlickNoteMcp};
