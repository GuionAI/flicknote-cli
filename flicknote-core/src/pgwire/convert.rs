//! Conversion from pgwire row types to domain types.

use super::row::{KeytermPgRow, NotePgRow, ProjectPgRow, PromptPgRow};
use crate::types::{Keyterm, Note, Project, Prompt};

impl From<NotePgRow> for Note {
    fn from(r: NotePgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            r#type: r.r#type,
            status: r.status,
            title: r.title,
            content: r.content,
            summary: r.summary,
            is_flagged: r.is_flagged.map(|b| if b { 1 } else { 0 }),
            project_id: r.project_id.map(|u| u.to_string()),
            metadata: r.metadata.map(|v| v.to_string()),
            source: r.source.map(|v| v.to_string()),
            external_id: r.external_id.map(|v| v.to_string()),
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
            deleted_at: r.deleted_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<ProjectPgRow> for Project {
    fn from(r: ProjectPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            name: r.name,
            color: r.color,
            prompt_id: r.prompt_id.map(|u| u.to_string()),
            keyterm_id: r.keyterm_id.map(|u| u.to_string()),
            is_archived: r.is_archived.map(|b| if b { 1 } else { 0 }),
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<PromptPgRow> for Prompt {
    fn from(r: PromptPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            title: r.title,
            description: r.description,
            prompt: r.prompt,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<KeytermPgRow> for Keyterm {
    fn from(r: KeytermPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            name: r.name,
            description: r.description,
            content: r.content,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
        }
    }
}
