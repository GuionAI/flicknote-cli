//! `flicknote replace` — overwrite a whole section.

use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteMutationResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, print_section_tree, try_read_stdin};

const REPLACE_HELP: &str = include_str!("../help/replace.md");

#[derive(Args)]
#[command(after_help = REPLACE_HELP)]
pub(crate) struct ReplaceArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Replace the named section (stdin must start with a heading)
    #[arg(short = 's', long = "section")]
    section: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ReplaceArgs) -> Result<(), CliError> {
    let Some(content) = try_read_stdin()? else {
        return Err(CliError::Other(
            "--section requires content from stdin".into(),
        ));
    };
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteReplaceSection {
            id: args.id.clone(),
            section: args.section.clone(),
            content,
        })
        .await?;
    println!(
        "Replaced section in note {}.\n",
        display_summary_id(&result.note)
    );
    print_section_tree(&result.sections);
    Ok(())
}

#[cfg(test)]
mod tests {
    use flicknote_core::services::markdown::{parse_markdown, replace_entire_section};
    use flicknote_core::services::sections::{content_starts_with_heading, find_section};

    #[test]
    fn test_replace_section_setext_atx() {
        assert!(content_starts_with_heading("My Section\n=========="));
        assert!(content_starts_with_heading("My Section\n----------"));
    }

    #[test]
    fn test_replace_section_preserves_frontmatter_outside_section_scope() {
        let content = "---\ncustom: keep\n---\n\n## Target\nold body\n\n## Other\nother body";
        let document = parse_markdown(content);
        let heading = document
            .headings
            .iter()
            .find(|heading| heading.text == "Target")
            .unwrap();
        let bounds = find_section(&document, &heading.id, "note-id").unwrap();
        let updated =
            replace_entire_section(content, bounds.start, bounds.end, "## Target\nnew body");
        assert!(updated.starts_with("---\ncustom: keep\n---"));
        assert!(updated.contains("## Target\nnew body"));
        assert!(updated.contains("## Other\nother body"));
    }
}
