//! Note source parsing and rendering.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::error::ServiceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceView {
    #[default]
    Rendered,
    Raw,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum SourceResult {
    Rendered {
        source_type: String,
        range_unit: String,
        total_count: usize,
        selected_start: usize,
        selected_end: usize,
        content: String,
    },
    Raw {
        source_type: String,
        value: serde_json::Value,
    },
    Info {
        source_type: String,
        range_unit: String,
        count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceRange {
    Single(usize),
    Closed(usize, usize),
    To(usize),
    From(usize),
}

pub fn parse_source(
    source: &str,
    view: SourceView,
    range: Option<&str>,
) -> Result<SourceResult, ServiceError> {
    if view != SourceView::Rendered && range.is_some() {
        return Err(ServiceError::InvalidArgument(
            "source range can only be used with rendered view".to_string(),
        ));
    }

    let parsed = serde_json::from_str::<serde_json::Value>(source).ok();
    let (source_type, range_unit, count) = source_metadata(source, parsed.as_ref());

    match view {
        SourceView::Info => Ok(SourceResult::Info {
            source_type: source_type.to_string(),
            range_unit: range_unit.to_string(),
            count,
        }),
        SourceView::Raw => Ok(SourceResult::Raw {
            source_type: source_type.to_string(),
            value: parsed.unwrap_or_else(|| serde_json::Value::String(source.to_string())),
        }),
        SourceView::Rendered => {
            let (selected_start, selected_end, content) =
                render_source(source, parsed.as_ref(), range, count)?;
            Ok(SourceResult::Rendered {
                source_type: source_type.to_string(),
                range_unit: range_unit.to_string(),
                total_count: count,
                selected_start,
                selected_end,
                content,
            })
        }
    }
}

fn source_metadata<'a>(
    source: &'a str,
    value: Option<&'a serde_json::Value>,
) -> (&'static str, &'static str, usize) {
    let Some(value) = value else {
        return ("text", "line", source.lines().count());
    };
    if let Some((kind, content)) = text_source(value) {
        return (kind, "line", content.lines().count());
    }
    if let Some(transcripts) = meeting_transcripts(value) {
        return ("meeting", "sentence", transcripts.len());
    }
    ("json", "none", 0)
}

fn render_source(
    source: &str,
    value: Option<&serde_json::Value>,
    range: Option<&str>,
    count: usize,
) -> Result<(usize, usize, String), ServiceError> {
    let Some(value) = value else {
        return render_text_source(source, range);
    };
    if let Some((_, content)) = text_source(value) {
        return render_text_source(content, range);
    }
    if let Some(transcripts) = meeting_transcripts(value) {
        return render_meeting_source(transcripts, range);
    }
    if range.is_some() {
        return Err(ServiceError::InvalidArgument(
            "source range is only supported for meeting transcripts and text content".to_string(),
        ));
    }
    let mut content = serde_json::to_string_pretty(value)
        .map_err(|error| ServiceError::Internal(error.to_string()))?;
    content.push('\n');
    Ok((0, count, content))
}

fn text_source(value: &serde_json::Value) -> Option<(&'static str, &str)> {
    ["link", "scan", "file", "flash"]
        .into_iter()
        .find_map(|kind| Some((kind, value.get(kind)?.get("content")?.as_str()?)))
}

fn meeting_transcripts(value: &serde_json::Value) -> Option<&[serde_json::Value]> {
    value
        .get("meeting")?
        .get("transcripts")?
        .as_array()
        .map(Vec::as_slice)
}

fn render_text_source(
    source: &str,
    range: Option<&str>,
) -> Result<(usize, usize, String), ServiceError> {
    let lines = source.lines().collect::<Vec<_>>();
    let (start, end) = resolve_range(range, lines.len())?;
    if range.is_none() {
        return Ok((selection_start(start, end), end, source.to_string()));
    }
    let mut output = lines[start..end].join("\n");
    output.push('\n');
    Ok((selection_start(start, end), end, output))
}

fn render_meeting_source(
    transcripts: &[serde_json::Value],
    range: Option<&str>,
) -> Result<(usize, usize, String), ServiceError> {
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
    Ok((selection_start(start, end), end, output))
}

fn selection_start(start: usize, end: usize) -> usize {
    if end == 0 { 0 } else { start + 1 }
}

fn resolve_range(range: Option<&str>, len: usize) -> Result<(usize, usize), ServiceError> {
    if len == 0 {
        if let Some(range) = range {
            return Err(ServiceError::InvalidArgument(format!(
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
    if start > len || end > len {
        return Err(ServiceError::InvalidArgument(format!(
            "source range {range:?} is outside available 1..{len}"
        )));
    }
    Ok((start - 1, end))
}

fn parse_range(input: &str) -> Result<SourceRange, ServiceError> {
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

fn parse_range_index(range: &str, input: &str) -> Result<usize, ServiceError> {
    let value = input.parse::<usize>().map_err(|_| invalid_range(range))?;
    if value == 0 {
        return Err(invalid_range(range));
    }
    Ok(value)
}

fn invalid_range(range: &str) -> ServiceError {
    ServiceError::InvalidArgument(format!(
        "invalid source range {range:?}; use N, N:M, N:, or :M with 1-based indices"
    ))
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
    use super::{SourceResult, SourceView, parse_source};

    #[test]
    fn rendered_text_source_reports_range_metadata() {
        let source = r#"{"link":{"content":"one\ntwo\nthree\nfour"}}"#;

        let result = parse_source(source, SourceView::Rendered, Some("2:3")).unwrap();

        assert_eq!(
            result,
            SourceResult::Rendered {
                source_type: "link".to_string(),
                range_unit: "line".to_string(),
                total_count: 4,
                selected_start: 2,
                selected_end: 3,
                content: "two\nthree\n".to_string(),
            }
        );
    }

    #[test]
    fn info_describes_meeting_without_rendering_transcript() {
        let source = r#"{"meeting":{"transcripts":[[0,1000,"first",99],[1000,2000,"second",98]]}}"#;

        let result = parse_source(source, SourceView::Info, None).unwrap();

        assert_eq!(
            result,
            SourceResult::Info {
                source_type: "meeting".to_string(),
                range_unit: "sentence".to_string(),
                count: 2,
            }
        );
    }

    #[test]
    fn raw_unknown_json_is_lossless() {
        let source = r#"{"other":{"value":1}}"#;

        let result = parse_source(source, SourceView::Raw, None).unwrap();

        assert_eq!(
            result,
            SourceResult::Raw {
                source_type: "json".to_string(),
                value: serde_json::json!({"other":{"value":1}}),
            }
        );
    }

    #[test]
    fn rendered_text_without_range_preserves_content() {
        let source = r#"{"link":{"content":"one"}}"#;
        let result = parse_source(source, SourceView::Rendered, None).unwrap();
        let SourceResult::Rendered { content, .. } = result else {
            panic!("expected rendered source");
        };
        assert_eq!(content, "one");
    }

    #[test]
    fn rendered_empty_text_rejects_a_range() {
        let source = r#"{"link":{"content":""}}"#;
        let error = parse_source(source, SourceView::Rendered, Some("1")).unwrap_err();
        assert!(error.to_string().contains("outside available range"));
    }

    #[test]
    fn rendered_meeting_uses_one_based_sentence_indices() {
        let source = r#"{"meeting":{"transcripts":[[0,1000,"first",99],[1000,2000,"second",98],[2000,3000,"third",97]]}}"#;
        let result = parse_source(source, SourceView::Rendered, Some("2")).unwrap();
        let SourceResult::Rendered {
            selected_start,
            selected_end,
            content,
            ..
        } = result
        else {
            panic!("expected rendered source");
        };
        assert_eq!((selected_start, selected_end), (2, 2));
        assert_eq!(content, "2  00:01.000-00:02.000  second\n");
    }

    #[test]
    fn rendered_unknown_json_is_pretty_printed() {
        let source = r#"{"other":{"value":1}}"#;
        let result = parse_source(source, SourceView::Rendered, None).unwrap();
        let SourceResult::Rendered { content, .. } = result else {
            panic!("expected rendered source");
        };
        assert_eq!(content, "{\n  \"other\": {\n    \"value\": 1\n  }\n}\n");
    }
}
