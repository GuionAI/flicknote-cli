use flicknote_core::services::dto::ExtractionFilterDto;
use flicknote_core::services::source::SourceView;
use rmcp::schemars::JsonSchema;
use serde::Deserialize;

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "NoteType")]
pub(super) enum ListNoteType {
    Normal,
    Meeting,
    Link,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename = "NoteType")]
pub(super) enum CountNoteType {
    Normal,
    Meeting,
    Link,
    File,
}

impl ListNoteType {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Meeting => "meeting",
            Self::Link => "link",
        }
    }
}

impl CountNoteType {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Meeting => "meeting",
            Self::Link => "link",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteListParams {
    #[serde(rename = "type")]
    pub note_type: Option<ListNoteType>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteFindParams {
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub extractions: Vec<ExtractionFilterDto>,
    pub project: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteCountParams {
    #[serde(default)]
    pub keywords: Vec<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub note_type: Option<CountNoteType>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteIdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteGetParams {
    pub id: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSectionParams {
    pub id: String,
    pub section: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteAddParams {
    pub content: String,
    pub project: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteModifyParams {
    pub id: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub section: Option<String>,
    pub project: Option<String>,
    pub flagged: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteContentParams {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteInsertParams {
    pub id: String,
    pub section: String,
    pub position: flicknote_core::services::dto::InsertPosition,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSectionContentParams {
    pub id: String,
    pub section: String,
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteRenameSectionParams {
    pub id: String,
    pub section: String,
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct NoteSourceParams {
    pub id: String,
    #[serde(default)]
    pub archived: bool,
    pub range: Option<String>,
    #[serde(default)]
    pub view: SourceView,
}
