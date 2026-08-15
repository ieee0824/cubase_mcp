use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const BRIDGE_PROTOCOL_VERSION: u32 = 1;

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BridgeMessageType {
    Request,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeRequest {
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: BridgeMessageType,
    pub method: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

impl BridgeRequest {
    pub fn new(id: String, method: impl Into<String>, params: Value) -> Self {
        Self {
            version: BRIDGE_PROTOCOL_VERSION,
            id,
            message_type: BridgeMessageType::Request,
            method: method.into(),
            params,
        }
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.version != BRIDGE_PROTOCOL_VERSION {
            return Err(BridgeError::new(
                ErrorCode::ProtocolError,
                format!(
                    "Unsupported bridge protocol version {}; expected {}",
                    self.version, BRIDGE_PROTOCOL_VERSION
                ),
            ));
        }
        if self.id.is_empty() {
            return Err(BridgeError::new(
                ErrorCode::ProtocolError,
                "Request id must not be empty",
            ));
        }
        if self.method.is_empty() {
            return Err(BridgeError::new(
                ErrorCode::ProtocolError,
                "Request method must not be empty",
            ));
        }
        if !self.params.is_object() {
            return Err(BridgeError::new(
                ErrorCode::InvalidArgument,
                "Request params must be an object",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BridgeIncoming {
    Response {
        version: u32,
        id: String,
        result: Value,
    },
    Error {
        version: u32,
        id: String,
        error: BridgeError,
    },
    Event {
        version: u32,
        event: String,
        data: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeResponse {
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: ResponseMessageType,
    pub result: Value,
}

impl BridgeResponse {
    pub fn new(id: String, result: Value) -> Self {
        Self {
            version: BRIDGE_PROTOCOL_VERSION,
            id,
            message_type: ResponseMessageType::Response,
            result,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResponseMessageType {
    Response,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeErrorResponse {
    pub version: u32,
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: ErrorMessageType,
    pub error: BridgeError,
}

impl BridgeErrorResponse {
    pub fn new(id: String, error: BridgeError) -> Self {
        Self {
            version: BRIDGE_PROTOCOL_VERSION,
            id,
            message_type: ErrorMessageType::Error,
            error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ErrorMessageType {
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotConnected,
    ProjectNotOpen,
    NotSupported,
    InvalidArgument,
    TrackNotFound,
    MarkerNotFound,
    CommandNotFound,
    CommandNotAllowed,
    Timeout,
    Busy,
    ProtocolError,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConnected => "NOT_CONNECTED",
            Self::ProjectNotOpen => "PROJECT_NOT_OPEN",
            Self::NotSupported => "NOT_SUPPORTED",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::TrackNotFound => "TRACK_NOT_FOUND",
            Self::MarkerNotFound => "MARKER_NOT_FOUND",
            Self::CommandNotFound => "COMMAND_NOT_FOUND",
            Self::CommandNotAllowed => "COMMAND_NOT_ALLOWED",
            Self::Timeout => "TIMEOUT",
            Self::Busy => "BUSY",
            Self::ProtocolError => "PROTOCOL_ERROR",
            Self::InternalError => "INTERNAL_ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeError {
    pub code: ErrorCode,
    pub message: String,
}

impl BridgeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn not_connected(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotConnected, message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ProtocolError, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for BridgeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_protocol_shape() {
        let request = BridgeRequest::new("req-1".into(), "transport.play", json!({}));
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "version": 1,
                "id": "req-1",
                "type": "request",
                "method": "transport.play",
                "params": {}
            })
        );
    }

    #[test]
    fn error_codes_use_spec_names() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::TrackNotFound).unwrap(),
            "\"TRACK_NOT_FOUND\""
        );
    }

    #[test]
    fn incoming_event_is_distinct_from_response() {
        let message: BridgeIncoming = serde_json::from_value(json!({
            "version": 1,
            "type": "event",
            "event": "tempo.changed",
            "data": {"tempo": 132.0}
        }))
        .unwrap();

        assert!(matches!(message, BridgeIncoming::Event { .. }));
    }

    #[test]
    fn incoming_response_requires_result() {
        let error = serde_json::from_value::<BridgeIncoming>(json!({
            "version": 1,
            "id": "req-1",
            "type": "response"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("result"));
    }

    #[test]
    fn incoming_event_requires_data() {
        let error = serde_json::from_value::<BridgeIncoming>(json!({
            "version": 1,
            "type": "event",
            "event": "transport.changed"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("data"));
    }
}
