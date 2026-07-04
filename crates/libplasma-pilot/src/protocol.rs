use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{BackendCapability, Observation, SafetyClass, ToolApprovalLevel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthStatus {
    pub service: String,
    pub version: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub capabilities: Vec<BackendCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyStatus {
    pub default_observe: ToolApprovalLevel,
    pub default_control: ToolApprovalLevel,
    pub default_clipboard_read: ToolApprovalLevel,
    pub default_clipboard_write: ToolApprovalLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health,
    Capabilities,
    PolicyStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DaemonResponse {
    Health(HealthStatus),
    Capabilities(CapabilitySet),
    PolicyStatus(PolicyStatus),
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub id: Uuid,
    pub tool: String,
    pub safety_class: SafetyClass,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub id: Uuid,
    pub ok: bool,
    pub observation: Option<Observation>,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_health_request_as_snake_case_method() {
        let encoded =
            serde_json::to_string(&DaemonRequest::Health).expect("health request serializes");
        assert_eq!(encoded, r#"{"method":"health"}"#);
        let decoded: DaemonRequest =
            serde_json::from_str(&encoded).expect("health request deserializes");
        assert_eq!(decoded, DaemonRequest::Health);
    }

    #[test]
    fn serializes_capabilities_response_with_type_tag() {
        let response = DaemonResponse::Capabilities(CapabilitySet {
            capabilities: vec![BackendCapability::DaemonHealth],
        });
        let encoded = serde_json::to_string(&response).expect("capabilities response serializes");
        assert!(encoded.contains(r#""type":"capabilities""#));
        assert!(encoded.contains(r#""daemon_health""#));
    }
}
