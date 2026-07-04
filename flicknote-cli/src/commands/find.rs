use clap::Args;
use flicknote_core::backend::{MetadataFilter, NoteDb, NoteFilter, NoteSearch};
use flicknote_core::error::CliError;

use super::util::{note_json, print_notes_table, resolve_project_arg};

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
    /// Include archived notes
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
    extractions: Vec<MetadataFilter>,
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
            extractions.push(MetadataFilter {
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

pub(crate) async fn run(db: &dyn NoteDb, args: &FindArgs) -> Result<(), CliError> {
    let effective_project = resolve_project_arg(&args.project);
    let parsed = parse_search_input(&args.keywords)?;

    let project_id: Option<String> = if let Some(ref name) = effective_project {
        if args.project.is_none() {
            eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
        }
        match db.find_project_by_name(name).await? {
            Some(id) => Some(id),
            None => {
                return Err(CliError::Other(format!(
                    "no project found with name \"{name}\""
                )));
            }
        }
    } else {
        None
    };

    let notes = db
        .search_notes_structured(
            &NoteSearch {
                keywords: parsed.keywords.clone(),
                extractions: parsed.extractions,
            },
            &NoteFilter {
                project_id: project_id.as_deref(),
                note_type: None,
                archived: args.archived,
                limit: args.limit,
            },
        )
        .await?;

    if args.json {
        let values: Vec<_> = notes.iter().map(|note| note_json(note, None)).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&values).map_err(CliError::Json)?
        );
    } else if notes.is_empty() {
        println!("No notes found matching: {}", args.keywords.join(", "));
    } else {
        let note_id_refs: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        let topics_map = db.list_note_topics(&note_id_refs).await?;
        let mut project_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for note in &notes {
            let Some(ref pid) = note.project_id else {
                continue;
            };
            if project_names.contains_key(pid) {
                continue;
            }
            if let Some(name) = db.find_project_name_by_id(pid).await? {
                project_names.insert(pid.clone(), name);
            }
        }
        print_notes_table(&notes, &topics_map, &project_names);
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
                MetadataFilter {
                    key: "::topic".to_string(),
                    value: "ASR".to_string(),
                },
                MetadataFilter {
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
        assert_eq!(
            parsed.extractions,
            vec![
                MetadataFilter {
                    key: "::topic".to_string(),
                    value: "AI".to_string(),
                },
                MetadataFilter {
                    key: "::company".to_string(),
                    value: "OpenAI".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_search_input_rejects_incomplete_structured_filter() {
        let err = parse_search_input(&["::topic::AI::person".to_string()]).unwrap_err();

        assert!(err.to_string().contains("structured find filters"));
    }
}
