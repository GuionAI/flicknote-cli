use std::sync::Arc;

use flicknote_core::services::dto::{
    ExtractionDto, NoteArchiveResult, NoteDetail, NoteMutationResult, NoteSummary, ProjectDto,
    SectionDto,
};
use flicknote_core::services::source::SourceResult;
use rmcp::handler::server::tool::schema_for_output;
use rmcp::model::JsonObject;
use rmcp::schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum McpEntityType {
    Person,
    Company,
    Location,
    Product,
}

impl McpEntityType {
    pub(super) const fn extraction_key(self) -> &'static str {
        match self {
            Self::Person => "::person",
            Self::Company => "::company",
            Self::Location => "::location",
            Self::Product => "::product",
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpTopicListResult {
    pub topics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpEntity {
    pub value: String,
    #[serde(rename = "type")]
    pub entity_type: McpEntityType,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpEntityListResult {
    pub entities: Vec<McpEntity>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(super) struct McpNoteSummary {
    pub id: Option<i64>,
    #[serde(rename = "type")]
    pub note_type: String,
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

/// Every JSON value type plus `null` (for `Option` fields). Used wherever
/// `serde_json::Value` appears so the schema stays an object-form term.
const ARBITRARY_JSON_TYPES: [&str; 7] = [
    "array", "boolean", "integer", "null", "number", "object", "string",
];

/// Object-form JSON Schema term for arbitrary JSON values.
///
/// `serde_json::Value` derives a bare boolean schema term (`true`) that strict
/// MCP clients reject. This term keeps arbitrary values unconstrained while
/// remaining a parseable JSON Schema object.
fn arbitrary_json_schema(_generator: &mut SchemaGenerator) -> Schema {
    serde_json::from_value(serde_json::json!({ "type": ARBITRARY_JSON_TYPES }))
        .expect("arbitrary-json schema is valid JSON Schema")
}

/// MCP boundary result for source queries.
///
/// Mirrors `SourceResult`'s `view`-tagged variants so that the advertised
/// output schema and the serialized `structuredContent` evolve together. The
/// raw `value` keeps an object-form arbitrary-JSON term (its only schema
/// customization); the explicit object root is applied by
/// [`source_output_schema`].
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case")]
pub(super) enum McpSourceResult {
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
        #[schemars(schema_with = "arbitrary_json_schema")]
        value: serde_json::Value,
    },
    Info {
        source_type: String,
        range_unit: String,
        count: usize,
    },
}

impl From<SourceResult> for McpSourceResult {
    fn from(result: SourceResult) -> Self {
        match result {
            SourceResult::Rendered {
                source_type,
                range_unit,
                total_count,
                selected_start,
                selected_end,
                content,
            } => Self::Rendered {
                source_type,
                range_unit,
                total_count,
                selected_start,
                selected_end,
                content,
            },
            SourceResult::Raw { source_type, value } => Self::Raw { source_type, value },
            SourceResult::Info {
                source_type,
                range_unit,
                count,
            } => Self::Info {
                source_type,
                range_unit,
                count,
            },
        }
    }
}

/// Object-rooted output schema for `McpSourceResult`.
///
/// Derives the union from the boundary DTO (so schema and serialization stay
/// in sync) and applies the single local compatibility fix: internally tagged
/// enum unions derive without an explicit root `type`, which strict MCP
/// clients reject.
pub(super) fn source_output_schema() -> Arc<JsonObject> {
    let mut schema = (*schema_for_output::<McpSourceResult>()).clone();
    schema.insert(
        "type".to_string(),
        serde_json::Value::String("object".to_string()),
    );
    Arc::new(schema)
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
