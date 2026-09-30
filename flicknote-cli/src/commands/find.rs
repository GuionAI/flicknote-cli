use clap::Args;
use flicknote_client::dto::{ExtractionFilterDto, NoteFindInput, SearchHit};
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::error::CliError;

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
    /// Include notes created at or after this RFC3339 timestamp
    #[arg(long)]
    created_after: Option<String>,
    /// Include notes created before this RFC3339 timestamp
    #[arg(long)]
    created_before: Option<String>,
    /// Exclude notes created through MCP
    #[arg(long)]
    human: bool,
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

pub(crate) async fn run(daemon: &DaemonClient, args: &FindArgs) -> Result<(), CliError> {
    let project = args.project.clone();
    let parsed = parse_search_input(&args.keywords)?;
    let notes: Vec<SearchHit> = daemon
        .call(AppRequest::NoteFind(NoteFindInput {
            keywords: parsed.keywords,
            extractions: parsed.extractions,
            project,
            created_after: args.created_after.clone(),
            created_before: args.created_before.clone(),
            human: args.human,
            archived: args.archived,
            limit: args.limit,
        }))
        .await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else if notes.is_empty() {
        println!("No notes found matching: {}", args.keywords.join(", "));
    } else {
        print!("{}", format_search_hits(&notes));
    }
    Ok(())
}

fn format_search_hits(hits: &[SearchHit]) -> String {
    let mut output = String::from("ID       Title                          Snippet\n");
    for hit in hits {
        let id = hit
            .short_id
            .map_or_else(|| "-".to_owned(), |id| id.to_string());
        let title = hit.title.as_deref().unwrap_or("(untitled)");
        let excerpt = hit
            .snippet
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        output.push_str(&format!("{id:<8} {title:<30} {excerpt}\n"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_client::dto::{SearchSnippet, SnippetSegment};

    #[test]
    fn search_table_renders_segmented_snippet_as_readable_text() {
        let output = format_search_hits(&[SearchHit {
            short_id: Some(42),
            note_type: "normal".into(),
            content_bytes: 11,
            draft: false,
            title: Some("Office plan".into()),
            summary: None,
            created_at: None,
            updated_at: None,
            project_id: None,
            snippet: SearchSnippet {
                segments: vec![
                    SnippetSegment {
                        text: "The ".into(),
                        highlighted: false,
                    },
                    SnippetSegment {
                        text: "office".into(),
                        highlighted: true,
                    },
                    SnippetSegment {
                        text: " plan".into(),
                        highlighted: false,
                    },
                ],
            },
        }]);
        assert!(output.contains("42"));
        assert!(output.contains("Office plan"));
        assert!(output.contains("The office plan"));
        assert!(!output.contains('\u{1f}'));
    }

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
