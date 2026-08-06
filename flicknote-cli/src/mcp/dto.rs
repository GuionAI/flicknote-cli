use flicknote_core::services::dto::{
    ExtractionDto, NoteArchiveResult, NoteDetail, NoteMutationResult, NoteSummary, ProjectDto,
    SectionDto,
};
use rmcp::schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteSummary {
    pub id: Option<i64>,
    #[serde(rename = "type")]
    pub note_type: String,
    pub status: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub topics: Vec<String>,
    pub summary: Option<String>,
    pub flagged: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl From<NoteSummary> for McpNoteSummary {
    fn from(note: NoteSummary) -> Self {
        Self {
            id: note.short_id,
            note_type: note.note_type,
            status: note.status,
            title: note.title,
            project: note.project,
            topics: note.topics,
            summary: note.summary,
            flagged: note.flagged,
            created_at: note.created_at,
            updated_at: note.updated_at,
            deleted_at: note.deleted_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteDetail {
    #[serde(flatten)]
    pub note: McpNoteSummary,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub extractions: Vec<ExtractionDto>,
    pub sections: Vec<SectionDto>,
}

impl From<NoteDetail> for McpNoteDetail {
    fn from(detail: NoteDetail) -> Self {
        Self {
            note: detail.note.into(),
            content: detail.content,
            metadata: detail.metadata,
            extractions: detail.extractions,
            sections: detail.sections,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteMutationResult {
    pub note: McpNoteSummary,
    pub sections: Vec<SectionDto>,
}

impl From<NoteMutationResult> for McpNoteMutationResult {
    fn from(result: NoteMutationResult) -> Self {
        Self {
            note: result.note.into(),
            sections: result.sections,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteArchiveResult {
    pub id: Option<i64>,
    pub archived: bool,
}

impl From<NoteArchiveResult> for McpNoteArchiveResult {
    fn from(result: NoteArchiveResult) -> Self {
        Self {
            id: result.short_id,
            archived: result.archived,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpProjectDto {
    pub name: String,
    pub color: Option<String>,
    pub archived: bool,
    pub created_at: Option<String>,
}

impl From<ProjectDto> for McpProjectDto {
    fn from(project: ProjectDto) -> Self {
        Self {
            name: project.name,
            color: project.color,
            archived: project.archived,
            created_at: project.created_at,
        }
    }
}
