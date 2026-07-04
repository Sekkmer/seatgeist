use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    BackendCapability, CoordinateSpace, MonitorInfo, Observation, SafetyClass, ToolApprovalLevel,
};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotInfo {
    pub path: PathBuf,
    pub backend: String,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub transform: ScreenshotTransform,
    pub coordinate_space: CoordinateSpace,
    pub monitors: Vec<MonitorInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotTransform {
    pub source_coordinate_space: CoordinateSpace,
    pub output_coordinate_space: CoordinateSpace,
    pub source_origin_x: u32,
    pub source_origin_y: u32,
    pub scale_x: f64,
    pub scale_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    pub output: PathBuf,
    pub max_edge: Option<u32>,
    pub full_resolution: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health,
    Capabilities,
    PolicyStatus,
    ListMonitors,
    Screenshot(ScreenshotRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DaemonResponse {
    Health(HealthStatus),
    Capabilities(CapabilitySet),
    PolicyStatus(PolicyStatus),
    Monitors(Vec<MonitorInfo>),
    Screenshot(ScreenshotInfo),
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

    #[test]
    fn serializes_screenshot_request_with_output_path() {
        let request = DaemonRequest::Screenshot(ScreenshotRequest {
            output: PathBuf::from("/tmp/plasma-pilot.png"),
            max_edge: Some(1600),
            full_resolution: false,
        });
        let encoded = serde_json::to_string(&request).expect("screenshot request serializes");
        assert!(encoded.contains(r#""method":"screenshot""#));
        assert!(encoded.contains(r#"/tmp/plasma-pilot.png"#));
        assert!(encoded.contains(r#""max_edge":1600"#));
    }

    #[test]
    fn serializes_monitor_response_with_type_tag() {
        let response = DaemonResponse::Monitors(Vec::new());
        let encoded = serde_json::to_string(&response).expect("monitor response serializes");
        assert_eq!(encoded, r#"{"type":"monitors","data":[]}"#);
    }
}
