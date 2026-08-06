use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct GatewayWebSearchParams {
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct GatewayWebFetchParams {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct GatewayWebSearchResult {
    pub results: Vec<GatewayWebSearchItem>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct GatewayWebSearchItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct GatewayWebFetchResult {
    pub content: String,
    #[serde(rename = "wordCount")]
    pub word_count: u64,
}
