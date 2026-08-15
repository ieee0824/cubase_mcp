use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::bridge::CubaseBridge;
use crate::protocol::{
    BRIDGE_PROTOCOL_VERSION, BridgeError, BridgeIncoming, BridgeRequest, ErrorCode,
};

pub const DEFAULT_TO_CUBASE_PORT: &str = "Cubase MCP To Cubase";
pub const DEFAULT_FROM_CUBASE_PORT: &str = "Cubase MCP From Cubase";

const SYSEX_HEADER: [u8; 7] = [0xF0, 0x7D, b'C', b'M', b'C', b'P', 0x01];
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_SYSEX_BYTES: usize = SYSEX_HEADER.len() + MAX_JSON_BYTES * 2 + 1;
const MAX_MESSAGES_BEFORE_RESPONSE: usize = 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MidiPortListing {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

struct MidiCallbackState {
    sender: Sender<Vec<u8>>,
    framer: SysexFramer,
}

struct MidiState {
    output: MidiOutputConnection,
    _input: MidiInputConnection<MidiCallbackState>,
    receiver: Receiver<Vec<u8>>,
}

/// Bridge Protocol over a 7-bit-safe MIDI SysEx envelope.
///
/// On Unix platforms the default constructor creates two virtual ports, so
/// Cubase can attach without an external loopback MIDI driver.
pub struct MidiBridge {
    state: Mutex<MidiState>,
    connected: AtomicBool,
}

impl MidiBridge {
    #[cfg(unix)]
    pub fn new_virtual() -> Result<Self, BridgeError> {
        let output = MidiOutput::new("cubase-mcp-output")
            .map_err(|error| midi_init_error("MIDI output", error))?
            .create_virtual(DEFAULT_TO_CUBASE_PORT)
            .map_err(|error| midi_connect_error(DEFAULT_TO_CUBASE_PORT, error))?;

        let mut input = MidiInput::new("cubase-mcp-input")
            .map_err(|error| midi_init_error("MIDI input", error))?;
        input.ignore(Ignore::None);
        let (sender, receiver) = mpsc::channel();
        let input = input
            .create_virtual(
                DEFAULT_FROM_CUBASE_PORT,
                receive_midi,
                MidiCallbackState {
                    sender,
                    framer: SysexFramer::default(),
                },
            )
            .map_err(|error| midi_connect_error(DEFAULT_FROM_CUBASE_PORT, error))?;

        Ok(Self {
            state: Mutex::new(MidiState {
                output,
                _input: input,
                receiver,
            }),
            connected: AtomicBool::new(false),
        })
    }

    #[cfg(not(unix))]
    pub fn new_virtual() -> Result<Self, BridgeError> {
        Err(BridgeError::new(
            ErrorCode::NotSupported,
            "Virtual MIDI ports are not supported on this platform; configure existing ports",
        ))
    }

    pub fn new_with_ports(input_name: &str, output_name: &str) -> Result<Self, BridgeError> {
        let mut input = MidiInput::new("cubase-mcp-input")
            .map_err(|error| midi_init_error("MIDI input", error))?;
        input.ignore(Ignore::None);
        let input_port = find_input_port(&input, input_name)?;

        let output = MidiOutput::new("cubase-mcp-output")
            .map_err(|error| midi_init_error("MIDI output", error))?;
        let output_port = find_output_port(&output, output_name)?;

        let (sender, receiver) = mpsc::channel();
        let input = input
            .connect(
                &input_port,
                "cubase-mcp-input-connection",
                receive_midi,
                MidiCallbackState {
                    sender,
                    framer: SysexFramer::default(),
                },
            )
            .map_err(|error| midi_connect_error(input_name, error))?;
        let output = output
            .connect(&output_port, "cubase-mcp-output-connection")
            .map_err(|error| midi_connect_error(output_name, error))?;

        Ok(Self {
            state: Mutex::new(MidiState {
                output,
                _input: input,
                receiver,
            }),
            connected: AtomicBool::new(false),
        })
    }

    pub fn list_ports() -> Result<MidiPortListing, BridgeError> {
        let input = MidiInput::new("cubase-mcp-list-inputs")
            .map_err(|error| midi_init_error("MIDI input", error))?;
        let output = MidiOutput::new("cubase-mcp-list-outputs")
            .map_err(|error| midi_init_error("MIDI output", error))?;

        let inputs = input
            .ports()
            .iter()
            .map(|port| {
                input.port_name(port).map_err(|error| {
                    BridgeError::internal(format!("Could not read MIDI input port name: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let outputs = output
            .ports()
            .iter()
            .map(|port| {
                output.port_name(port).map_err(|error| {
                    BridgeError::internal(format!("Could not read MIDI output port name: {error}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(MidiPortListing { inputs, outputs })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, MidiState>, BridgeError> {
        self.state
            .lock()
            .map_err(|_| BridgeError::internal("MIDI bridge state lock was poisoned"))
    }

    fn receive_until_response(
        &self,
        state: &mut MidiState,
        expected_id: &str,
        timeout: Duration,
    ) -> Result<Value, BridgeError> {
        let deadline = Instant::now() + timeout;
        let mut message_count = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.connected.store(false, Ordering::Release);
                return Err(BridgeError::not_connected(
                    "Cubase MIDI Remote did not respond before the timeout",
                ));
            }

            let frame = match state.receiver.recv_timeout(remaining) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => {
                    self.connected.store(false, Ordering::Release);
                    return Err(BridgeError::not_connected(
                        "Cubase MIDI Remote did not respond before the timeout",
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.connected.store(false, Ordering::Release);
                    return Err(BridgeError::internal(
                        "MIDI receive callback stopped unexpectedly",
                    ));
                }
            };
            message_count += 1;
            if message_count > MAX_MESSAGES_BEFORE_RESPONSE {
                return Err(BridgeError::protocol(
                    "Too many MIDI bridge messages arrived before the response",
                ));
            }

            match decode_sysex(&frame)? {
                BridgeIncoming::Response {
                    version,
                    id,
                    result,
                } => {
                    validate_version(version)?;
                    if id != expected_id {
                        log_stale_response(&id, expected_id);
                        continue;
                    }
                    self.connected.store(true, Ordering::Release);
                    return Ok(result);
                }
                BridgeIncoming::Error { version, id, error } => {
                    validate_version(version)?;
                    if id != expected_id {
                        log_stale_response(&id, expected_id);
                        continue;
                    }
                    if error.message.is_empty() {
                        return Err(BridgeError::protocol(
                            "Bridge error message must not be empty",
                        ));
                    }
                    self.connected.store(true, Ordering::Release);
                    return Err(error);
                }
                BridgeIncoming::Event {
                    version,
                    event,
                    data,
                } => {
                    validate_version(version)?;
                    self.handle_event(&event, &data)?;
                }
            }
        }
    }

    fn drain_events(&self, state: &mut MidiState) -> Result<(), BridgeError> {
        loop {
            match state.receiver.try_recv() {
                Ok(frame) => match decode_sysex(&frame)? {
                    BridgeIncoming::Event {
                        version,
                        event,
                        data,
                    } => {
                        validate_version(version)?;
                        self.handle_event(&event, &data)?;
                    }
                    BridgeIncoming::Response { id, .. } | BridgeIncoming::Error { id, .. } => {
                        log_stale_response(&id, "none");
                    }
                },
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    return Err(BridgeError::internal(
                        "MIDI receive callback stopped unexpectedly",
                    ));
                }
            }
        }
    }

    fn handle_event(&self, event: &str, data: &Value) -> Result<(), BridgeError> {
        if event.is_empty() || !data.is_object() {
            return Err(BridgeError::protocol(
                "Bridge event requires a non-empty name and object data",
            ));
        }
        if event == "connection.changed"
            && let Some(connected) = data.get("connected").and_then(Value::as_bool)
        {
            self.connected.store(connected, Ordering::Release);
        } else {
            self.connected.store(true, Ordering::Release);
        }
        let timestamp = unix_timestamp_ms();
        eprintln!(
            "{}",
            json!({
                "timestamp": timestamp,
                "event": event,
                "source": "midi_bridge"
            })
        );
        Ok(())
    }
}

impl CubaseBridge for MidiBridge {
    fn call(&self, request: &BridgeRequest, timeout: Duration) -> Result<Value, BridgeError> {
        request.validate()?;
        let frame = encode_sysex(request)?;
        let mut state = self.lock_state()?;
        self.drain_events(&mut state)?;
        state.output.send(&frame).map_err(|error| {
            self.connected.store(false, Ordering::Release);
            BridgeError::not_connected(format!("Could not send MIDI SysEx to Cubase: {error}"))
        })?;
        self.receive_until_response(&mut state, &request.id, timeout)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
}

fn receive_midi(_timestamp: u64, message: &[u8], state: &mut MidiCallbackState) {
    for frame in state.framer.push(message) {
        let _ = state.sender.send(frame);
    }
}

fn find_input_port(input: &MidiInput, requested: &str) -> Result<MidiInputPort, BridgeError> {
    let ports = input.ports();
    select_port(
        requested,
        ports
            .iter()
            .filter_map(|port| input.port_name(port).ok().map(|name| (port, name))),
        "input",
    )
}

fn find_output_port(output: &MidiOutput, requested: &str) -> Result<MidiOutputPort, BridgeError> {
    let ports = output.ports();
    select_port(
        requested,
        ports
            .iter()
            .filter_map(|port| output.port_name(port).ok().map(|name| (port, name))),
        "output",
    )
}

fn select_port<'a, T: Clone + 'a>(
    requested: &str,
    ports: impl Iterator<Item = (&'a T, String)>,
    direction: &str,
) -> Result<T, BridgeError> {
    let ports: Vec<(&T, String)> = ports.collect();
    if let Some((port, _)) = ports.iter().find(|(_, name)| name == requested) {
        return Ok((*port).clone());
    }

    let requested_lower = requested.to_lowercase();
    let matches: Vec<&T> = ports
        .iter()
        .filter(|(_, name)| name.to_lowercase().contains(&requested_lower))
        .map(|(port, _)| *port)
        .collect();
    match matches.as_slice() {
        [port] => Ok((*port).clone()),
        [] => Err(BridgeError::not_connected(format!(
            "MIDI {direction} port matching '{requested}' was not found"
        ))),
        _ => Err(BridgeError::new(
            ErrorCode::InvalidArgument,
            format!("MIDI {direction} port name '{requested}' is ambiguous"),
        )),
    }
}

fn encode_sysex(value: &impl Serialize) -> Result<Vec<u8>, BridgeError> {
    let json = serde_json::to_vec(value).map_err(|error| {
        BridgeError::internal(format!("Could not encode MIDI request: {error}"))
    })?;
    if json.len() > MAX_JSON_BYTES {
        return Err(BridgeError::new(
            ErrorCode::InvalidArgument,
            format!("MIDI bridge JSON exceeds the {MAX_JSON_BYTES}-byte limit"),
        ));
    }

    let mut frame = Vec::with_capacity(SYSEX_HEADER.len() + json.len() * 2 + 1);
    frame.extend_from_slice(&SYSEX_HEADER);
    for byte in json {
        frame.push((byte >> 4) & 0x0F);
        frame.push(byte & 0x0F);
    }
    frame.push(0xF7);
    Ok(frame)
}

fn decode_sysex(frame: &[u8]) -> Result<BridgeIncoming, BridgeError> {
    if frame.len() < SYSEX_HEADER.len() + 1
        || !frame.starts_with(&SYSEX_HEADER)
        || frame.last() != Some(&0xF7)
    {
        return Err(BridgeError::protocol(
            "MIDI message is not a Cubase MCP SysEx frame",
        ));
    }
    let payload = &frame[SYSEX_HEADER.len()..frame.len() - 1];
    if !payload.len().is_multiple_of(2) {
        return Err(BridgeError::protocol(
            "MIDI SysEx payload has an odd number of nibbles",
        ));
    }
    if payload.len() / 2 > MAX_JSON_BYTES {
        return Err(BridgeError::protocol(
            "MIDI SysEx payload exceeds the JSON size limit",
        ));
    }

    let mut json_bytes = Vec::with_capacity(payload.len() / 2);
    for nibbles in payload.chunks_exact(2) {
        if nibbles[0] > 0x0F || nibbles[1] > 0x0F {
            return Err(BridgeError::protocol(
                "MIDI SysEx payload contains a non-nibble byte",
            ));
        }
        json_bytes.push((nibbles[0] << 4) | nibbles[1]);
    }

    serde_json::from_slice(&json_bytes)
        .map_err(|error| BridgeError::protocol(format!("Invalid MIDI bridge JSON: {error}")))
}

fn validate_version(version: u32) -> Result<(), BridgeError> {
    if version == BRIDGE_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(BridgeError::protocol(format!(
            "Unsupported bridge protocol version {version}"
        )))
    }
}

fn midi_init_error(component: &str, error: impl std::fmt::Display) -> BridgeError {
    BridgeError::not_connected(format!("Could not initialize {component}: {error}"))
}

fn midi_connect_error(port: &str, error: impl std::fmt::Display) -> BridgeError {
    BridgeError::not_connected(format!("Could not connect MIDI port '{port}': {error}"))
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn log_stale_response(actual_id: &str, expected_id: &str) {
    eprintln!(
        "{}",
        json!({
            "timestamp": unix_timestamp_ms(),
            "event": "bridge.stale_response",
            "actual_request_id": actual_id,
            "expected_request_id": expected_id,
            "source": "midi_bridge"
        })
    );
}

#[derive(Default)]
struct SysexFramer {
    frame: Vec<u8>,
    receiving: bool,
}

impl SysexFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut completed = Vec::new();
        for byte in bytes.iter().copied() {
            if byte == 0xF0 {
                self.frame.clear();
                self.frame.push(byte);
                self.receiving = true;
                continue;
            }
            if !self.receiving {
                continue;
            }

            self.frame.push(byte);
            if self.frame.len() > MAX_SYSEX_BYTES {
                self.frame.clear();
                self.receiving = false;
                continue;
            }
            if byte == 0xF7 {
                self.receiving = false;
                if self.frame.starts_with(&SYSEX_HEADER) {
                    completed.push(std::mem::take(&mut self.frame));
                } else {
                    self.frame.clear();
                }
            }
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::protocol::{BridgeResponse, ResponseMessageType};

    #[test]
    fn sysex_codec_round_trips_unicode_json() {
        let response = BridgeResponse {
            version: 1,
            id: "リクエスト-1".into(),
            message_type: ResponseMessageType::Response,
            result: json!({"track": "ボーカル"}),
        };
        let frame = encode_sysex(&response).unwrap();
        assert!(
            frame
                .iter()
                .all(|byte| *byte < 0x80 || *byte == 0xF0 || *byte == 0xF7)
        );

        let decoded = decode_sysex(&frame).unwrap();
        let BridgeIncoming::Response { id, result, .. } = decoded else {
            panic!("expected response");
        };
        assert_eq!(id, "リクエスト-1");
        assert_eq!(result["track"], "ボーカル");
    }

    #[test]
    fn framer_reassembles_split_sysex_and_ignores_other_midi() {
        let response = BridgeResponse::new("1".into(), json!({"playing": true}));
        let frame = encode_sysex(&response).unwrap();
        let split = frame.len() / 2;
        let mut framer = SysexFramer::default();

        assert!(framer.push(&[0x90, 60, 127]).is_empty());
        assert!(framer.push(&frame[..split]).is_empty());
        let completed = framer.push(&frame[split..]);
        assert_eq!(completed, vec![frame]);
    }

    #[test]
    fn decoder_rejects_non_nibble_payload() {
        let mut frame = SYSEX_HEADER.to_vec();
        frame.extend_from_slice(&[0x10, 0x00, 0xF7]);
        let error = decode_sysex(&frame).unwrap_err();
        assert_eq!(error.code, ErrorCode::ProtocolError);
    }

    #[test]
    fn port_selector_prefers_exact_match_and_rejects_ambiguity() {
        let first = 1;
        let second = 2;
        let exact = select_port(
            "Port",
            [(&first, "Port".into()), (&second, "Port Extended".into())].into_iter(),
            "input",
        )
        .unwrap();
        assert_eq!(exact, first);

        let error = select_port(
            "port",
            [(&first, "Port A".into()), (&second, "Port B".into())].into_iter(),
            "input",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }
}
