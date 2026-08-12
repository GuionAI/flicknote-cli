use std::borrow::Cow;

use flicknote_core::services::dto::{
    ExtractionDto, NoteArchiveResult, NoteDetail, NoteMutationResult, NoteSummary, ProjectDto,
    SectionDto,
};
use flicknote_core::services::source::SourceResult;
use rmcp::schemars::{JsonSchema, Schema, SchemaGenerator};
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

/// Object-wrapped note list.
///
/// MCP 2025-era clients require `structuredContent` to be a JSON object
/// (record), rejecting bare arrays. Wrapping the list keeps the result
/// spec-compliant while preserving structured data.
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteListResult {
    pub notes: Vec<McpNoteSummary>,
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
    #[schemars(schema_with = "arbitrary_json_schema")]
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

/// Object-form JSON Schema term for arbitrary JSON values.
///
/// `serde_json::Value` derives a bare boolean schema term (`true`) that strict
/// MCP clients reject. This term keeps arbitrary values unconstrained while
/// remaining a parseable JSON Schema object. `null` is included because
/// `Option<serde_json::Value>` fields serialize as `null` when absent.
fn arbitrary_json_schema(_generator: &mut SchemaGenerator) -> Schema {
    serde_json::from_value(serde_json::json!({
        "type": ["array", "boolean", "integer", "null", "number", "object", "string"],
    }))
    .expect("arbitrary-json schema is valid JSON Schema")
}

/// MCP boundary result for source queries.
///
/// Serializes identically to `SourceResult` (a `view`-tagged union) while
/// advertising an object-rooted JSON Schema whose variants describe each
/// source view and use object-form terms for the arbitrary raw value, both of
/// which strict MCP clients require.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(super) struct McpSourceResult(pub(super) SourceResult);

impl From<SourceResult> for McpSourceResult {
    fn from(result: SourceResult) -> Self {
        Self(result)
    }
}

impl JsonSchema for McpSourceResult {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("McpSourceResult")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        serde_json::from_value(serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "view": {"type": "string", "enum": ["rendered"]},
                        "source_type": {"type": "string"},
                        "range_unit": {"type": "string"},
                        "total_count": {"type": "integer", "format": "uint", "minimum": 0},
                        "selected_start": {"type": "integer", "format": "uint", "minimum": 0},
                        "selected_end": {"type": "integer", "format": "uint", "minimum": 0},
                        "content": {"type": "string"}
                    },
                    "required": ["view", "source_type", "range_unit", "total_count", "selected_start", "selected_end", "content"]
                },
                {
                    "type": "object",
                    "properties": {
                        "view": {"type": "string", "enum": ["raw"]},
                        "source_type": {"type": "string"},
                        "value": {"type": ["array", "boolean", "integer", "null", "number", "object", "string"]}
                    },
                    "required": ["view", "source_type", "value"]
                },
                {
                    "type": "object",
                    "properties": {
                        "view": {"type": "string", "enum": ["info"]},
                        "source_type": {"type": "string"},
                        "range_unit": {"type": "string"},
                        "count": {"type": "integer", "format": "uint", "minimum": 0}
                    },
                    "required": ["view", "source_type", "range_unit", "count"]
                }
            ]
        }))
        .expect("source-result schema is valid JSON Schema")
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

/// Object-wrapped project list; see `McpNoteListResult` for why.
#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpProjectListResult {
    pub projects: Vec<McpProjectDto>,
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
