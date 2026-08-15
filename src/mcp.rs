use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::protocol::{BridgeError, ErrorCode};
use crate::service::IntegrationService;

pub const LATEST_MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_MCP_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_MCP_MESSAGE_BYTES: usize = 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const SERVER_NOT_INITIALIZED: i64 = -32002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Uninitialized,
    AwaitingInitializedNotification,
    Ready,
}

pub struct McpServer {
    service: IntegrationService,
    lifecycle: LifecycleState,
    protocol_version: String,
}

impl McpServer {
    pub fn new(service: IntegrationService) -> Self {
        Self {
            service,
            lifecycle: LifecycleState::Uninitialized,
            protocol_version: LATEST_MCP_PROTOCOL_VERSION.into(),
        }
    }

    pub fn serve_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        self.serve(stdin.lock(), stdout.lock())
    }

    pub fn serve<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> io::Result<()> {
        loop {
            let mut bytes = Vec::new();
            let byte_count = reader.read_until(b'\n', &mut bytes)?;
            if byte_count == 0 {
                return Ok(());
            }

            if bytes.len() > MAX_MCP_MESSAGE_BYTES {
                write_message(
                    &mut writer,
                    &rpc_error(
                        Value::Null,
                        INVALID_REQUEST,
                        "MCP message exceeds the 1 MiB limit",
                    ),
                )?;
                continue;
            }
            if bytes.ends_with(b"\n") {
                bytes.pop();
            }
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }

            let message = match serde_json::from_slice::<Value>(&bytes) {
                Ok(message) => message,
                Err(error) => {
                    write_message(
                        &mut writer,
                        &rpc_error(Value::Null, PARSE_ERROR, &format!("Parse error: {error}")),
                    )?;
                    continue;
                }
            };

            if let Some(response) = self.process_message(message) {
                write_message(&mut writer, &response)?;
            }
        }
    }

    fn process_message(&mut self, message: Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(rpc_error(
                Value::Null,
                INVALID_REQUEST,
                "JSON-RPC message must be an object",
            ));
        };

        let id = object.get("id").cloned();
        let response_id = id.clone().unwrap_or(Value::Null);
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return id.map(|_| rpc_error(response_id, INVALID_REQUEST, "jsonrpc must equal '2.0'"));
        }

        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(rpc_error(
                response_id,
                INVALID_REQUEST,
                "JSON-RPC request must include a string method",
            ));
        };

        if let Some(id) = &id
            && !id.is_string()
            && !id.is_number()
        {
            return Some(rpc_error(
                Value::Null,
                INVALID_REQUEST,
                "JSON-RPC id must be a string or number",
            ));
        }

        let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
        match id {
            Some(id) => Some(self.handle_request(id, method, params)),
            None => {
                self.handle_notification(method);
                None
            }
        }
    }

    fn handle_notification(&mut self, method: &str) {
        if method == "notifications/initialized"
            && self.lifecycle == LifecycleState::AwaitingInitializedNotification
        {
            self.lifecycle = LifecycleState::Ready;
        }
    }

    fn handle_request(&mut self, id: Value, method: &str, params: Value) -> Value {
        match method {
            "initialize" => self.initialize(id, params),
            "ping" => rpc_success(id, json!({})),
            _ if self.lifecycle != LifecycleState::Ready => {
                rpc_error(id, SERVER_NOT_INITIALIZED, "Server is not initialized")
            }
            "tools/list" => self.list_tools(id, params),
            "tools/call" => self.call_tool(id, params),
            _ => rpc_error(id, METHOD_NOT_FOUND, "Method not found"),
        }
    }

    fn initialize(&mut self, id: Value, params: Value) -> Value {
        if self.lifecycle != LifecycleState::Uninitialized {
            return rpc_error(id, INVALID_REQUEST, "Server is already initialized");
        }
        let Some(params) = params.as_object() else {
            return rpc_error(id, INVALID_PARAMS, "initialize params must be an object");
        };
        let Some(requested_version) = params.get("protocolVersion").and_then(Value::as_str) else {
            return rpc_error(id, INVALID_PARAMS, "initialize requires protocolVersion");
        };
        if !params.get("capabilities").is_some_and(Value::is_object) {
            return rpc_error(id, INVALID_PARAMS, "initialize requires capabilities");
        }
        let client_info_is_valid = params
            .get("clientInfo")
            .and_then(Value::as_object)
            .is_some_and(|client_info| {
                client_info.get("name").is_some_and(Value::is_string)
                    && client_info.get("version").is_some_and(Value::is_string)
            });
        if !client_info_is_valid {
            return rpc_error(
                id,
                INVALID_PARAMS,
                "initialize requires clientInfo.name and clientInfo.version",
            );
        }

        self.protocol_version = if SUPPORTED_MCP_PROTOCOL_VERSIONS.contains(&requested_version) {
            requested_version.into()
        } else {
            LATEST_MCP_PROTOCOL_VERSION.into()
        };
        self.lifecycle = LifecycleState::AwaitingInitializedNotification;

        rpc_success(
            id,
            json!({
                "protocolVersion": self.protocol_version,
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "cubase_mcp",
                    "title": "Cubase MCP Integration",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Use cubase.get_capabilities before relying on optional Cubase features. State-changing tools affect the open Cubase project through the configured local bridge."
            }),
        )
    }

    fn list_tools(&self, id: Value, params: Value) -> Value {
        let Some(params) = params.as_object() else {
            return rpc_error(id, INVALID_PARAMS, "tools/list params must be an object");
        };
        if params.get("cursor").is_some_and(|cursor| !cursor.is_null()) {
            return rpc_error(id, INVALID_PARAMS, "tools/list cursor is not supported");
        }

        rpc_success(id, json!({"tools": tool_definitions()}))
    }

    fn call_tool(&self, id: Value, params: Value) -> Value {
        let Some(params) = params.as_object() else {
            return rpc_error(id, INVALID_PARAMS, "tools/call params must be an object");
        };
        let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
            return rpc_error(id, INVALID_PARAMS, "tools/call requires a tool name");
        };
        if !is_known_tool(tool_name) {
            return rpc_error(id, INVALID_PARAMS, &format!("Unknown tool '{tool_name}'"));
        }

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(arguments) = arguments.as_object() else {
            return rpc_error(id, INVALID_PARAMS, "Tool arguments must be an object");
        };
        if !arguments.is_empty() {
            return rpc_success(
                id,
                tool_error(&BridgeError::new(
                    ErrorCode::InvalidArgument,
                    format!("Tool '{tool_name}' does not accept arguments"),
                )),
            );
        }

        match self.service.invoke_tool(tool_name) {
            Ok(result) => rpc_success(id, tool_success(result)),
            Err(error) => rpc_success(id, tool_error(&error)),
        }
    }
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool_definition(
            "cubase.get_status",
            "Get Cubase Status",
            "Get Cubase bridge connectivity and basic project/transport state.",
            true,
            true,
        ),
        tool_definition(
            "cubase.play",
            "Start Cubase Playback",
            "Start playback in the open Cubase project.",
            false,
            true,
        ),
        tool_definition(
            "cubase.stop",
            "Stop Cubase Transport",
            "Stop playback or recording in Cubase.",
            false,
            true,
        ),
        tool_definition(
            "cubase.record",
            "Start Cubase Recording",
            "Start recording in the open Cubase project.",
            false,
            true,
        ),
        tool_definition(
            "cubase.get_transport",
            "Get Cubase Transport",
            "Get playback, recording, tempo, and musical position when available.",
            true,
            true,
        ),
        tool_definition(
            "cubase.get_capabilities",
            "Get Cubase Capabilities",
            "Get the features supported by the active Cubase bridge.",
            true,
            true,
        ),
    ]
}

