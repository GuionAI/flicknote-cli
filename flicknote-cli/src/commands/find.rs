use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{ExtractionFilterDto, NoteFindInput, NoteSummary};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{note_summaries_json, print_summaries_table, resolve_project_arg};

const FIND_HELP: &str = include_str!("../help/find.md");

#[derive(Args)]
#[command(after_help = FIND_HELP)]
pub(crate) struct FindArgs {
    /// Keywords to search (OR match across title, content, summary)
    #[arg(required = true)]
    keywords: Vec<String>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Search only archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug)]
struct ParsedSearch {
    keywords: Vec<String>,
    extractions: Vec<ExtractionFilterDto>,
}

fn parse_search_input(args: &[String]) -> Result<ParsedSearch, CliError> {
    let mut keywords = Vec::new();
    let mut extractions = Vec::new();
    for arg in args {
        if !arg.starts_with("::") {
            keywords.push(arg.clone());
            continue;
        }
        let parts = arg.split("::").skip(1).collect::<Vec<_>>();
        if parts.len() % 2 != 0 || parts.iter().any(|part| part.is_empty()) {
            return Err(CliError::Other(
                "structured find filters must use ::type::value pairs".into(),
            ));
        }
        for pair in parts.chunks(2) {
            extractions.push(ExtractionFilterDto {
                key: format!("::{}", pair[0]),
                value: pair[1].to_string(),
            });
        }
    }
    Ok(ParsedSearch {
        keywords,
        extractions,
    })
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &FindArgs) -> Result<(), CliError> {
    let project = resolve_project_arg(&args.project);
    if args.project.is_none()
        && let Some(name) = project.as_deref()
    {
        eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
    }
    let parsed = parse_search_input(&args.keywords)?;
    let notes: Vec<NoteSummary> = daemon
        .call(AppRequest::NoteFind(NoteFindInput {
            keywords: parsed.keywords,
            extractions: parsed.extractions,
            project,
            archived: args.archived,
            limit: args.limit,
        }))
        .await?;
    if args.json {
        let values = note_summaries_json(daemon, &notes, args.archived).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&values).map_err(CliError::Json)?
        );
    } else if notes.is_empty() {
        println!("No notes found matching: {}", args.keywords.join(", "));
    } else {
        print_summaries_table(&notes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_input_splits_plain_keywords_and_structured_filters() {
        let parsed = parse_search_input(&[
            "whisper".to_string(),
            "::topic::ASR::person::瓜子".to_string(),
        ])
        .unwrap();
        assert_eq!(parsed.keywords, vec!["whisper"]);
        assert_eq!(
            parsed.extractions,
            vec![
                ExtractionFilterDto {
                    key: "::topic".to_string(),
                    value: "ASR".to_string(),
                },
                ExtractionFilterDto {
                    key: "::person".to_string(),
                    value: "瓜子".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_search_input_accepts_structured_only_search() {
        let parsed = parse_search_input(&["::topic::AI::company::OpenAI".to_string()]).unwrap();
        assert!(parsed.keywords.is_empty());
        assert_eq!(parsed.extractions.len(), 2);
    }

    #[test]
    fn parse_search_input_rejects_incomplete_structured_filter() {
        let error = parse_search_input(&["::topic::AI::person".to_string()]).unwrap_err();
        assert!(error.to_string().contains("structured find filters"));
    }
}
