use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::source::{SourceResult, SourceView};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const SOURCE_HELP: &str = include_str!("../help/source.md");

#[derive(Args)]
#[command(after_help = SOURCE_HELP)]
pub(crate) struct SourceArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Optional 1-based range. Meeting uses sentence indices; text sources use line numbers.
    range: Option<String>,
    /// Print raw source JSON instead of rendered text
    #[arg(long)]
    json: bool,
    /// Print source type, range unit, and count
    #[arg(long)]
    info: bool,
    /// Read an archived note
    #[arg(long)]
    archived: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &SourceArgs) -> Result<(), CliError> {
    if args.info && args.json {
        return Err(CliError::Other(
            "--info cannot be used with --json source output".into(),
        ));
    }
    let view = if args.info {
        SourceView::Info
    } else if args.json {
        SourceView::Raw
    } else {
        SourceView::Rendered
    };
    let result: SourceResult = daemon
        .call(AppRequest::NoteSource {
            id: args.id.clone(),
            archived: args.archived,
            view,
            range: args.range.clone(),
        })
        .await?;
    let output = match result {
        SourceResult::Rendered { content, .. } => content,
        SourceResult::Info {
            source_type,
            range_unit,
            count,
        } => format!("type: {source_type}\nrange_unit: {range_unit}\ncount: {count}\n"),
        SourceResult::Raw { value, .. } => match value {
            serde_json::Value::String(text) => text,
            value => serde_json::to_string_pretty(&value).map_err(CliError::Json)?,
        },
    };
    print!("{output}");
    if !output.ends_with('\n') {
        println!();
    }
    Ok(())
}
