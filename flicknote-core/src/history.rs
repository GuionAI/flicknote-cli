//! Shared bounded iOS-compatible note history codec.
use crate::error::CliError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Datelike};
use serde::{
    Deserialize,
    de::{self, MapAccess, Visitor},
};
use std::{collections::BTreeMap, fmt};

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENCODED: usize = 360_710_144;

pub struct History(pub BTreeMap<i64, String>);
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

fn invalid_history() -> CliError {
    CliError::InvalidHistory
}

pub fn decode(encoded: Option<String>) -> Result<History, CliError> {
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

/// Preserve the old complete body at the old update marker.
pub fn save(
    encoded: Option<String>,
    old_content: Option<&str>,
    old_updated_at: Option<&str>,
) -> Result<Option<String>, CliError> {
    let mut history = decode(encoded.clone())?;
    let Some(old_content) = old_content else {
        return Ok(encoded);
    };
    if old_content.len() > MAX_BYTES {
        return Err(invalid_history());
    }
    let parsed =
        DateTime::parse_from_rfc3339(old_updated_at.ok_or(CliError::InvalidHistoryTimestamp)?)
            .map_err(|_| CliError::InvalidHistoryTimestamp)?;
    let nanos = parsed.timestamp_subsec_nanos();
    let mut millis = parsed.timestamp() * 1000 + i64::from(nanos / 1_000_000);
    let remainder = nanos % 1_000_000;
    if remainder > 500_000 || (remainder == 500_000 && millis >= 0) {
        millis += 1;
    }
    while history.0.contains_key(&millis) {
        millis = millis
            .checked_add(1)
            .ok_or(CliError::InvalidHistoryTimestamp)?;
    }
    if !DateTime::from_timestamp_millis(millis)
        .is_some_and(|date| (0..=9999).contains(&date.year()))
    {
        return Err(CliError::InvalidHistoryTimestamp);
    }
    history.0.insert(millis, old_content.to_owned());
    let entries: BTreeMap<String, String> = history
        .0
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let json = serde_json::to_vec(&entries).map_err(|_| invalid_history())?;
    if json.len() > MAX_BYTES {
        return Err(invalid_history());
    }
    let bytes = zstd::bulk::compress(&json, 3).map_err(|_| invalid_history())?;
    let result = STANDARD.encode(bytes);
    if result.len() > MAX_ENCODED {
        return Err(invalid_history());
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_time_collisions_and_invalid_timestamps() {
        let first = save(None, Some(""), Some("2020-01-01T00:00:00.000500Z")).unwrap();
        let second = save(first, Some("previous"), Some("2020-01-01T00:00:00.001Z")).unwrap();
        let entries = decode(second).unwrap().0;
        assert_eq!(entries.get(&1_577_836_800_001).unwrap(), "");
        assert_eq!(entries.get(&1_577_836_800_002).unwrap(), "previous");
        let negative = save(
            None,
            Some("before epoch"),
            Some("1969-12-31T23:59:59.999500Z"),
        )
        .unwrap();
        assert_eq!(
            decode(negative).unwrap().0.get(&-1).unwrap(),
            "before epoch"
        );
        assert!(matches!(
            save(None, Some("body"), Some("invalid")),
            Err(CliError::InvalidHistoryTimestamp)
        ));
        assert!(matches!(
            save(None, Some("body"), None),
            Err(CliError::InvalidHistoryTimestamp)
        ));
        let at_end = save(None, Some("a"), Some("9999-12-31T23:59:59.999Z")).unwrap();
        assert!(matches!(
            save(at_end, Some("b"), Some("9999-12-31T23:59:59.999Z")),
            Err(CliError::InvalidHistoryTimestamp)
        ));
        assert!(matches!(
            save(Some("bad!".into()), None, None),
            Err(CliError::InvalidHistory)
        ));
    }
}
