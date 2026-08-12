use std::fmt;
use std::path::PathBuf;

use flicknote_core::config::Config;
use flicknote_core::services::dto::{
    InsertPosition, NoteAddInput, NoteArchiveResult, NoteCountInput, NoteDetail, NoteFindInput,
    NoteListInput, NoteModifyInput, NoteMutationResult, NoteRecord, NoteSectionResult, NoteSummary,
    OpenResult, ProjectAddInput, ProjectDto, ProjectModifyInput, ShareResult, UnshareResult,
};
use flicknote_core::services::editable_document::EditableSaveResult;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::source::{SourceResult, SourceView};
use flicknote_core::types::Project;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::net::UnixStream;

use crate::app::Application;

mod client;
mod protocol;
mod server;

#[cfg(test)]
pub(crate) use client::response_timeout_for;
pub use client::{DaemonClient, send_request, socket_path};
pub use protocol::*;
#[cfg(test)]
pub(crate) use server::write_json;
pub use server::{read_request, serve_app, serve_app_once, write_response};

#[cfg(test)]
mod tests;
