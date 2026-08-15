use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::bridge::CubaseBridge;
use crate::protocol::{BridgeError, BridgeRequest, ErrorCode};

const STATUS_METHOD: &str = "system.get_status";

pub struct IntegrationService {
    bridge: Arc<dyn CubaseBridge>,
    timeout: Duration,
    request_sequence: AtomicU64,
    session_prefix: u128,
}

impl IntegrationService {
    pub fn new(bridge: Arc<dyn CubaseBridge>, timeout: Duration) -> Self {
        let session_prefix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros();
        Self {
            bridge,
            timeout,
            request_sequence: AtomicU64::new(1),
            session_prefix,
        }
    }

    pub fn invoke_tool(&self, tool_name: &str) -> Result<Value, BridgeError> {
        let bridge_method = match tool_name {
            "cubase.get_status" => STATUS_METHOD,
            "cubase.play" => "transport.play",
            "cubase.stop" => "transport.stop",
            "cubase.record" => "transport.record",
            "cubase.get_transport" => "transport.get",
            "cubase.get_capabilities" => "capabilities.get",
            _ => {
                return Err(BridgeError::new(
                    ErrorCode::NotSupported,
                    format!("MCP tool '{tool_name}' is not supported"),
                ));
            }
        };

        match self.invoke_bridge(bridge_method) {
            Ok(value) => normalize_result(bridge_method, value),
            Err(error)
                if bridge_method == STATUS_METHOD && error.code == ErrorCode::NotConnected =>
            {
                Ok(json!({
                    "connected": false,
                    "project_open": null,
                    "playing": null,
                    "recording": null,
                    "tempo": null
                }))
            }
            Err(error) => Err(error),
        }
    }

    fn invoke_bridge(&self, method: &str) -> Result<Value, BridgeError> {
        let request_id = format!(
            "req-{}-{}",
            self.session_prefix,
            self.request_sequence.fetch_add(1, Ordering::Relaxed)
        );
        let request = BridgeRequest::new(request_id.clone(), method, json!({}));
        let started = Instant::now();
        let result = self.bridge.call(&request, self.timeout);
        log_request(
            &request_id,
            method,
            started.elapsed(),
            &result,
            self.bridge.is_connected(),
        );
        result
    }
}

fn normalize_result(method: &str, value: Value) -> Result<Value, BridgeError> {
    match method {
        STATUS_METHOD => normalize_status(value),
        "transport.get" => normalize_transport(value),
        "capabilities.get" => normalize_capabilities(value),
        "transport.play" | "transport.stop" | "transport.record" => require_object(value, method),
        _ => require_object(value, method),
    }
}

fn normalize_status(value: Value) -> Result<Value, BridgeError> {
    let object = object(value, STATUS_METHOD)?;
    let connected = required_bool(&object, "connected", STATUS_METHOD)?;
    let project_open = optional_bool(&object, "project_open", STATUS_METHOD)?;
    let playing = optional_bool(&object, "playing", STATUS_METHOD)?;
    let recording = optional_bool(&object, "recording", STATUS_METHOD)?;
    let tempo = optional_positive_number(&object, "tempo", STATUS_METHOD)?;

    Ok(json!({
        "connected": connected,
        "project_open": project_open,
        "playing": playing,
        "recording": recording,
        "tempo": tempo
    }))
}

fn normalize_transport(value: Value) -> Result<Value, BridgeError> {
    let object = object(value, "transport.get")?;
    let playing = required_bool(&object, "playing", "transport.get")?;
    let recording = required_bool(&object, "recording", "transport.get")?;
    let tempo = optional_positive_number(&object, "tempo", "transport.get")?;
    let position = match object.get("position") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Object(position)) => {
            for field in ["bars", "beats", "ticks"] {
                if let Some(value) = position.get(field)
                    && !value.is_i64()
                    && !value.is_u64()
                {
                    return Err(invalid_bridge_result(format!(
                        "transport.get position.{field} must be an integer"
                    )));
                }
            }
            Value::Object(position.clone())
        }
        Some(_) => {
            return Err(invalid_bridge_result(
                "transport.get position must be an object or null",
            ));
        }
    };

    Ok(json!({
        "playing": playing,
        "recording": recording,
        "tempo": tempo,
        "position": position
    }))
}

