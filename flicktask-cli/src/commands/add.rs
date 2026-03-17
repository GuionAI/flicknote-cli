use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica, Status, Tag, Uuid};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct AddArgs {
    /// Task description
    pub description: String,

    /// Parent task ID (8-char hex or full UUID)
    #[arg(long)]
    pub parent: Option<String>,

    /// Due date (YYYY-MM-DD)
    #[arg(long)]
    pub due: Option<String>,

    /// Priority (H, M, or L)
    #[arg(long)]
    pub priority: Option<String>,

    /// Tag to add (repeatable)
    #[arg(long = "tag", short = 't')]
    pub tags: Vec<String>,

    /// Project name
    #[arg(long)]
    pub project: Option<String>,

    /// Set a UDA value (key=value, repeatable)
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: AddArgs) -> Result<()> {
    let mut ops = Operations::new();

    let uuid = Uuid::new_v4();
    let mut task = replica
        .create_task(uuid, &mut ops)
        .await
        .context("Failed to create task")?;

    task.set_description(args.description, &mut ops)?;
    task.set_status(Status::Pending, &mut ops)?;

    let now = taskchampion::chrono::Utc::now();
    task.set_value("entry", Some(now.timestamp().to_string()), &mut ops)?;

    if let Some(parent_id) = args.parent {
        let parent_uuid = resolve_id(replica, &parent_id).await?;
        task.set_value("parent", Some(parent_uuid.to_string()), &mut ops)?;
    }

    if let Some(due_str) = args.due {
        let due = parse_date(&due_str)?;
        task.set_due(Some(due), &mut ops)?;
    }

    if let Some(priority) = args.priority {
        task.set_priority(priority, &mut ops)?;
    }

    for tag_str in args.tags {
        let tag: Tag = tag_str
            .parse()
            .with_context(|| format!("Invalid tag: {tag_str:?}"))?;
        task.add_tag(&tag, &mut ops)?;
    }

    if let Some(project) = args.project {
        task.set_value("project", Some(project), &mut ops)?;
    }

    for kv in args.set {
        let (key, value) = parse_kv(&kv)?;
        task.set_value(key, Some(value.to_string()), &mut ops)?;
    }

    // Run on-add hooks — may enrich task (e.g. add branch, project_path)
    let task_json = crate::tw_json::task_to_tw_json(&uuid.to_string(), &task);
    let final_json = crate::hooks::run_on_add(&task_json)?;
    super::apply_hook_fields(&final_json, &task_json, &mut task, &mut ops)?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit task")?;

    println!("{}", short_id(&uuid));

    Ok(())
}

pub fn parse_date(s: &str) -> Result<taskchampion::chrono::DateTime<taskchampion::chrono::Utc>> {
    use taskchampion::chrono::{NaiveDate, TimeZone, Utc};
    let naive = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .with_context(|| format!("Invalid date {s:?} — expected YYYY-MM-DD"))?;
    Ok(Utc.from_utc_datetime(&naive.and_hms_opt(0, 0, 0).expect("midnight always valid")))
}

pub fn parse_kv(kv: &str) -> Result<(&str, &str)> {
    let Some(pos) = kv.find('=') else {
        anyhow::bail!("Invalid --set format {kv:?} — expected key=value");
    };
    Ok((&kv[..pos], &kv[pos + 1..]))
}
