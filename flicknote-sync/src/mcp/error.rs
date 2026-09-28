use flicknote_core::services::error::ServiceError;
use rmcp::model::{CallToolResult, ContentBlock};
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
        ServiceError::Remote {
            details: Some(details),
            ..
        } => details.clone(),
        _ => serde_json::json!({}),
    };
    let payload = serde_json::to_value(ToolErrorPayload {
        code: error.code().to_string(),
        message: error.to_string(),
        details,
        retryable: error.retryable(),
    })
    .expect("tool error payload is serializable");
    let mut result = CallToolResult::error(vec![ContentBlock::text(error.to_string())]);
    result.structured_content = Some(payload);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_partial_success_details_are_structured_for_mcp_callers() {
        let result = tool_error(&ServiceError::Remote {
            code: "note_create_partial".to_string(),
            message: "created with pending topics".to_string(),
            retryable: false,
            details: Some(serde_json::json!({"created": true, "short_id": 80})),
        });

        assert_eq!(
            result.structured_content.unwrap()["details"],
            serde_json::json!({"created": true, "short_id": 80})
        );
    }
}
