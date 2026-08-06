use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::schemars::JsonSchema;
use serde::Serialize;

use crate::gateway::GatewayRequestError;

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

pub(super) fn gateway_tool_error(error: &GatewayRequestError) -> CallToolResult {
    let message = error.to_string();
    let (code, retryable) = match error {
        GatewayRequestError::NotAuthenticated | GatewayRequestError::Authentication { .. } => {
            ("not_authenticated", false)
        }
        GatewayRequestError::RateLimited { .. } => ("gateway_rate_limited", true),
        _ => ("gateway_request_failed", false),
    };
    let payload = serde_json::to_value(ToolErrorPayload {
        code: code.to_string(),
        message: message.clone(),
        details: serde_json::json!({}),
        retryable,
    })
    .expect("tool error payload is serializable");
    let mut result = CallToolResult::error(vec![ContentBlock::text(message)]);
    result.structured_content = Some(payload);
    result
}

pub(super) fn gateway_config_error(error: &CliError) -> CallToolResult {
    let message = error.to_string();
    let payload = serde_json::to_value(ToolErrorPayload {
        code: "gateway_request_failed".to_string(),
        message: message.clone(),
        details: serde_json::json!({}),
        retryable: false,
    })
    .expect("tool error payload is serializable");
    let mut result = CallToolResult::error(vec![ContentBlock::text(message)]);
    result.structured_content = Some(payload);
    result
}

pub(super) fn gateway_invalid_response_error() -> CallToolResult {
    let message = "Gateway returned an invalid response.";
    let payload = serde_json::to_value(ToolErrorPayload {
        code: "gateway_invalid_response".to_string(),
        message: message.to_string(),
        details: serde_json::json!({}),
        retryable: false,
    })
    .expect("tool error payload is serializable");
    let mut result = CallToolResult::error(vec![ContentBlock::text(message)]);
    result.structured_content = Some(payload);
    result
}