fn tool_definition(
    name: &str,
    title: &str,
    description: &str,
    read_only: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {},
            "additionalProperties": false
        },
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
            "openWorldHint": false
        }
    })
}

fn is_known_tool(name: &str) -> bool {
    matches!(
        name,
        "cubase.get_status"
            | "cubase.play"
            | "cubase.stop"
            | "cubase.record"
            | "cubase.get_transport"
            | "cubase.get_capabilities"
    )
}

fn tool_success(result: Value) -> Value {
    let text = serde_json::to_string(&result).unwrap_or_else(|_| "{}".into());
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": result,
        "isError": false
    })
}

fn tool_error(error: &BridgeError) -> Value {
    let structured = json!({"error": error});
    let text = serde_json::to_string(&structured).unwrap_or_else(|_| {
        "{\"error\":{\"code\":\"INTERNAL_ERROR\",\"message\":\"Could not encode error\"}}".into()
    });
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
        "isError": true
    })
}

fn rpc_success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|error| io::Error::other(format!("Could not encode MCP response: {error}")))?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::bridge::MockBridge;

    use super::*;

    fn run_session(input: &str) -> Vec<Value> {
        let service =
            IntegrationService::new(Arc::new(MockBridge::new()), Duration::from_millis(100));
        let mut server = McpServer::new(service);
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut output = Vec::new();
        server.serve(reader, &mut output).unwrap();

        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn initialize() -> &'static str {
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n\
         {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n"
    }

    #[test]
    fn initialize_then_list_all_mvp_tools() {
        let input = format!(
            "{}{}\n",
            initialize(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        );
        let responses = run_session(&input);
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(responses[1]["result"]["tools"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn tool_call_returns_structured_and_text_content() {
        let input = format!(
            "{}{}\n",
            initialize(),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "cubase.get_transport", "arguments": {}}
            })
        );
        let responses = run_session(&input);
        let result = &responses[1]["result"];
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["playing"], false);
        assert!(result["content"][0]["text"].as_str().is_some());
    }

    #[test]
    fn invalid_tool_arguments_are_visible_to_the_model() {
        let input = format!(
            "{}{}\n",
            initialize(),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "cubase.play", "arguments": {"unexpected": true}}
            })
        );
        let responses = run_session(&input);
        let result = &responses[1]["result"];
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["structuredContent"]["error"]["code"],
            "INVALID_ARGUMENT"
        );
    }

    #[test]
    fn tools_are_rejected_until_initialized_notification() {
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n";
        let responses = run_session(input);
        assert_eq!(responses[0]["error"]["code"], SERVER_NOT_INITIALIZED);
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let responses = run_session("not-json\n");
        assert_eq!(responses[0]["error"]["code"], PARSE_ERROR);
        assert!(responses[0]["id"].is_null());
    }
}
