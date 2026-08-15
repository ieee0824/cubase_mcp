use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bridge::CubaseBridge;
use crate::protocol::{
    BRIDGE_PROTOCOL_VERSION, BridgeError, BridgeIncoming, BridgeRequest, ErrorCode,
};

pub const DEFAULT_TO_CUBASE_PORT: &str = "Cubase MCP To Cubase";
pub const DEFAULT_FROM_CUBASE_PORT: &str = "Cubase MCP From Cubase";
pub const MIN_MIDI_TIMEOUT_MS: u64 = 500;

const SYSEX_HEADER: [u8; 7] = [0xF0, 0x7D, b'C', b'M', b'C', b'P', 0x01];
const MIDI_TRANSPORT_VERSION: u32 = 1;
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_SYSEX_BYTES: usize = SYSEX_HEADER.len() + MAX_JSON_BYTES * 2 + 1;
const MAX_MESSAGES_BEFORE_RESPONSE: usize = 1024;
const MIDI_QUEUE_CAPACITY: usize = 64;
const MAX_DRAIN_MESSAGES: usize = 256;
const MAX_TRACKED_INSTANCES: usize = 16;
const MAX_INSTANCE_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MidiPortListing {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

struct MidiCallbackState {
    sender: SyncSender<Vec<u8>>,
    framer: SysexFramer,
    queue_overflowed: Arc<AtomicBool>,
}

struct MidiState {
    output: MidiOutputConnection,
    _input: MidiInputConnection<MidiCallbackState>,
    receiver: Receiver<Vec<u8>>,
    active_instances: HashSet<String>,
    target_instance_id: Option<String>,
}

#[derive(Serialize)]
struct MidiRequestEnvelope<'a> {
    midi_transport_version: u32,
    target_instance_id: Option<&'a str>,
    message: &'a BridgeRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MidiIncomingEnvelope {
    midi_transport_version: u32,
    source_instance_id: String,
    message: BridgeIncoming,
}

#[derive(Default)]
struct DiscoveryRound {
    candidates: HashSet<String>,
    announcements: HashSet<String>,
    errors: Vec<(String, BridgeError)>,
    observed_sources: HashSet<String>,
}

impl DiscoveryRound {
    fn observe_source(&mut self, source_instance_id: &str) -> Result<(), BridgeError> {
        remember_active_instance(&mut self.observed_sources, source_instance_id)
    }

    fn record_success(&mut self, source_instance_id: String) -> Result<(), BridgeError> {
        self.observe_source(&source_instance_id)?;
        self.errors
            .retain(|(source, _)| source != &source_instance_id);
        self.candidates.insert(source_instance_id);
        Ok(())
    }

    fn record_error(
        &mut self,
        source_instance_id: String,
        error: BridgeError,
    ) -> Result<(), BridgeError> {
        self.observe_source(&source_instance_id)?;
        if self.candidates.contains(&source_instance_id) {
            return Ok(());
        }
        self.announcements.remove(&source_instance_id);
        if let Some((_, existing)) = self
            .errors
            .iter_mut()
            .find(|(source, _)| source == &source_instance_id)
        {
            *existing = error;
        } else {
            self.errors.push((source_instance_id, error));
        }
        Ok(())
    }

    fn record_announcement(&mut self, source_instance_id: String) -> Result<(), BridgeError> {
        self.observe_source(&source_instance_id)?;
        if self
            .errors
            .iter()
            .any(|(source, _)| source == &source_instance_id)
        {
            return Ok(());
        }
        self.announcements.insert(source_instance_id);
        Ok(())
    }

    fn record_disconnect(&mut self, source_instance_id: &str) {
        self.candidates.remove(source_instance_id);
        self.announcements.remove(source_instance_id);
    }

    fn selection(&self) -> Result<String, BridgeError> {
        let observed_active_count = self.candidates.union(&self.announcements).take(2).count();
        if observed_active_count > 1 {
            return Err(multiple_instances_error());
        }
        match self.candidates.len() {
            0 => Err(self
                .errors
                .first()
                .map(|(_, error)| error.clone())
                .unwrap_or_else(|| {
                    BridgeError::not_connected(
                        "No Cubase MIDI Remote instance responded to discovery",
                    )
                })),
            1 => Ok(self
                .candidates
                .iter()
                .next()
                .expect("a set with one item has a first item")
                .clone()),
            _ => Err(multiple_instances_error()),
        }
    }
}

