use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::io::{self, IsTerminal};
use taskchampion::{Annotation, Operations, PowerSyncStorage, Replica, Status, Tag, Task, Uuid};

#[derive(Args)]
pub struct ImportArgs {}

const TW_TIMESTAMP_FMT: &str = "%Y%m%dT%H%M%SZ";

/// Fields handled explicitly — skip them in the UDA pass
const HANDLED_FIELDS: &[&str] = &[
    "description",
    "priority",
    "project",
    "status",
    "entry",
    "modified",
    "end",
    "start",
    "scheduled",
    "due",
    "wait",
    "tags",
    "annotations",
    "depends",
    "parent",
    // skip-only fields
    "id",
    "urgency",
    "project_path",
    "uuid",
];

/// Timestamp fields converted from TW format to Unix epoch before status is set.
/// "end" is excluded here and handled conditionally based on status.
const TIMESTAMP_FIELDS: &[&str] = &["entry", "modified", "start", "scheduled", "due", "wait"];

const BATCH_SIZE: usize = 50;

pub async fn run(replica: &mut Replica<PowerSyncStorage>, _args: ImportArgs) -> Result<()> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        anyhow::bail!("Pipe taskwarrior export: task export | flicktask import");
    }

    let tasks: Vec<serde_json::Value> = serde_json::from_reader(stdin)
        .context("Failed to parse JSON from stdin — expected taskwarrior export format")?;

    let total = tasks.len();

    let existing_uuids: HashSet<Uuid> = replica
        .all_task_uuids()
        .await
        .context("Failed to load existing task UUIDs")?
        .into_iter()
        .collect();

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut processed = 0usize;
    let mut ops = Operations::new();
    let mut batch_count = 0usize;

    for (i, task_json) in tasks.iter().enumerate() {
        processed += 1;

        let Some(obj) = task_json.as_object() else {
            eprintln!("Warning: skipping non-object task at index {i}");
            continue;
        };

        let Some(uuid_str) = obj.get("uuid").and_then(|v| v.as_str()) else {
            eprintln!("Warning: skipping task without uuid at index {i}");
            continue;
        };

        let uuid = match Uuid::parse_str(uuid_str) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("Warning: skipping task with invalid uuid {uuid_str:?}: {e}");
                continue;
            }
        };

        if existing_uuids.contains(&uuid) {
            skipped += 1;
            continue;
        }

        let mut task = replica
            .create_task(uuid, &mut ops)
            .await
            .with_context(|| format!("Failed to create task {uuid}"))?;

        import_task(uuid, obj, &mut task, &mut ops)?;

        imported += 1;
        batch_count += 1;

        if batch_count >= BATCH_SIZE {
            replica
                .commit_operations(ops)
                .await
                .inspect_err(|_| {
                    eprintln!("Import interrupted — {imported} tasks committed before failure")
                })
                .context("Failed to commit batch")?;
            ops = Operations::new();
            batch_count = 0;
            eprintln!("Importing... {processed}/{total}");
        }
    }

    if batch_count > 0 {
        replica
            .commit_operations(ops)
            .await
            .inspect_err(|_| {
                eprintln!("Import interrupted — {imported} tasks committed before failure")
            })
            .context("Failed to commit final batch")?;
    }

    println!("Imported: {imported} new tasks");
    println!("Skipped: {skipped} (already exist)");

    Ok(())
}

