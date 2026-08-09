use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteDetail;
use flicknote_core::types::Note;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, note_json, print_section_tree};

const DETAIL_HELP: &str = include_str!("../help/detail.md");

#[derive(Args)]
#[command(after_help = DETAIL_HELP)]
pub(crate) struct DetailArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Show markdown heading structure
    #[arg(long)]
    tree: bool,
    /// Output as JSON
    #[arg(long)]
    json: bool,
    /// Read an archived note
    #[arg(long)]
    archived: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &DetailArgs) -> Result<(), CliError> {
    let detail: NoteDetail = daemon
        .call(AppRequest::NoteGet {
            id: args.id.clone(),
            archived: args.archived,
        })
        .await?;
    if args.tree {
        if detail.sections.is_empty() {
            println!("(no headings found)");
        } else {
            print_section_tree(&detail.sections);
        }
        return Ok(());
    }
    if args.json {
        let note: Note = daemon
            .call(AppRequest::NoteRecord {
                id: detail.note.uuid.clone(),
                archived: args.archived,
            })
            .await?;
        let value = note_json(&note, detail.note.project.as_deref());
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(CliError::Json)?
        );
        return Ok(());
    }

    println!("ID:         {}", display_summary_id(&detail.note));
    println!("Type:       {}", detail.note.note_type);
    println!(
        "Title:      {}",
        detail.note.title.as_deref().unwrap_or("(untitled)")
    );
    if let Some(summary) = detail.note.summary.as_deref() {
        println!("Summary:    {summary}");
    }
    if let Some(project) = detail.note.project.as_deref() {
        println!("Project:    {project}");
    }
    if detail.note.flagged {
        println!("Flagged:    yes");
    }
    println!(
        "Created:    {}",
        detail.note.created_at.as_deref().unwrap_or("-")
    );
    println!(
        "Updated:    {}",
        detail.note.updated_at.as_deref().unwrap_or("-")
    );
    if !detail.content.is_empty() {
        println!("\nContent:\n{}", detail.content);
    }
    if let Some(url) = detail
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("link"))
        .and_then(|link| link.get("url"))
        .and_then(serde_json::Value::as_str)
    {
        println!("Link:       {url}");
    }
    Ok(())
}