/// Bridge Protocol over a 7-bit-safe MIDI SysEx envelope.
///
/// On Unix platforms the default constructor creates two virtual ports, so
/// Cubase can attach without an external loopback MIDI driver.
pub struct MidiBridge {
    state: Mutex<MidiState>,
    connected: AtomicBool,
    queue_overflowed: Arc<AtomicBool>,
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
        let queue_overflowed = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
        let input = input
            .create_virtual(
                DEFAULT_FROM_CUBASE_PORT,
                receive_midi,
                MidiCallbackState {
                    sender,
                    framer: SysexFramer::default(),
                    queue_overflowed: Arc::clone(&queue_overflowed),
                },
            )
            .map_err(|error| midi_connect_error(DEFAULT_FROM_CUBASE_PORT, error))?;

        Ok(Self {
            state: Mutex::new(MidiState {
                output,
                _input: input,
                receiver,
                active_instances: HashSet::new(),
                target_instance_id: None,
            }),
            connected: AtomicBool::new(false),
            queue_overflowed,
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

        let queue_overflowed = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
        let input = input
            .connect(
                &input_port,
                "cubase-mcp-input-connection",
                receive_midi,
                MidiCallbackState {
                    sender,
                    framer: SysexFramer::default(),
                    queue_overflowed: Arc::clone(&queue_overflowed),
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
                active_instances: HashSet::new(),
                target_instance_id: None,
            }),
            connected: AtomicBool::new(false),
            queue_overflowed,
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

    fn invalidate_instance_selection(&self, state: &mut MidiState) {
        state.target_instance_id = None;
        state.active_instances.clear();
        self.connected.store(false, Ordering::Release);
    }

    fn queue_overflow_error(&self, state: &mut MidiState) -> BridgeError {
        self.invalidate_instance_selection(state);
        BridgeError::new(
            ErrorCode::Busy,
            "MIDI receive queue overflowed; instance selection was discarded and discovery must be retried",
        )
    }

    fn fail_if_queue_overflowed(&self, state: &mut MidiState) -> Result<(), BridgeError> {
        if self.queue_overflowed.load(Ordering::Acquire) {
            Err(self.queue_overflow_error(state))
        } else {
            Ok(())
        }
    }

    fn receive_until_response(
        &self,
        state: &mut MidiState,
        expected_id: &str,
        expected_instance_id: &str,
        deadline: Instant,
    ) -> Result<Value, BridgeError> {
        let mut message_count = 0;
        loop {
            self.fail_if_queue_overflowed(state)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(response_timeout_error());
            }

            let frame = match state.receiver.recv_timeout(remaining) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => return Err(response_timeout_error()),
                Err(RecvTimeoutError::Disconnected) => {
                    self.connected.store(false, Ordering::Release);
                    return Err(BridgeError::internal(
                        "MIDI receive callback stopped unexpectedly",
                    ));
                }
            };
            self.fail_if_queue_overflowed(state)?;
            message_count += 1;
            if message_count > MAX_MESSAGES_BEFORE_RESPONSE {
                return Err(BridgeError::protocol(
                    "Too many MIDI bridge messages arrived before the response",
                ));
            }

            let incoming = decode_sysex(&frame)?;
            let source_instance_id = incoming.source_instance_id;
            match incoming.message {
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
                    if source_instance_id != expected_instance_id {
                        log_unexpected_instance(&source_instance_id, expected_instance_id);
                        continue;
                    }
                    remember_active_instance(&mut state.active_instances, &source_instance_id)?;
                    state.target_instance_id = Some(source_instance_id);
                    self.connected.store(true, Ordering::Release);
                    return Ok(result);
                }
                BridgeIncoming::Error { version, id, error } => {
                    validate_version(version)?;
                    if id != expected_id {
                        log_stale_response(&id, expected_id);
                        continue;
                    }
                    if source_instance_id != expected_instance_id {
                        log_unexpected_instance(&source_instance_id, expected_instance_id);
                        continue;
                    }
                    remember_active_instance(&mut state.active_instances, &source_instance_id)?;
                    if error.message.is_empty() {
                        return Err(BridgeError::protocol(
                            "Bridge error message must not be empty",
                        ));
                    }
                    state.target_instance_id = Some(source_instance_id);
                    self.connected.store(true, Ordering::Release);
                    return Err(error);
                }
                BridgeIncoming::Event {
                    version,
                    event,
                    data,
                } => {
                    validate_version(version)?;
                    self.handle_event(state, &source_instance_id, &event, &data)?;
                    if state.target_instance_id.as_deref() != Some(expected_instance_id) {
                        return Err(BridgeError::not_connected(
                            "The selected Cubase MIDI Remote instance disconnected",
                        ));
                    }
                }
            }
        }
    }

    fn drain_events(&self, state: &mut MidiState) -> Result<(), BridgeError> {
        for _ in 0..MAX_DRAIN_MESSAGES {
            match state.receiver.try_recv() {
                Ok(frame) => {
                    let incoming = decode_sysex(&frame)?;
                    let source_instance_id = incoming.source_instance_id;
                    match incoming.message {
                        BridgeIncoming::Event {
                            version,
                            event,
                            data,
                        } => {
                            validate_version(version)?;
                            self.handle_event(state, &source_instance_id, &event, &data)?;
                        }
                        BridgeIncoming::Response { version, id, .. } => {
                            validate_version(version)?;
                            remember_active_instance(
                                &mut state.active_instances,
                                &source_instance_id,
                            )?;
                            log_stale_response(&id, "none");
                        }
                        BridgeIncoming::Error { version, id, .. } => {
                            validate_version(version)?;
                            log_stale_response(&id, "none");
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    if self.queue_overflowed.swap(false, Ordering::AcqRel) {
                        return Err(self.queue_overflow_error(state));
                    }
                    return Ok(());
                }
                Err(TryRecvError::Disconnected) => {
                    self.connected.store(false, Ordering::Release);
                    return Err(BridgeError::internal(
                        "MIDI receive callback stopped unexpectedly",
                    ));
                }
            }
        }

        self.queue_overflowed.store(true, Ordering::Release);
        Err(self.queue_overflow_error(state))
    }

    fn handle_event(
        &self,
        state: &mut MidiState,
        source_instance_id: &str,
        event: &str,
        data: &Value,
    ) -> Result<(), BridgeError> {
        if event.is_empty() || !data.is_object() {
            return Err(BridgeError::protocol(
                "Bridge event requires a non-empty name and object data",
            ));
        }

        if event == "connection.changed" {
            let connected = data
                .get("connected")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    BridgeError::protocol(
                        "connection.changed event requires boolean data.connected",
                    )
                })?;
            if connected {
                remember_active_instance(&mut state.active_instances, source_instance_id)?;
            } else {
                state.active_instances.remove(source_instance_id);
                if state.target_instance_id.as_deref() == Some(source_instance_id) {
                    state.target_instance_id = None;
                }
            }
            let has_live_instance = state.target_instance_id.as_ref().map_or_else(
                || !state.active_instances.is_empty(),
                |target| state.active_instances.contains(target),
            );
            self.connected.store(has_live_instance, Ordering::Release);
        } else {
            if state
                .target_instance_id
                .as_deref()
                .is_some_and(|target| target != source_instance_id)
            {
                return Ok(());
            }
            remember_active_instance(&mut state.active_instances, source_instance_id)?;
            self.connected.store(true, Ordering::Release);
        }
        let timestamp = unix_timestamp_ms();
        eprintln!(
            "{}",
            json!({
                "timestamp": timestamp,
                "event": event,
                "source_instance_id": source_instance_id,
                "source": "midi_bridge"
            })
        );
        Ok(())
    }

    fn ensure_target(
        &self,
        state: &mut MidiState,
        request_id: &str,
        discovery_timeout: Duration,
    ) -> Result<String, BridgeError> {
        if let Some(target) = state.target_instance_id.clone() {
            return Ok(target);
        }

        self.discover_target(state, request_id, discovery_timeout)
    }

    fn discover_target(
        &self,
        state: &mut MidiState,
        request_id: &str,
        timeout: Duration,
    ) -> Result<String, BridgeError> {
        self.fail_if_queue_overflowed(state)?;
        state.target_instance_id = None;
        state.active_instances.clear();

        let discovery_id = format!("{request_id}-midi-discovery");
        let discovery = BridgeRequest::new(discovery_id.clone(), "system.discover", json!({}));
        let frame = encode_request_sysex(&discovery, None)?;
        state.output.send(&frame).map_err(|error| {
            self.connected.store(false, Ordering::Release);
            BridgeError::not_connected(format!(
                "Could not send MIDI discovery SysEx to Cubase: {error}"
            ))
        })?;

        let discovery_window = discovery_window(timeout);
        let deadline = Instant::now() + discovery_window;
        let mut round = DiscoveryRound::default();
        let mut message_count = 0;
        loop {
            self.fail_if_queue_overflowed(state)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }

            let frame = match state.receiver.recv_timeout(remaining) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    self.connected.store(false, Ordering::Release);
                    return Err(BridgeError::internal(
                        "MIDI receive callback stopped unexpectedly",
                    ));
                }
            };
            self.fail_if_queue_overflowed(state)?;
            message_count += 1;
            if message_count > MAX_MESSAGES_BEFORE_RESPONSE {
                return Err(BridgeError::protocol(
                    "Too many MIDI bridge messages arrived during instance discovery",
                ));
            }

            let incoming = decode_sysex(&frame)?;
            let source_instance_id = incoming.source_instance_id;
            match incoming.message {
                BridgeIncoming::Response {
                    version,
                    id,
                    result,
                } => {
                    validate_version(version)?;
                    if id != discovery_id {
                        log_stale_response(&id, &discovery_id);
                        continue;
                    }
                    let reported_instance_id = result
                        .get("instance_id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            BridgeError::protocol(
                                "MIDI discovery response requires a non-empty result.instance_id",
                            )
                        })?;
                    if reported_instance_id != source_instance_id {
                        return Err(BridgeError::protocol(
                            "MIDI discovery response instance id does not match its transport source",
                        ));
                    }
                    round.record_success(source_instance_id)?;
                    if round.candidates.len() > 1 {
                        break;
                    }
                }
                BridgeIncoming::Error { version, id, error } => {
                    validate_version(version)?;
                    if id == discovery_id {
                        if error.message.is_empty() {
                            return Err(BridgeError::protocol(
                                "Bridge error message must not be empty",
                            ));
                        }
                        round.record_error(source_instance_id, error)?;
                        continue;
                    }
                    log_stale_response(&id, &discovery_id);
                }
                BridgeIncoming::Event {
                    version,
                    event,
                    data,
                } => {
                    validate_version(version)?;
                    let connection_state = if event == "connection.changed" {
                        data.get("connected").and_then(Value::as_bool)
                    } else {
                        None
                    };
                    self.handle_event(state, &source_instance_id, &event, &data)?;
                    match connection_state {
                        Some(true) => round.record_announcement(source_instance_id)?,
                        Some(false) => round.record_disconnect(&source_instance_id),
                        None => {}
                    }
                }
            }
        }

        self.fail_if_queue_overflowed(state)?;
        state.active_instances = round.candidates.clone();
        let selection = round.selection();
        match &selection {
            Ok(target) => {
                state.target_instance_id = Some(target.clone());
                self.connected.store(true, Ordering::Release);
            }
            Err(_) => {
                state.target_instance_id = None;
                self.connected
                    .store(!state.active_instances.is_empty(), Ordering::Release);
            }
        }
        selection
    }
}

