use rmcp::schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::dto::arbitrary_json_schema;

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CommentListParams {
    pub note_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CommentCreateParams {
    pub note_id: String,
    pub block_text: String,
    #[schemars(schema_with = "arbitrary_json_schema")]
    pub content: serde_json::Value,
    pub author: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub is_read: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct CommentModifyParams {
    pub id: String,
    #[schemars(schema_with = "arbitrary_json_schema")]
    pub content: Option<serde_json::Value>,
    pub is_read: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PendingCommentsParams {
    #[serde(default = "default_limit")]
    pub limit: u32,
}

const fn default_limit() -> u32 {
    100
}
