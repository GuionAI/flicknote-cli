use flicknote_core::services::error::ServiceError;
use rmcp::model::CallToolResult;
use rmcp::schemars::JsonSchema;
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
struct ToolErrorPayload {
    code: String,
    message: String,
    details: serde_json::Value,
    retryable: bool,
}

pub(super) fn tool_error(error: &ServiceError) -> CallToolResult {
    let details = match error {
        ServiceError::BeforeAmbiguous { matches, .. } => {
            serde_json::json!({ "matches": matches })
        }
        _ => serde_json::json!({}),
    };
    CallToolResult::structured_error(
        serde_json::to_value(ToolErrorPayload {
            code: error.code().to_string(),
            message: error.to_string(),
            details,
            retryable: error.retryable(),
        })
        .expect("tool error payload is serializable"),
    )
}