impl CubaseBridge for MidiBridge {
    fn call(&self, request: &BridgeRequest, timeout: Duration) -> Result<Value, BridgeError> {
        request.validate()?;
        if timeout.is_zero() {
            return Err(BridgeError::new(
                ErrorCode::InvalidArgument,
                "MIDI bridge timeout must be greater than zero",
            ));
        }
        let mut state = self.lock_state()?;
        self.drain_events(&mut state)?;
        let target_instance_id = self.ensure_target(&mut state, &request.id, timeout)?;

        // Process frames that arrived at the discovery boundary before allowing
        // a state-changing request to use the selected instance.
        self.drain_events(&mut state)?;
        self.fail_if_queue_overflowed(&mut state)?;
        if state.target_instance_id.as_deref() != Some(&target_instance_id) {
            return Err(BridgeError::not_connected(
                "The selected Cubase MIDI Remote instance disconnected before the request",
            ));
        }
        if state.active_instances.len() > 1 {
            state.target_instance_id = None;
            return Err(multiple_instances_error());
        }

        let frame = encode_request_sysex(request, Some(&target_instance_id))?;
        if let Err(error) = state.output.send(&frame) {
            self.invalidate_instance_selection(&mut state);
            return Err(BridgeError::not_connected(format!(
                "Could not send MIDI SysEx to Cubase: {error}"
            )));
        }
        let response_deadline = Instant::now() + timeout;
        let result = self.receive_until_response(
            &mut state,
            &request.id,
            &target_instance_id,
            response_deadline,
        );
        if matches!(&result, Err(error) if error.code == ErrorCode::Timeout) {
            let MidiState {
                active_instances,
                target_instance_id: selected_target,
                ..
            } = &mut *state;
            expire_instance(active_instances, selected_target, &target_instance_id);
            // Preserve the last-known connection hint for this one timeout. The
            // next call cannot reuse the stale id and must complete discovery.
        }
        result
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
}

fn receive_midi(_timestamp: u64, message: &[u8], state: &mut MidiCallbackState) {
    for frame in state.framer.push(message) {
        if matches!(
            state.sender.try_send(frame),
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_))
        ) {
            state.queue_overflowed.store(true, Ordering::Release);
        }
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

fn encode_request_sysex(
    request: &BridgeRequest,
    target_instance_id: Option<&str>,
) -> Result<Vec<u8>, BridgeError> {
    encode_sysex(&MidiRequestEnvelope {
        midi_transport_version: MIDI_TRANSPORT_VERSION,
        target_instance_id,
        message: request,
    })
}

fn decode_sysex(frame: &[u8]) -> Result<MidiIncomingEnvelope, BridgeError> {
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

    let envelope: MidiIncomingEnvelope = serde_json::from_slice(&json_bytes)
        .map_err(|error| BridgeError::protocol(format!("Invalid MIDI bridge JSON: {error}")))?;
    if envelope.midi_transport_version != MIDI_TRANSPORT_VERSION {
        return Err(BridgeError::protocol(format!(
            "Unsupported MIDI transport envelope version {}",
            envelope.midi_transport_version
        )));
    }
    if envelope.source_instance_id.is_empty()
        || envelope.source_instance_id.len() > MAX_INSTANCE_ID_BYTES
    {
        return Err(BridgeError::protocol(
            "MIDI transport source_instance_id must contain 1 to 128 UTF-8 bytes",
        ));
    }
    Ok(envelope)
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

fn response_timeout_error() -> BridgeError {
    BridgeError::new(
        ErrorCode::Timeout,
        "Cubase MIDI Remote did not respond before the request timeout",
    )
}

fn discovery_window(request_timeout: Duration) -> Duration {
    request_timeout.max(Duration::from_millis(MIN_MIDI_TIMEOUT_MS))
}

fn multiple_instances_error() -> BridgeError {
    BridgeError::new(
        ErrorCode::Busy,
        "Multiple Cubase MIDI Remote instances are active; close extra Cubase instances and retry",
    )
}

fn remember_active_instance(
    active_instances: &mut HashSet<String>,
    instance_id: &str,
) -> Result<(), BridgeError> {
    if active_instances.contains(instance_id) {
        return Ok(());
    }
    if active_instances.len() >= MAX_TRACKED_INSTANCES {
        return Err(BridgeError::new(
            ErrorCode::Busy,
            "Too many Cubase MIDI Remote instances are sending bridge messages",
        ));
    }
    active_instances.insert(instance_id.to_owned());
    Ok(())
}

fn expire_instance(
    active_instances: &mut HashSet<String>,
    target_instance_id: &mut Option<String>,
    expired_instance_id: &str,
) {
    active_instances.remove(expired_instance_id);
    if target_instance_id.as_deref() == Some(expired_instance_id) {
        *target_instance_id = None;
    }
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

fn log_unexpected_instance(actual_id: &str, expected_id: &str) {
    eprintln!(
        "{}",
        json!({
            "timestamp": unix_timestamp_ms(),
            "event": "bridge.unexpected_instance",
            "actual_instance_id": actual_id,
            "expected_instance_id": expected_id,
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

    #[test]
    fn sysex_codec_round_trips_unicode_json() {
        let response = MidiIncomingEnvelope {
            midi_transport_version: 1,
            source_instance_id: "cubase-日本語".into(),
            message: BridgeIncoming::Response {
                version: 1,
                id: "リクエスト-1".into(),
                result: json!({"track": "ボーカル"}),
            },
        };
        let frame = encode_sysex(&response).unwrap();
        assert!(
            frame
                .iter()
                .all(|byte| *byte < 0x80 || *byte == 0xF0 || *byte == 0xF7)
        );

        let decoded = decode_sysex(&frame).unwrap();
        assert_eq!(decoded.source_instance_id, "cubase-日本語");
        let BridgeIncoming::Response { id, result, .. } = decoded.message else {
            panic!("expected response");
        };
        assert_eq!(id, "リクエスト-1");
        assert_eq!(result["track"], "ボーカル");
    }

    #[test]
    fn framer_reassembles_split_sysex_and_ignores_other_midi() {
        let response = MidiIncomingEnvelope {
            midi_transport_version: 1,
            source_instance_id: "cubase-1".into(),
            message: BridgeIncoming::Response {
                version: 1,
                id: "1".into(),
                result: json!({"playing": true}),
            },
        };
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

    #[test]
    fn request_envelope_scopes_message_to_one_instance() {
        let request = BridgeRequest::new("req-1".into(), "transport.play", json!({}));
        let envelope = MidiRequestEnvelope {
            midi_transport_version: MIDI_TRANSPORT_VERSION,
            target_instance_id: Some("cubase-1"),
            message: &request,
        };
        let value = serde_json::to_value(envelope).unwrap();

        assert_eq!(value["target_instance_id"], "cubase-1");
        assert_eq!(value["message"]["method"], "transport.play");
    }

    #[test]
    fn callback_records_bounded_queue_overflow() {
        let response = MidiIncomingEnvelope {
            midi_transport_version: 1,
            source_instance_id: "cubase-1".into(),
            message: BridgeIncoming::Response {
                version: 1,
                id: "1".into(),
                result: json!({}),
            },
        };
        let frame = encode_sysex(&response).unwrap();
        let (sender, receiver) = mpsc::sync_channel(2);
        let queue_overflowed = Arc::new(AtomicBool::new(false));
        let mut callback = MidiCallbackState {
            sender,
            framer: SysexFramer::default(),
            queue_overflowed: Arc::clone(&queue_overflowed),
        };

        for _ in 0..10 {
            receive_midi(0, &frame, &mut callback);
        }

        assert_eq!(receiver.try_iter().count(), 2);
        assert!(queue_overflowed.load(Ordering::Acquire));
    }

    #[test]
    fn response_deadline_uses_timeout_error_without_claiming_disconnect() {
        assert_eq!(response_timeout_error().code, ErrorCode::Timeout);
    }

    #[test]
    fn discovery_has_its_own_minimum_time_budget() {
        assert_eq!(
            discovery_window(Duration::from_millis(50)),
            Duration::from_millis(MIN_MIDI_TIMEOUT_MS)
        );
        assert_eq!(
            discovery_window(Duration::from_secs(2)),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn decoder_rejects_oversized_instance_id() {
        let response = MidiIncomingEnvelope {
            midi_transport_version: 1,
            source_instance_id: "x".repeat(MAX_INSTANCE_ID_BYTES + 1),
            message: BridgeIncoming::Response {
                version: 1,
                id: "1".into(),
                result: json!({}),
            },
        };
        let frame = encode_sysex(&response).unwrap();
        let error = decode_sysex(&frame).unwrap_err();

        assert_eq!(error.code, ErrorCode::ProtocolError);
    }

    #[test]
    fn active_instance_tracking_is_bounded() {
        let mut active = HashSet::new();
        for index in 0..MAX_TRACKED_INSTANCES {
            remember_active_instance(&mut active, &format!("cubase-{index}")).unwrap();
        }

        let error = remember_active_instance(&mut active, "one-too-many").unwrap_err();
        assert_eq!(error.code, ErrorCode::Busy);
        assert_eq!(active.len(), MAX_TRACKED_INSTANCES);
    }

    #[test]
    fn discovery_errors_do_not_hide_a_usable_instance() {
        let mut round = DiscoveryRound::default();
        round.record_announcement("inactive".into()).unwrap();
        round
            .record_error(
                "inactive".into(),
                BridgeError::not_connected("mapping is inactive"),
            )
            .unwrap();
        round.record_success("usable".into()).unwrap();

        assert_eq!(round.selection().unwrap(), "usable");
    }

    #[test]
    fn discovery_rejects_multiple_instances_even_with_one_response() {
        let mut round = DiscoveryRound::default();
        round.record_success("first".into()).unwrap();
        round.record_announcement("second".into()).unwrap();

        assert_eq!(round.selection().unwrap_err().code, ErrorCode::Busy);
    }

    #[test]
    fn timed_out_instance_is_expired_before_rediscovery() {
        let mut active = HashSet::from(["stale".to_owned(), "other".to_owned()]);
        let mut target = Some("stale".to_owned());

        expire_instance(&mut active, &mut target, "stale");

        assert!(!active.contains("stale"));
        assert!(active.contains("other"));
        assert!(target.is_none());
    }
}
