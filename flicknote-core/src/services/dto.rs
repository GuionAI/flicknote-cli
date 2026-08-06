//! Shared application DTOs used by CLI and MCP adapters.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> Patch<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<'de, T> Deserialize<'de> for Patch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(|value| match value {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectModifyInput {
    pub id: String,
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    pub keyterm: Patch<String>,
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    pub color: Patch<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectAddInput {
    pub name: String,
    pub keyterm: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub keyterm_id: Option<String>,
    pub archived: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteSummary {
    #[serde(rename = "id")]
    pub short_id: Option<i64>,
    pub uuid: String,
    #[serde(rename = "type")]
    pub note_type: String,
    pub status: String,
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub project: Option<String>,
    pub topics: Vec<String>,
    pub summary: Option<String>,
    pub flagged: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SectionDto {
    pub id: String,
    pub level: usize,
    pub title: String,
    pub children: Vec<Self>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteMutationResult {
    pub note: NoteSummary,
    pub sections: Vec<SectionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteModifyInput {
    pub id: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub section: Option<String>,
    pub project: Option<String>,
    pub flagged: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteArchiveResult {
    pub short_id: Option<i64>,
    pub uuid: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteListInput {
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_note_limit")]
    pub limit: u32,
}

pub const fn default_note_limit() -> u32 {
    20
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionDto {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteDetail {
    #[serde(flatten)]
    pub note: NoteSummary,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub extractions: Vec<ExtractionDto>,
    pub sections: Vec<SectionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionFilterDto {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteFindInput {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub extractions: Vec<ExtractionFilterDto>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_note_limit")]
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteAddInput {
    pub content: String,
    pub project: Option<String>,
    #[serde(default)]
    pub interpret_as_url: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteCountInput {
    #[serde(default)]
    pub keywords: Vec<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsertPosition {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteSectionResult {
    pub id: String,
    pub level: usize,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ShareResult {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnshareResult {
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenResult {
    pub url: String,
    pub opened: bool,
}

#[cfg(test)]
mod tests {
    use super::{NoteSummary, Patch, ProjectModifyInput};

    #[test]
    fn project_patch_distinguishes_missing_null_and_value() {
        let missing: ProjectModifyInput =
            serde_json::from_value(serde_json::json!({ "id": "project-id" })).unwrap();
        assert_eq!(missing.keyterm, Patch::Missing);
        assert_eq!(missing.color, Patch::Missing);

        let clear: ProjectModifyInput = serde_json::from_value(serde_json::json!({
            "id": "project-id",
            "keyterm": null,
            "color": null
        }))
        .unwrap();
        assert_eq!(clear.keyterm, Patch::Null);
        assert_eq!(clear.color, Patch::Null);

        let set: ProjectModifyInput = serde_json::from_value(serde_json::json!({
            "id": "project-id",
            "keyterm": "keyterm-id",
            "color": "#336699"
        }))
        .unwrap();
        assert_eq!(set.keyterm, Patch::Value("keyterm-id".to_string()));
        assert_eq!(set.color, Patch::Value("#336699".to_string()));
    }

    #[test]
    fn note_summary_serializes_short_id_as_id() {
        let value = serde_json::to_value(NoteSummary {
            short_id: Some(42),
            uuid: "note-uuid".to_string(),
            note_type: "normal".to_string(),
            status: "synced".to_string(),
            title: None,
            project_id: None,
            project: None,
            topics: Vec::new(),
            summary: None,
            flagged: false,
            created_at: None,
            updated_at: None,
            deleted_at: None,
        })
        .unwrap();

        assert_eq!(value["id"], 42);
        assert!(value.get("short_id").is_none());
    }
}
