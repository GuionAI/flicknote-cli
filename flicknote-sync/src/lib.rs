use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{
    REMOTE_COMMITTED_INSERT_METADATA, TOPIC_EXTRACTION_KEY,
    backend::{NoteDb, SqliteBackend},
    config::Config,
    db::Database,
    schema::app_schema,
    services::ports::{CreateNote, NoteCreator, ShareGateway, ShareResource as CoreShareResource},
};
use futures_lite::StreamExt;
use powersync::{
    BackendConnector, ConnectionPool, PowerSyncCredentials, PowerSyncDatabase, SyncOptions,
    UpdateType, env::PowerSyncEnvironment, error::PowerSyncError,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use tokio::{net::UnixListener, sync::mpsc};

pub mod app;
mod connector;
pub mod ipc;
mod remote;
mod runtime;
mod storage_maintenance;
mod upload;

use app::Application;
use ipc::DaemonError;
pub(crate) use remote::attachment::*;
pub(crate) use remote::create::*;
pub(crate) use remote::share::*;
pub use runtime::run;
pub(crate) use storage_maintenance::*;
pub(crate) use upload::*;

#[cfg(test)]
mod test_support;
