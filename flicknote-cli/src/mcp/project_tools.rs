use flicknote_core::services::dto::Patch;
use rmcp::schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectListParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectIdParams {
    pub project: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectAddParams {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectModifyParams {
    pub project: String,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub color: Patch<String>,
}