fn import_task(
    uuid: Uuid,
    obj: &serde_json::Map<String, serde_json::Value>,
    task: &mut Task,
    ops: &mut Operations,
) -> Result<()> {
    match obj.get("description").and_then(|v| v.as_str()) {
        Some(desc) => task.set_description(desc.to_string(), ops)?,
        None => eprintln!("Warning: task {uuid} has no description"),
    }

    if let Some(p) = obj.get("priority").and_then(|v| v.as_str()) {
        task.set_priority(p.to_string(), ops)?;
    }

    if let Some(proj) = obj.get("project").and_then(|v| v.as_str()) {
        task.set_value("project", Some(proj.to_string()), ops)?;
    }

    // Set non-end timestamps before status
    for key in TIMESTAMP_FIELDS {
        let Some(ts_str) = obj.get(*key).and_then(|v| v.as_str()) else {
            continue;
        };
        match parse_tw_timestamp(ts_str) {
            Ok(epoch) => task.set_value(*key, Some(epoch), ops)?,
            Err(e) => eprintln!("Warning: skipping {key} for task {uuid}: {e}"),
        }
    }

    let status_str = obj
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending");
    let status = match status_str {
        "pending" => Status::Pending,
        "completed" => Status::Completed,
        "deleted" => Status::Deleted,
        "recurring" => Status::Recurring,
        other => Status::Unknown(other.to_string()),
    };

    // Set "end" BEFORE status only for completed/deleted tasks.
    // set_status(Pending/Recurring) unconditionally clears "end", so setting it
    // first for those statuses would be overwritten anyway.
    if matches!(status, Status::Completed | Status::Deleted)
        && let Some(ts_str) = obj.get("end").and_then(|v| v.as_str())
    {
        match parse_tw_timestamp(ts_str) {
            Ok(epoch) => task.set_value("end", Some(epoch), ops)?,
            Err(e) => eprintln!("Warning: skipping end for task {uuid}: {e}"),
        }
    }

    task.set_status(status, ops)?;

    if let Some(tags_arr) = obj.get("tags").and_then(|v| v.as_array()) {
        for tag_val in tags_arr {
            let Some(tag_str) = tag_val.as_str() else {
                continue;
            };
            match tag_str.parse::<Tag>() {
                Ok(tag) => task.add_tag(&tag, ops)?,
                Err(e) => {
                    eprintln!("Warning: skipping invalid tag {tag_str:?} for task {uuid}: {e}")
                }
            }
        }
    }

    if let Some(annotations_arr) = obj.get("annotations").and_then(|v| v.as_array()) {
        for ann_val in annotations_arr {
            let Some(ann_obj) = ann_val.as_object() else {
                continue;
            };
            let entry_str = ann_obj.get("entry").and_then(|v| v.as_str());
            let desc = ann_obj.get("description").and_then(|v| v.as_str());
            let (Some(entry_str), Some(desc)) = (entry_str, desc) else {
                eprintln!(
                    "Warning: skipping malformed annotation for task {uuid}: missing entry/description"
                );
                continue;
            };
            match parse_tw_datetime(entry_str) {
                Ok(entry_dt) => task.add_annotation(
                    Annotation {
                        entry: entry_dt,
                        description: desc.to_string(),
                    },
                    ops,
                )?,
                Err(e) => eprintln!("Warning: skipping annotation for task {uuid}: {e}"),
            }
        }
    }

    if let Some(deps_arr) = obj.get("depends").and_then(|v| v.as_array()) {
        for dep_val in deps_arr {
            let Some(dep_str) = dep_val.as_str() else {
                continue;
            };
            match Uuid::parse_str(dep_str) {
                Ok(dep_uuid) => task.add_dependency(dep_uuid, ops)?,
                Err(e) => eprintln!(
                    "Warning: skipping invalid depends uuid {dep_str:?} for task {uuid}: {e}"
                ),
            }
        }
    }

    if let Some(parent_str) = obj.get("parent").and_then(|v| v.as_str()) {
        match Uuid::parse_str(parent_str) {
            Ok(_) => task.set_value("parent", Some(parent_str.to_string()), ops)?,
            Err(e) => eprintln!(
                "Warning: skipping invalid parent uuid {parent_str:?} for task {uuid}: {e}"
            ),
        }
    }

    // UDA fields — everything not handled above
    for (key, value) in obj {
        if HANDLED_FIELDS.contains(&key.as_str()) {
            continue;
        }
        let stringified = match value {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None, // skip null, objects, arrays
        };
        if let Some(val_str) = stringified {
            task.set_value(key, Some(val_str), ops)?;
        }
    }

    Ok(())
}

fn parse_tw_datetime(s: &str) -> Result<taskchampion::chrono::DateTime<taskchampion::chrono::Utc>> {
    use taskchampion::chrono::NaiveDateTime;
    NaiveDateTime::parse_from_str(s, TW_TIMESTAMP_FMT)
        .with_context(|| format!("Invalid TW timestamp {s:?}"))
        .map(|n| n.and_utc())
}

fn parse_tw_timestamp(s: &str) -> Result<String> {
    parse_tw_datetime(s).map(|dt| dt.timestamp().to_string())
}
