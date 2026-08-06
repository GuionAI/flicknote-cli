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
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectAddParams {
    pub name: String,
    pub keyterm: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ProjectModifyParams {
    pub id: String,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub keyterm: Patch<String>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub color: Patch<String>,
}