fn normalize_capabilities(value: Value) -> Result<Value, BridgeError> {
    let object = object(value, "capabilities.get")?;
    let transport =
        optional_capability_group(&object, "transport", &["read", "write"], "capabilities.get")?;
    let tracks = optional_capability_group(
        &object,
        "tracks",
        &["list", "select", "mute", "solo", "volume", "pan"],
        "capabilities.get",
    )?;

    Ok(json!({
        "transport": transport,
        "tracks": tracks,
        "markers": optional_bool(&object, "markers", "capabilities.get")?.unwrap_or(false),
        "commands": optional_bool(&object, "commands", "capabilities.get")?.unwrap_or(false),
        "audio_analysis": optional_bool(&object, "audio_analysis", "capabilities.get")?.unwrap_or(false),
        "plugin_parameters": optional_bool(&object, "plugin_parameters", "capabilities.get")?.unwrap_or(false)
    }))
}

fn optional_capability_group(
    object: &Map<String, Value>,
    group_name: &str,
    fields: &[&str],
    method: &str,
) -> Result<Value, BridgeError> {
    let group = match object.get(group_name) {
        None | Some(Value::Null) => None,
        Some(Value::Object(group)) => Some(group),
        Some(_) => {
            return Err(invalid_bridge_result(format!(
                "{method} field '{group_name}' must be an object"
            )));
        }
    };

    let mut normalized = Map::new();
    for field in fields {
        let value = match group.and_then(|group| group.get(*field)) {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(invalid_bridge_result(format!(
                    "{method} field '{group_name}.{field}' must be a boolean"
                )));
            }
        };
        normalized.insert((*field).into(), Value::Bool(value));
    }
    Ok(Value::Object(normalized))
}

fn require_object(value: Value, method: &str) -> Result<Value, BridgeError> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(invalid_bridge_result(format!(
            "{method} result must be an object"
        )))
    }
}

fn object(value: Value, method: &str) -> Result<Map<String, Value>, BridgeError> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| invalid_bridge_result(format!("{method} result must be an object")))
}

fn required_bool(
    object: &Map<String, Value>,
    field: &str,
    method: &str,
) -> Result<bool, BridgeError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid_bridge_result(format!("{method} field '{field}' must be a boolean")))
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
    method: &str,
) -> Result<Option<bool>, BridgeError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(invalid_bridge_result(format!(
            "{method} field '{field}' must be a boolean or null"
        ))),
    }
}

fn optional_positive_number(
    object: &Map<String, Value>,
    field: &str,
    method: &str,
) -> Result<Option<f64>, BridgeError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_f64() {
            Some(number) if number.is_finite() && number > 0.0 => Ok(Some(number)),
            _ => Err(invalid_bridge_result(format!(
                "{method} field '{field}' must be a positive number or null"
            ))),
        },
    }
}

fn invalid_bridge_result(message: impl Into<String>) -> BridgeError {
    BridgeError::new(ErrorCode::ProtocolError, message)
}

fn log_request(
    request_id: &str,
    method: &str,
    duration: Duration,
    result: &Result<Value, BridgeError>,
    connected: bool,
) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let (result_name, error_code) = match result {
        Ok(_) => ("success", Value::Null),
        Err(error) => ("error", Value::String(error.code.as_str().into())),
    };
    eprintln!(
        "{}",
        json!({
            "timestamp": timestamp,
            "request_id": request_id,
            "method": method,
            "duration_ms": duration.as_secs_f64() * 1000.0,
            "result": result_name,
            "error_code": error_code,
            "bridge_connection_state": connected
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::MockBridge;

    #[test]
    fn status_reports_disconnected_without_failing_tool_call() {
        let bridge = Arc::new(MockBridge::new());
        bridge.set_connected(false);
        let service = IntegrationService::new(bridge, Duration::from_millis(10));

        let status = service.invoke_tool("cubase.get_status").unwrap();
        assert_eq!(status["connected"], false);
        assert!(status["tempo"].is_null());
    }

    #[test]
    fn state_changing_tools_round_trip_through_bridge() {
        let service =
            IntegrationService::new(Arc::new(MockBridge::new()), Duration::from_millis(10));

        service.invoke_tool("cubase.record").unwrap();
        let transport = service.invoke_tool("cubase.get_transport").unwrap();
        assert_eq!(transport["playing"], true);
        assert_eq!(transport["recording"], true);
    }

    #[test]
    fn omitted_capabilities_are_normalized_to_false() {
        let normalized = normalize_capabilities(json!({
            "transport": {"read": true}
        }))
        .unwrap();
        assert_eq!(normalized["transport"]["read"], true);
        assert_eq!(normalized["transport"]["write"], false);
        assert_eq!(normalized["tracks"]["list"], false);
        assert_eq!(normalized["markers"], false);
    }
}
