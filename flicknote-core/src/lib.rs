pub mod backend;
pub mod config;
pub mod error;
#[cfg(feature = "powersync")]
pub mod schema;
pub mod services;
pub mod session;
#[allow(unsafe_code)] // SQLite's process-wide extension registration requires FFI.
pub mod sqlite_extension;
pub mod types;

pub const TOPIC_EXTRACTION_KEY: &str = "::topic";
pub const ENTITY_EXTRACTION_KEYS: &[&str] = &["::person", "::company", "::location", "::product"];
pub const REMOTE_COMMITTED_INSERT_METADATA: &str = r#"{"flicknote":"remote_committed_insert_v1"}"#;

pub mod profile;
