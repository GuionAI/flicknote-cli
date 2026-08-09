pub mod backend;
pub mod config;
#[cfg(feature = "powersync")]
pub mod db;
pub mod error;
#[cfg(feature = "storage-pgwire")]
pub mod pgwire;
#[cfg(feature = "powersync")]
pub mod schema;
pub mod services;
pub mod session;
pub mod types;

pub const TOPIC_EXTRACTION_KEY: &str = "::topic";
pub const ENTITY_EXTRACTION_KEYS: &[&str] = &["::person", "::company", "::location", "::product"];
pub const REMOTE_COMMITTED_INSERT_METADATA: &str = r#"{"flicknote":"remote_committed_insert_v1"}"#;
