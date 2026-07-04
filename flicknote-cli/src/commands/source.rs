use clap::{Args, Subcommand};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

const SOURCE_HELP: &str = include_str!("../help/source.md");

#[derive(Args)]
#[command(after_help = SOURCE_HELP)]
pub(crate) struct SourceArgs {
    #[command(subcommand)]
    command: SourceCommands,
}

#[derive(Subcommand)]
enum SourceCommands {
    /// Show the stored source for a note
    Show(ShowArgs),
}

#[derive(Args)]
#[command(after_help = SOURCE_HELP)]
struct ShowArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Optional 1-based range. Voice uses sentence indices; text sources use line numbers.
    range: Option<String>,
    /// Print raw source JSON instead of rendered text
    #[arg(long)]
    json: bool,
    /// Read an archived note
    #[arg(long)]
    archived: bool,
}

pub(crate) async fn run(db: &dyn NoteDb, args: &SourceArgs) -> Result<(), CliError> {
    match &args.command {
        SourceCommands::Show(args) => show(db, args).await,
    }
}

async fn show(db: &dyn NoteDb, args: &ShowArgs) -> Result<(), CliError> {
    let full_id = if args.archived {
        db.resolve_archived_note_id(&args.id).await?
    } else {
        db.resolve_note_id(&args.id).await?
    };
    let note = if args.archived {
        db.find_archived_note(&full_id).await?
    } else {
        db.find_note(&full_id).await?
    };
    let source = note
        .source
        .ok_or_else(|| CliError::Other("This note has no source data".into()))?;
    if args.json && args.range.is_some() {
        return Err(CliError::Other(
            "range cannot be used with --json source output".into(),
        ));
    }
    let output = if args.json {
        render_json_source(&source)?
    } else {
        render_source(&source, args.range.as_deref())?
    };
    print!("{output}");
    if !output.ends_with('\n') {
        println!();
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceRange {
    Single(usize),
    Closed(usize, usize),
    To(usize),
    From(usize),
}

fn parse_range(input: &str) -> Result<SourceRange, CliError> {
    if let Some((start, end)) = input.split_once(':') {
        return match (start, end) {
            ("", "") => Err(invalid_range(input)),
            ("", end) => Ok(SourceRange::To(parse_range_index(input, end)?)),
            (start, "") => Ok(SourceRange::From(parse_range_index(input, start)?)),
            (start, end) => {
                let start = parse_range_index(input, start)?;
                let end = parse_range_index(input, end)?;
                if start > end {
                    return Err(invalid_range(input));
                }
                Ok(SourceRange::Closed(start, end))
            }
        };
    }

    Ok(SourceRange::Single(parse_range_index(input, input)?))
}

fn parse_range_index(range: &str, input: &str) -> Result<usize, CliError> {
    let value = input.parse::<usize>().map_err(|_| invalid_range(range))?;
    if value == 0 {
        return Err(invalid_range(range));
    }
    Ok(value)
}

fn invalid_range(range: &str) -> CliError {
    CliError::Other(format!(
        "invalid source range {range:?}; use N, N:M, N:, or :M with 1-based indices"
    ))
}

fn render_source(source: &str, range: Option<&str>) -> Result<String, CliError> {
    let value: serde_json::Value = match serde_json::from_str(source) {
        Ok(value) => value,
        Err(_) => return render_text_source(source, range),
    };

    if let Some(content) = text_content(&value) {
        return render_text_source(content, range);
    }

    if let Some(transcripts) = value
        .get("voice")
        .and_then(|voice| voice.get("transcripts"))
        .and_then(serde_json::Value::as_array)
    {
        return render_voice_source(transcripts, range);
    }

    if range.is_some() {
        return Err(CliError::Other(
            "source range is only supported for voice transcripts and text content".into(),
        ));
    }

    render_json_value(&value)
}

fn render_json_source(source: &str) -> Result<String, CliError> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(source) else {
        return Ok(format!("{source}\n"));
    };
    render_json_value(&value)
}

fn render_json_value(value: &serde_json::Value) -> Result<String, CliError> {
    let mut output = serde_json::to_string_pretty(value).map_err(CliError::Json)?;
    output.push('\n');
    Ok(output)
}

fn text_content(value: &serde_json::Value) -> Option<&str> {
    ["link", "scan", "file", "flash"]
        .into_iter()
        .find_map(|kind| value.get(kind)?.get("content")?.as_str())
}

fn render_text_source(source: &str, range: Option<&str>) -> Result<String, CliError> {
    if range.is_none() {
        return Ok(source.to_string());
    }

    let lines = source.lines().collect::<Vec<_>>();
    let (start, end) = resolve_range(range, lines.len())?;
    let mut output = lines[start..end].join("\n");
    output.push('\n');
    Ok(output)
}

fn render_voice_source(
    transcripts: &[serde_json::Value],
    range: Option<&str>,
) -> Result<String, CliError> {
    let (start, end) = resolve_range(range, transcripts.len())?;
    let mut output = String::new();

    for (offset, tuple) in transcripts[start..end].iter().enumerate() {
        let Some(items) = tuple.as_array() else {
            continue;
        };
        let start_ms = items
            .first()
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let end_ms = items
            .get(1)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let text = items
            .get(2)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let index = start + offset + 1;
        output.push_str(&format!(
            "{index}  {}-{}  {text}\n",
            format_ms(start_ms),
            format_ms(end_ms)
        ));
    }

    Ok(output)
}

fn resolve_range(range: Option<&str>, len: usize) -> Result<(usize, usize), CliError> {
    if len == 0 {
        if let Some(range) = range {
            return Err(CliError::Other(format!(
                "source range {range:?} is outside available range; source is empty"
            )));
        }
        return Ok((0, 0));
    }

    let Some(range) = range else {
        return Ok((0, len));
    };

    let (start, end) = match parse_range(range)? {
        SourceRange::Single(index) => (index, index),
        SourceRange::Closed(start, end) => (start, end),
        SourceRange::To(end) => (1, end),
        SourceRange::From(start) => (start, len),
    };

    if start == 0 || start > len || end > len {
        return Err(CliError::Other(format!(
            "source range {range:?} is outside available 1..{len}"
        )));
    }

    Ok((start - 1, end))
}

fn format_ms(ms: i64) -> String {
    let ms = ms.max(0);
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let millis = ms % 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_accepts_single_and_open_ranges() {
        assert_eq!(parse_range("12").unwrap(), SourceRange::Single(12));
        assert_eq!(parse_range("12:19").unwrap(), SourceRange::Closed(12, 19));
        assert_eq!(parse_range(":19").unwrap(), SourceRange::To(19));
        assert_eq!(parse_range("12:").unwrap(), SourceRange::From(12));
    }

    #[test]
    fn render_source_slices_text_sources_by_line_number() {
        let source = r#"{"link":{"content":"one\ntwo\nthree\nfour"}}"#;

        assert_eq!(render_source(source, Some("2:3")).unwrap(), "two\nthree\n");
    }

    #[test]
    fn render_source_preserves_text_content_without_range() {
        let source = r#"{"link":{"content":"one"}}"#;

        assert_eq!(render_source(source, None).unwrap(), "one");
    }

    #[test]
    fn render_source_rejects_range_for_empty_text_content() {
        let source = r#"{"link":{"content":""}}"#;

        let err = render_source(source, Some("1")).unwrap_err();
        assert!(err.to_string().contains("outside available range"));
    }

    #[test]
    fn render_source_slices_voice_sources_by_sentence_number() {
        let source = r#"{"voice":{"transcripts":[[0,1000,"first",99],[1000,2000,"second",98],[2000,3000,"third",97]]}}"#;

        assert_eq!(
            render_source(source, Some("2")).unwrap(),
            "2  00:01.000-00:02.000  second\n"
        );
    }

    #[test]
    fn render_source_rejects_range_for_empty_voice_transcripts() {
        let source = r#"{"voice":{"transcripts":[]}}"#;

        let err = render_source(source, Some("1")).unwrap_err();
        assert!(err.to_string().contains("outside available range"));
    }

    #[test]
    fn render_source_pretty_prints_unknown_json_without_range() {
        let source = r#"{"other":{"value":1}}"#;

        assert_eq!(
            render_source(source, None).unwrap(),
            "{\n  \"other\": {\n    \"value\": 1\n  }\n}\n"
        );
    }
}
