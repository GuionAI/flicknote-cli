//! Read the existing iOS-compatible content history format.
use chrono::{DateTime, SecondsFormat, Utc};
use flicknote_core::history;
use flicknote_core::services::error::ServiceError;
use rmcp::schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
struct History(BTreeMap<i64, String>);
fn decode(encoded: Option<String>) -> Result<History, ServiceError> {
    history::decode(encoded)
        .map(|h| History(h.0))
        .map_err(|_| invalid_history())
}
fn invalid_history() -> ServiceError {
    ServiceError::Remote {
        code: "invalid_history".into(),
        message: "Unable to decode note history or history exceeds the size limit".into(),
        retryable: false,
        details: None,
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct VersionInfo {
    #[schemars(range(min = 1))]
    pub version: usize,
    pub time: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct VersionList {
    pub versions: Vec<VersionInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct HistoricalNote {
    pub id: i64,
    #[schemars(range(min = 1))]
    pub version: usize,
    pub time: String,
    pub content: String,
}

fn time(millis: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .expect("timestamps validated")
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl History {
    fn list(self) -> VersionList {
        VersionList {
            versions: self
                .0
                .into_keys()
                .enumerate()
                .rev()
                .map(|(i, timestamp)| VersionInfo {
                    version: i + 1,
                    time: time(timestamp),
                })
                .collect(),
        }
    }
    #[allow(clippy::result_large_err)] // Preserve the shared service error contract on macOS.
    fn get(self, id: i64, version: usize) -> Result<HistoricalNote, ServiceError> {
        if version == 0 {
            return Err(ServiceError::InvalidArgument(
                "version must be a positive integer".into(),
            ));
        }
        let (timestamp, content) =
            self.0
                .into_iter()
                .nth(version - 1)
                .ok_or_else(|| ServiceError::Remote {
                    code: "version_not_found".into(),
                    message: format!("History version {version} not found"),
                    retryable: false,
                    details: None,
                })?;
        Ok(HistoricalNote {
            id,
            version,
            time: time(timestamp),
            content,
        })
    }
}

impl crate::app::Application {
    async fn history(&self, id: i64, archived: bool) -> Result<History, ServiceError> {
        let input = id.to_string();
        let uuid = if archived {
            self.db.resolve_archived_note_id(&input).await
        } else {
            self.db.resolve_note_id(&input).await
        }
        .map_err(ServiceError::from)?;
        self.history_by_uuid(&uuid, archived).await
    }
    async fn history_by_uuid(&self, uuid: &str, archived: bool) -> Result<History, ServiceError> {
        let encoded =
            self.db
                .find_note_history(uuid, archived)
                .await
                .map_err(|error| match error {
                    flicknote_core::error::CliError::Other(ref message)
                        if message == "History exceeds the size limit" =>
                    {
                        invalid_history()
                    }
                    other => ServiceError::from(other),
                })?;
        decode(encoded)
    }
    pub(crate) async fn history_list(
        &self,
        id: i64,
        archived: bool,
    ) -> Result<VersionList, ServiceError> {
        Ok(self.history(id, archived).await?.list())
    }
    pub(crate) async fn history_get(
        &self,
        id: i64,
        archived: bool,
        version: usize,
    ) -> Result<HistoricalNote, ServiceError> {
        if version == 0 {
            return Err(ServiceError::InvalidArgument(
                "version must be a positive integer".into(),
            ));
        }
        self.history(id, archived).await?.get(id, version)
    }

    /// Read the current body and every saved body from one local snapshot.
    #[cfg(feature = "experimental-spike")]
    pub async fn desktop_history(
        &self,
        local: &flicknote_core::backend::LocalPowerSyncBackend,
        uuid: &str,
        id: i64,
        archived: bool,
    ) -> Result<Vec<DesktopVersion>, ServiceError> {
        use flicknote_core::backend::NoteDb as _;
        if local.user_id() != self.db.user_id() {
            return Err(ServiceError::InvalidArgument("Note owner changed".into()));
        }
        let snapshot = local
            .desktop_history_snapshot(uuid, id, archived)
            .await
            .map_err(|error| match error {
                flicknote_core::error::CliError::Other(ref message)
                    if message == "History exceeds the size limit" =>
                {
                    invalid_history()
                }
                other => ServiceError::from(other),
            })?;
        let history = decode(snapshot.history)?;
        let mut versions = vec![DesktopVersion {
            version: None,
            time: snapshot.updated_at,
            content: snapshot.content.unwrap_or_default(),
        }];
        versions.extend(history.0.into_iter().enumerate().rev().map(
            |(i, (timestamp, content))| DesktopVersion {
                version: Some(i + 1),
                time: Some(time(timestamp)),
                content,
            },
        ));
        Ok(versions)
    }
}

/// A current or saved body from the same note snapshot.
#[cfg(feature = "experimental-spike")]
pub struct DesktopVersion {
    pub version: Option<usize>,
    pub time: Option<String>,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    fn encoded(json: &[u8]) -> String {
        STANDARD.encode(zstd::bulk::compress(json, 3).unwrap())
    }
    #[test]
    fn ios_fixture_and_invalid_history_are_distinct() {
        let history = decode(Some(
            "KLUv/SAa0QAAeyIxNzkxODAwMDAwMDAwIjoi5q2j5paHIn0=".into(),
        ))
        .unwrap();
        assert_eq!(history.get(42, 1).unwrap().content, "正文");
        for value in [
            "bad!".into(),
            encoded(br#"[]"#),
            encoded(br#"{"x":"body"}"#),
            encoded(br#"{"1":5}"#),
            encoded(br#"{"1":"a","01":"b"}"#),
            encoded(br#"{"999999999999999999":"a"}"#),
            encoded(&[0xff]),
            STANDARD.encode([
                0x28, 0xb5, 0x2f, 0xfd, 0xa0, 0x01, 0x00, 0x00, 0x10, 0x01, 0x00, 0x00,
            ]),
        ] {
            assert_eq!(decode(Some(value)).err().unwrap().code(), "invalid_history");
        }
    }
}
