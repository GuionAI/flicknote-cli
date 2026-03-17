//! Taskwarrior-compatible JSON serialization for TaskMap.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};
use taskchampion::storage::TaskMap;

/// Timestamp fields formatted as taskwarrior date strings.
pub(crate) const TIMESTAMP_KEYS: &[&str] = &[
    "entry",
    "modified",
    "due",
    "wait",
    "start",
    "end",
    "scheduled",
];

/// Convert a TaskMap + UUID to taskwarrior-compatible JSON.
pub fn taskmap_to_tw_json(uuid: &str, taskmap: &TaskMap) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("uuid".to_string(), json!(uuid));

    let mut tags: Vec<String> = Vec::new();
    let mut annotations: Vec<Value> = Vec::new();

    for (key, value) in taskmap {
        if let Some(tag_name) = key.strip_prefix("tag_") {
            tags.push(tag_name.to_string());
            continue;
        }
        if let Some(epoch_str) = key.strip_prefix("annotation_") {
            let entry = format_ts(epoch_str).unwrap_or_else(|| epoch_str.to_string());
            annotations.push(json!({"entry": entry, "description": value}));
            continue;
        }
        if TIMESTAMP_KEYS.contains(&key.as_str()) {
            if let Some(formatted) = format_ts(value) {
                obj.insert(key.clone(), json!(formatted));
            }
            continue;
        }
        obj.insert(key.clone(), json!(value));
    }

    if !tags.is_empty() {
        tags.sort();
        obj.insert("tags".to_string(), json!(tags));
    }
    if !annotations.is_empty() {
        annotations.sort_by(|a, b| {
            a["entry"]
                .as_str()
                .unwrap_or("")
                .cmp(b["entry"].as_str().unwrap_or(""))
        });
        obj.insert("annotations".to_string(), json!(annotations));
    }

    Value::Object(obj)
}

/// Convert a `Task` to taskwarrior-compatible JSON.
///
/// Wraps `taskmap_to_tw_json` using the deprecated `get_taskmap()` in one place.
#[allow(deprecated)]
pub fn task_to_tw_json(uuid: &str, task: &taskchampion::Task) -> Value {
    taskmap_to_tw_json(uuid, task.get_taskmap())
}

/// Format a unix epoch string to taskwarrior's date format: `%Y%m%dT%H%M%SZ`.
pub fn format_ts(epoch_str: &str) -> Option<String> {
    let secs: i64 = epoch_str.parse().ok()?;
    let dt: DateTime<Utc> = Utc.timestamp_opt(secs, 0).single()?;
    Some(dt.format("%Y%m%dT%H%M%SZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_taskmap(pairs: &[(&str, &str)]) -> taskchampion::storage::TaskMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_timestamp_formatting() {
        // Unix epoch 1741608000 = 2025-03-10T12:00:00Z
        assert_eq!(
            format_ts("1741608000"),
            Some("20250310T120000Z".to_string())
        );
        assert_eq!(format_ts("not_a_number"), None);
        assert_eq!(format_ts(""), None);
    }

    #[test]
    fn test_taskmap_to_tw_json_basic() {
        let uuid = "abc12345-0000-0000-0000-000000000000";
        let taskmap = make_taskmap(&[
            ("status", "pending"),
            ("description", "Fix the bug"),
            ("entry", "1741608000"),
            ("project", "fn-cli"),
            ("priority", "H"),
        ]);
        let json = taskmap_to_tw_json(uuid, &taskmap);
        assert_eq!(json["uuid"], uuid);
        assert_eq!(json["status"], "pending");
        assert_eq!(json["description"], "Fix the bug");
        assert_eq!(json["entry"], "20250310T120000Z");
        assert_eq!(json["project"], "fn-cli");
        assert_eq!(json["priority"], "H");
    }

    #[test]
    fn test_taskmap_to_tw_json_tags() {
        let uuid = "abc12345-0000-0000-0000-000000000001";
        let taskmap = make_taskmap(&[
            ("status", "pending"),
            ("description", "Tagged task"),
            ("tag_bugfix", ""),
            ("tag_planned", ""),
        ]);
        let json = taskmap_to_tw_json(uuid, &taskmap);
        let tags: Vec<&str> = json["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(tags.contains(&"bugfix"));
        assert!(tags.contains(&"planned"));
    }

    #[test]
    fn test_taskmap_to_tw_json_annotations() {
        let uuid = "abc12345-0000-0000-0000-000000000002";
        let taskmap = make_taskmap(&[
            ("status", "pending"),
            ("description", "Annotated task"),
            ("annotation_1741608000", "See issue #42"),
        ]);
        let json = taskmap_to_tw_json(uuid, &taskmap);
        let anns = json["annotations"].as_array().unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0]["entry"], "20250310T120000Z");
        assert_eq!(anns[0]["description"], "See issue #42");
    }

    #[test]
    fn test_taskmap_to_tw_json_udas() {
        let uuid = "abc12345-0000-0000-0000-000000000003";
        let taskmap = make_taskmap(&[
            ("status", "pending"),
            ("description", "UDA task"),
            ("branch", "worker/fix-bug"),
            ("project_path", "/some/path"),
        ]);
        let json = taskmap_to_tw_json(uuid, &taskmap);
        assert_eq!(json["branch"], "worker/fix-bug");
        assert_eq!(json["project_path"], "/some/path");
    }
}
