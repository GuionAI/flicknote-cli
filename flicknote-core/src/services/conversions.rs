//! Storage and Markdown projections into canonical client DTOs.

use flicknote_client::dto::{NoteRecord, SectionDto};

impl From<crate::types::Note> for NoteRecord {
    fn from(note: crate::types::Note) -> Self {
        Self {
            id: note.id,
            short_id: note.short_id,
            note_type: note.r#type,
            title: note.title,
            content: note.content,
            summary: note.summary,
            is_flagged: note.is_flagged,
            draft: note.status == "draft",
            project_id: note.project_id,
            created_at: note.created_at,
            updated_at: note.updated_at,
            deleted_at: note.deleted_at,
        }
    }
}

impl From<super::markdown::HeadingNode> for SectionDto {
    fn from(node: super::markdown::HeadingNode) -> Self {
        Self {
            id: node.heading.id,
            level: node.heading.level,
            title: node.heading.text,
            children: node.children.into_iter().map(Self::from).collect(),
        }
    }
}
