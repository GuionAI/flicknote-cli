use super::error::ServiceError;
use super::markdown::{Document, Heading};

/// Byte-range boundaries of a matched section in a Markdown document.
pub struct SectionBounds<'a> {
    pub heading: &'a Heading,
    pub start: usize,
    pub end: usize,
}

/// Find a section by its stable short section ID.
pub fn find_section<'a>(
    doc: &'a Document,
    section_id: &str,
    note_ref: &str,
) -> Result<SectionBounds<'a>, ServiceError> {
    let heading_idx = doc
        .headings
        .iter()
        .position(|heading| heading.id == section_id)
        .ok_or_else(|| {
            ServiceError::SectionNotFound(format!(
                "unknown section ID {section_id:?} in note {note_ref} — run `flicknote detail {note_ref} --tree` to see current IDs"
            ))
        })?;

    let heading = &doc.headings[heading_idx];
    let start = heading.offset;
    let end = doc
        .headings
        .iter()
        .skip(heading_idx + 1)
        .find(|candidate| candidate.level <= heading.level)
        .map(|candidate| candidate.offset)
        .unwrap_or(doc.content.len());

    Ok(SectionBounds {
        heading,
        start,
        end,
    })
}

pub fn content_starts_with_heading(content: &str) -> bool {
    use pulldown_cmark::{Event, Options, Parser, Tag};

    Parser::new_ext(content, Options::empty())
        .next()
        .is_some_and(|event| matches!(event, Event::Start(Tag::Heading { .. })))
}
