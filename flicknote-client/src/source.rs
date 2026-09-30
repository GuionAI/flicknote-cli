//! Source view and result wire types; parsing remains daemon-owned.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceView {
    #[default]
    Rendered,
    Raw,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum SourceResult {
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
        value: serde_json::Value,
    },
    Info {
        source_type: String,
        range_unit: String,
        count: usize,
    },
}
