//! Read the existing iOS-compatible content history format.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use flicknote_core::services::error::ServiceError;
use rmcp::schemars::JsonSchema;
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENCODED: usize = 360_710_144;

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

struct History(BTreeMap<i64, String>);
impl<'de> Deserialize<'de> for History {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HistoryVisitor;
        impl<'de> Visitor<'de> for HistoryVisitor {
            type Value = History;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of millisecond timestamps and text bodies")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<History, M::Error> {
                let mut entries = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    let digits = key.strip_prefix('-').unwrap_or(&key);
                    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
                        return Err(de::Error::custom("invalid history timestamp"));
                    }
                    let timestamp: i64 = key.parse().map_err(de::Error::custom)?;
                    if !DateTime::from_timestamp_millis(timestamp)
                        .is_some_and(|date| (0..=9999).contains(&date.year()))
                        || entries.insert(timestamp, value).is_some()
                    {
                        return Err(de::Error::custom("invalid or duplicate history timestamp"));
                    }
                }
                Ok(History(entries))
            }
        }
        deserializer.deserialize_map(HistoryVisitor)
    }
}

fn invalid_history() -> ServiceError {
    ServiceError::Remote {
        code: "invalid_history".into(),
        message: "Unable to decode note history or history exceeds the size limit".into(),
        retryable: false,
        details: None,
    }
}

fn decode(encoded: Option<String>) -> Result<History, ServiceError> {
    let Some(encoded) = encoded else {
        return Ok(History(BTreeMap::new()));
    };
    if encoded.len() > MAX_ENCODED {
        return Err(invalid_history());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid_history())?;
    // Require one ordinary known-size frame, never concatenated or skippable.
    if !bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
        || zstd::zstd_safe::find_frame_compressed_size(&bytes).ok() != Some(bytes.len())
    {
        return Err(invalid_history());
    }
    let descriptor = *bytes.get(4).ok_or_else(invalid_history)?;
    if descriptor & 0x18 != 0 {
        return Err(invalid_history());
    }
    if descriptor & 0x20 == 0 {
        let window = *bytes.get(5).ok_or_else(invalid_history)?;
        let base = 1u64 << (10 + (window >> 3));
        if base + (base / 8) * u64::from(window & 7) > MAX_BYTES as u64 {
            return Err(invalid_history());
        }
    }
    let size = zstd::zstd_safe::get_frame_content_size(&bytes)
        .ok()
        .flatten()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= MAX_BYTES)
        .ok_or_else(invalid_history)?;
    let mut decompressor = zstd::bulk::Decompressor::new().map_err(|_| invalid_history())?;
    decompressor
        .window_log_max(28)
        .map_err(|_| invalid_history())?;
    let json = decompressor
        .decompress(&bytes, size)
        .map_err(|_| invalid_history())?;
    if json.len() != size {
        return Err(invalid_history());
    }
    serde_json::from_slice(&json).map_err(|_| invalid_history())
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
        let encoded =
            self.db
                .find_note_history(&uuid, archived)
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
