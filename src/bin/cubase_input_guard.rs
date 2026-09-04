use std::io::{self, BufRead, Write};
use std::process::ExitCode;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const GUARD_PROTOCOL_VERSION: u32 = 5;
const GUARD_COVERAGE: &str = "action_windows";
const GUARD_PRIVACY: &str = "counts_and_held_state_boolean";
const GUARD_POLICY: &str = "consequential_input_only";
const MAX_COMMAND_BYTES: usize = 512;
const MAX_ACTION_ID_BYTES: usize = 128;
#[cfg(target_os = "macos")]
const COUNTER_READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
struct InputCounters {
    mouse_moved: u32,
    left_mouse_down: u32,
    left_mouse_up: u32,
    right_mouse_down: u32,
    right_mouse_up: u32,
    other_mouse_down: u32,
    other_mouse_up: u32,
    left_mouse_dragged: u32,
    right_mouse_dragged: u32,
    other_mouse_dragged: u32,
    key_down: u32,
    key_up: u32,
    flags_changed: u32,
    scroll_wheel: u32,
    tablet_pointer: u32,
    tablet_proximity: u32,
}

impl InputCounters {
    fn delta_since(self, earlier: Self) -> Self {
        Self {
            mouse_moved: self.mouse_moved.wrapping_sub(earlier.mouse_moved),
            left_mouse_down: self.left_mouse_down.wrapping_sub(earlier.left_mouse_down),
            left_mouse_up: self.left_mouse_up.wrapping_sub(earlier.left_mouse_up),
            right_mouse_down: self.right_mouse_down.wrapping_sub(earlier.right_mouse_down),
            right_mouse_up: self.right_mouse_up.wrapping_sub(earlier.right_mouse_up),
            other_mouse_down: self.other_mouse_down.wrapping_sub(earlier.other_mouse_down),
            other_mouse_up: self.other_mouse_up.wrapping_sub(earlier.other_mouse_up),
            left_mouse_dragged: self
                .left_mouse_dragged
                .wrapping_sub(earlier.left_mouse_dragged),
            right_mouse_dragged: self
                .right_mouse_dragged
                .wrapping_sub(earlier.right_mouse_dragged),
            other_mouse_dragged: self
                .other_mouse_dragged
                .wrapping_sub(earlier.other_mouse_dragged),
            key_down: self.key_down.wrapping_sub(earlier.key_down),
            key_up: self.key_up.wrapping_sub(earlier.key_up),
            flags_changed: self.flags_changed.wrapping_sub(earlier.flags_changed),
            scroll_wheel: self.scroll_wheel.wrapping_sub(earlier.scroll_wheel),
            tablet_pointer: self.tablet_pointer.wrapping_sub(earlier.tablet_pointer),
            tablet_proximity: self.tablet_proximity.wrapping_sub(earlier.tablet_proximity),
        }
    }

    fn any(self) -> bool {
        self != Self::default()
    }

    fn any_consequential(self) -> bool {
        Self {
            mouse_moved: 0,
            ..self
        }
        .any()
    }
}

#[cfg(any(target_os = "macos", test))]
fn input_changed_during_sample(aggregate_before: u32, aggregate_after: u32) -> bool {
    aggregate_before != aggregate_after
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SampleTiming {
    started_at_unix_ms: u64,
    completed_at_unix_ms: u64,
}

impl SampleTiming {
    fn decorate_record(self, mut value: Value) -> Value {
        let object = value
            .as_object_mut()
            .expect("guard records are always JSON objects");
        object.insert(
            "sample_started_at_unix_ms".into(),
            Value::from(self.started_at_unix_ms),
        );
        object.insert(
            "sample_completed_at_unix_ms".into(),
            Value::from(self.completed_at_unix_ms),
        );
        value
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TimedInputCounters {
    counters: InputCounters,
    timing: SampleTiming,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SampleCommandContext {
    command: &'static str,
    action_id: String,
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Arm { action_id: String },
    Check { action_id: String },
    Cancel { action_id: String },
    Reject { action_id: String },
    Ping,
    Finish,
}

impl Command {
    fn requires_counter_sample(&self) -> bool {
        matches!(self, Self::Arm { .. } | Self::Check { .. })
    }

    fn sample_context(&self) -> Option<SampleCommandContext> {
        match self {
            Self::Arm { action_id } => Some(SampleCommandContext {
                command: "arm",
                action_id: action_id.clone(),
            }),
            Self::Check { action_id } => Some(SampleCommandContext {
                command: "check",
                action_id: action_id.clone(),
            }),
            Self::Cancel { .. } | Self::Reject { .. } | Self::Ping | Self::Finish => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    command: String,
    action_id: Option<String>,
}

#[derive(Debug)]
struct ArmedAction {
    action_id: String,
    counters: InputCounters,
}

#[derive(Debug)]
struct GuardState {
    armed: Option<ArmedAction>,
    last_clean_result_action_id: Option<String>,
    interference_latched: bool,
    session_aborted: bool,
}

impl GuardState {
    fn new() -> Self {
        Self {
            armed: None,
            last_clean_result_action_id: None,
            interference_latched: false,
            session_aborted: false,
        }
    }

    fn handle(&mut self, command: Command, current: InputCounters) -> Result<Value, GuardError> {
        match command {
            Command::Arm { action_id } => {
                validate_action_id(&action_id)?;
                if self.session_aborted {
                    return Err(GuardError::new(
                        "SESSION_ABORTED",
                        "an earlier action was cancelled or rejected in this guard session",
                    ));
                }
                if self.interference_latched {
                    return Err(GuardError::new(
                        "INTERFERENCE_LATCHED",
                        "consequential HID input was detected during an earlier armed action window",
                    ));
                }
                if let Some(armed) = &self.armed {
                    return Err(GuardError::new(
                        "ALREADY_ARMED",
                        format!("input guard is already armed for {}", armed.action_id),
                    ));
                }
                self.last_clean_result_action_id = None;
                self.armed = Some(ArmedAction {
                    action_id: action_id.clone(),
                    counters: current,
                });
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "armed",
                    "action_id": action_id,
                    "source": "hid_system_state",
                    "coverage": GUARD_COVERAGE,
                    "policy": GUARD_POLICY
                }))
            }
            Command::Check { action_id } => {
                validate_action_id(&action_id)?;
                let armed = self
                    .armed
                    .as_ref()
                    .ok_or_else(|| GuardError::new("NOT_ARMED", "input guard is not armed"))?;
                if armed.action_id != action_id {
                    return Err(GuardError::new(
                        "ACTION_ID_MISMATCH",
                        "check action_id does not match the armed action",
                    ));
                }
                let deltas = current.delta_since(armed.counters);
                let interference_detected = deltas.any_consequential();
                self.armed = None;
                self.interference_latched |= interference_detected;
                self.last_clean_result_action_id = if interference_detected {
                    None
                } else {
                    Some(action_id.clone())
                };
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "result",
                    "action_id": action_id,
                    "source": "hid_system_state",
                    "coverage": GUARD_COVERAGE,
                    "policy": GUARD_POLICY,
                    "interference_detected": interference_detected,
                    "deltas": deltas
                }))
            }
            Command::Cancel { action_id } => {
                validate_action_id(&action_id)?;
                let armed = self
                    .armed
                    .as_ref()
                    .ok_or_else(|| GuardError::new("NOT_ARMED", "input guard is not armed"))?;
                if armed.action_id != action_id {
                    return Err(GuardError::new(
                        "ACTION_ID_MISMATCH",
                        "cancel action_id does not match the armed action",
                    ));
                }
                self.armed = None;
                self.last_clean_result_action_id = None;
                self.session_aborted = true;
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "cancelled",
                    "action_id": action_id,
                    "coverage": GUARD_COVERAGE,
                    "policy": GUARD_POLICY,
                    "session_aborted": true
                }))
            }
            Command::Reject { action_id } => {
                validate_action_id(&action_id)?;
                if self.armed.is_some() {
                    return Err(GuardError::new(
                        "REJECT_WHILE_ARMED",
                        "check the armed action before rejecting its UI postcondition",
                    ));
                }
                if self.interference_latched {
                    return Err(GuardError::new(
                        "INTERFERENCE_LATCHED",
                        "consequential HID input was detected during an earlier armed action window",
                    ));
                }
                if self.session_aborted {
                    return Err(GuardError::new(
                        "SESSION_ABORTED",
                        "this guard session was already aborted",
                    ));
                }
                let checked_action_id =
                    self.last_clean_result_action_id.as_deref().ok_or_else(|| {
                        GuardError::new(
                            "NO_CLEAN_RESULT",
                            "reject requires the most recent action to have a clean check result",
                        )
                    })?;
                if checked_action_id != action_id {
                    return Err(GuardError::new(
                        "ACTION_ID_MISMATCH",
                        "reject action_id does not match the most recent clean result",
                    ));
                }
                self.last_clean_result_action_id = None;
                self.session_aborted = true;
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "rejected",
                    "action_id": action_id,
                    "reason": "postcondition_failed",
                    "after_clean_result": true,
                    "coverage": GUARD_COVERAGE,
                    "policy": GUARD_POLICY,
                    "session_aborted": true
                }))
            }
            Command::Ping => Ok(json!({
                "version": GUARD_PROTOCOL_VERSION,
                "type": "pong",
                "source": "hid_system_state",
                "coverage": GUARD_COVERAGE,
                "policy": GUARD_POLICY,
                "armed": self.armed.is_some(),
                "interference_latched": self.interference_latched,
                "session_aborted": self.session_aborted
            })),
            Command::Finish => {
                if let Some(armed) = &self.armed {
                    return Err(GuardError::new(
                        "FINISH_WHILE_ARMED",
                        format!("input guard is still armed for {}", armed.action_id),
                    ));
                }
                if self.interference_latched {
                    return Err(GuardError::new(
                        "INTERFERENCE_LATCHED",
                        "consequential HID input was detected during an armed action window",
                    ));
                }
                if self.session_aborted {
                    return Err(GuardError::new(
                        "SESSION_ABORTED",
                        "an earlier action was cancelled or rejected in this guard session",
                    ));
                }
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "finished",
                    "source": "hid_system_state",
                    "coverage": GUARD_COVERAGE,
                    "policy": GUARD_POLICY,
                    "interference_detected": false
                }))
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct GuardError {
    code: &'static str,
    message: String,
    sample_timing: Option<SampleTiming>,
    sample_command: Option<SampleCommandContext>,
}

impl GuardError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            sample_timing: None,
            sample_command: None,
        }
    }

    fn with_sample_timing(mut self, timing: SampleTiming) -> Self {
        self.sample_timing = Some(timing);
        self
    }

    fn with_sample_command(mut self, command: SampleCommandContext) -> Self {
        self.sample_command = Some(command);
        self
    }

    fn as_json(&self) -> Value {
        let mut value = json!({
            "version": GUARD_PROTOCOL_VERSION,
            "type": "error",
            "coverage": GUARD_COVERAGE,
            "policy": GUARD_POLICY,
            "error": {
                "code": self.code,
                "message": self.message
            }
        });
        if let Some(timing) = self.sample_timing {
            value = timing.decorate_record(value);
        }
        if let Some(command) = &self.sample_command {
            let object = value
                .as_object_mut()
                .expect("guard errors are always JSON objects");
            object.insert("command".into(), Value::String(command.command.into()));
            object.insert("action_id".into(), Value::String(command.action_id.clone()));
        }
        value
    }
}

fn validate_action_id(action_id: &str) -> Result<(), GuardError> {
    if action_id.is_empty() || action_id.len() > MAX_ACTION_ID_BYTES {
        return Err(GuardError::new(
            "INVALID_ACTION_ID",
            format!("action_id must contain 1 to {MAX_ACTION_ID_BYTES} UTF-8 bytes"),
        ));
    }
    if !action_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(GuardError::new(
            "INVALID_ACTION_ID",
            "action_id contains an unsupported character",
        ));
    }
    Ok(())
}

fn parse_command(line: &[u8]) -> Result<Command, GuardError> {
    if line.len() > MAX_COMMAND_BYTES {
        return Err(GuardError::new(
            "COMMAND_TOO_LARGE",
            format!("command exceeds {MAX_COMMAND_BYTES} bytes"),
        ));
    }
    let raw: RawCommand = serde_json::from_slice(line)
        .map_err(|_| GuardError::new("INVALID_COMMAND", "command is not valid guard JSON"))?;
    match (raw.command.as_str(), raw.action_id) {
        ("arm", Some(action_id)) => Ok(Command::Arm { action_id }),
        ("check", Some(action_id)) => Ok(Command::Check { action_id }),
        ("cancel", Some(action_id)) => Ok(Command::Cancel { action_id }),
        ("reject", Some(action_id)) => Ok(Command::Reject { action_id }),
        ("ping", None) => Ok(Command::Ping),
        ("finish", None) => Ok(Command::Finish),
        _ => Err(GuardError::new(
            "INVALID_COMMAND",
            "command name and action_id shape are invalid",
        )),
    }
}

fn read_bounded_line(reader: &mut impl BufRead, maximum: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(maximum + 1);
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Ok(Some(bytes))
            };
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let segment_length = newline.map_or(buffer.len(), |index| index + 1);
        let remaining = (maximum + 1).saturating_sub(bytes.len());
        let copy_length = segment_length.min(remaining);
        bytes.extend_from_slice(&buffer[..copy_length]);
        reader.consume(copy_length);

        if newline.is_some() && copy_length == segment_length {
            bytes.pop();
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }
            return Ok(Some(bytes));
        }
        if bytes.len() > maximum {
            return Ok(Some(bytes));
        }
    }
}

fn current_unix_time_ms() -> Result<u64, GuardError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        GuardError::new("CLOCK_ERROR", "system time is earlier than the Unix epoch")
    })?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| GuardError::new("CLOCK_ERROR", "system time does not fit in milliseconds"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GuardSessionIdentity {
    session_id: String,
    process_id: u32,
    started_at_unix_ms: u64,
}

impl GuardSessionIdentity {
    fn new() -> Result<Self, GuardError> {
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
            GuardError::new("CLOCK_ERROR", "system time is earlier than the Unix epoch")
        })?;
        let started_at_unix_ms = u64::try_from(elapsed.as_millis()).map_err(|_| {
            GuardError::new("CLOCK_ERROR", "system time does not fit in milliseconds")
        })?;
        let process_id = std::process::id();
        let material = format!("{process_id}:{}", elapsed.as_nanos());
        let digest = Sha256::digest(material.as_bytes());
        let mut session_id = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut session_id, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Ok(Self {
            session_id,
            process_id,
            started_at_unix_ms,
        })
    }
}

fn decorate_session_record(
    mut value: Value,
    identity: &GuardSessionIdentity,
    record_sequence: u64,
    recorded_at_unix_ms: u64,
) -> Value {
    let object = value
        .as_object_mut()
        .expect("guard records are always JSON objects");
    object.insert(
        "guard_session_id".into(),
        Value::String(identity.session_id.clone()),
    );
    object.insert("guard_process_id".into(), Value::from(identity.process_id));
    object.insert(
        "guard_started_at_unix_ms".into(),
        Value::from(identity.started_at_unix_ms),
    );
    object.insert("record_sequence".into(), Value::from(record_sequence));
    object.insert(
        "recorded_at_unix_ms".into(),
        Value::from(recorded_at_unix_ms),
    );
    value
}

fn emit_raw(value: &Value) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value)?;
    lock.write_all(b"\n")?;
    lock.flush()
}

struct GuardOutput {
    identity: GuardSessionIdentity,
    next_record_sequence: u64,
}

impl GuardOutput {
    fn new(identity: GuardSessionIdentity) -> Self {
        Self {
            identity,
            next_record_sequence: 1,
        }
    }

    fn emit(&mut self, value: Value) -> Result<(), GuardError> {
        let recorded_at_unix_ms = current_unix_time_ms()?;
        let record = decorate_session_record(
            value,
            &self.identity,
            self.next_record_sequence,
            recorded_at_unix_ms,
        );
        emit_raw(&record).map_err(|error| GuardError::new("OUTPUT_ERROR", error.to_string()))?;
        self.next_record_sequence += 1;
        Ok(())
    }
}

fn time_input_counter_sample_with<C, S>(
    mut clock: C,
    sample: S,
) -> Result<TimedInputCounters, GuardError>
where
    C: FnMut() -> Result<u64, GuardError>,
    S: FnOnce() -> Result<InputCounters, GuardError>,
{
    let started_at_unix_ms = clock()?;
    let sample_result = sample();
    let completed_at_unix_ms = clock()?;
    if completed_at_unix_ms < started_at_unix_ms {
        return Err(GuardError::new(
            "CLOCK_ERROR",
            "system time moved backwards while the HID counter snapshot was being read",
        ));
    }
    let timing = SampleTiming {
        started_at_unix_ms,
        completed_at_unix_ms,
    };
    sample_result
        .map(|counters| TimedInputCounters { counters, timing })
        .map_err(|error| error.with_sample_timing(timing))
}

fn sample_input_counters_timed() -> Result<TimedInputCounters, GuardError> {
    time_input_counter_sample_with(current_unix_time_ms, sample_input_counters)
}

fn handle_command_with_sampler<S>(
    state: &mut GuardState,
    command: Command,
    sample: S,
) -> Result<Value, GuardError>
where
    S: FnOnce() -> Result<TimedInputCounters, GuardError>,
{
    let sample_context = command.sample_context();
    debug_assert_eq!(command.requires_counter_sample(), sample_context.is_some());
    let timed_sample = match &sample_context {
        Some(context) => {
            Some(sample().map_err(|error| error.with_sample_command(context.clone()))?)
        }
        None => None,
    };
    let counters = timed_sample
        .as_ref()
        .map_or_else(InputCounters::default, |sample| sample.counters);
    let response = match state.handle(command, counters) {
        Ok(response) => response,
        Err(error) => {
            let error = match (&timed_sample, sample_context) {
                (Some(sample), Some(context)) => error
                    .with_sample_timing(sample.timing)
                    .with_sample_command(context),
                _ => error,
            };
            return Err(error);
        }
    };
    Ok(match timed_sample {
        Some(sample) => sample.timing.decorate_record(response),
        None => response,
    })
}

fn main() -> ExitCode {
    let identity = match GuardSessionIdentity::new() {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("cubase_input_guard: {}: {}", error.code, error.message);
            return ExitCode::FAILURE;
        }
    };
    let mut output = GuardOutput::new(identity);
    match run(&mut output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = output.emit(error.as_json());
            ExitCode::FAILURE
        }
    }
}

fn run(output: &mut GuardOutput) -> Result<(), GuardError> {
    if !counter_source_supported() {
        return Err(GuardError::new(
            "NOT_SUPPORTED",
            "the HID-only input counter is available only on macOS",
        ));
    }

    sample_input_counters_timed()?;
    output.emit(json!({
        "version": GUARD_PROTOCOL_VERSION,
        "type": "ready",
        "source": "hid_system_state",
        "privacy": GUARD_PRIVACY,
        "coverage": GUARD_COVERAGE,
        "policy": GUARD_POLICY
    }))?;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut state = GuardState::new();
    while let Some(line) = read_bounded_line(&mut reader, MAX_COMMAND_BYTES)
        .map_err(|error| GuardError::new("INPUT_ERROR", error.to_string()))?
    {
        if line.is_empty() {
            continue;
        }
        let command = parse_command(&line)?;
        let finishing = command == Command::Finish;
        let response =
            handle_command_with_sampler(&mut state, command, sample_input_counters_timed)?;
        output.emit(response)?;
        if finishing {
            return Ok(());
        }
    }

    Err(GuardError::new(
        "EOF_WITHOUT_FINISH",
        "stdin reached EOF before a successful finish command",
    ))
}

#[cfg(target_os = "macos")]
fn sample_input_counters() -> Result<InputCounters, GuardError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("cubase-input-guard-counter".into())
        .spawn(move || {
            let _ = sender.send(read_input_state_unchecked());
        })
        .map_err(|error| GuardError::new("COUNTER_THREAD_ERROR", error.to_string()))?;
    let state = receiver
        .recv_timeout(COUNTER_READ_TIMEOUT)
        .map_err(|error| {
            GuardError::new(
                "COUNTER_UNAVAILABLE",
                format!("HID input counters were not available within 2000 ms: {error}"),
            )
        })?;
    if input_changed_during_sample(state.aggregate_before, state.aggregate_after) {
        return Err(GuardError::new(
            "INPUT_DURING_SAMPLE",
            "HID input occurred while the counter snapshot was being read",
        ));
    }
    if state.key_held {
        return Err(GuardError::new(
            "KEY_HELD",
            "a keyboard key was held while the HID counter snapshot was read",
        ));
    }
    if state.mouse_button_held {
        return Err(GuardError::new(
            "MOUSE_BUTTON_HELD",
            "a mouse button was held while the HID counter snapshot was read",
        ));
    }
    Ok(state.counters)
}

#[cfg(not(target_os = "macos"))]
fn sample_input_counters() -> Result<InputCounters, GuardError> {
    Err(GuardError::new(
        "NOT_SUPPORTED",
        "the HID-only input counter is available only on macOS",
    ))
}

#[cfg(target_os = "macos")]
fn counter_source_supported() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn counter_source_supported() -> bool {
    false
}

#[cfg(target_os = "macos")]
struct InputStateRead {
    aggregate_before: u32,
    aggregate_after: u32,
    key_held: bool,
    mouse_button_held: bool,
    counters: InputCounters,
}

#[cfg(target_os = "macos")]
fn read_input_state_unchecked() -> InputStateRead {
    const HID_SYSTEM_STATE: i32 = 1;
    const ANY_INPUT: u32 = u32::MAX;
    const MAX_STANDARD_KEY_CODE: u16 = 255;
    const MAX_MOUSE_BUTTON: u32 = 31;
    const LEFT_MOUSE_DOWN: u32 = 1;
    const LEFT_MOUSE_UP: u32 = 2;
    const RIGHT_MOUSE_DOWN: u32 = 3;
    const RIGHT_MOUSE_UP: u32 = 4;
    const MOUSE_MOVED: u32 = 5;
    const LEFT_MOUSE_DRAGGED: u32 = 6;
    const RIGHT_MOUSE_DRAGGED: u32 = 7;
    const KEY_DOWN: u32 = 10;
    const KEY_UP: u32 = 11;
    const FLAGS_CHANGED: u32 = 12;
    const SCROLL_WHEEL: u32 = 22;
    const TABLET_POINTER: u32 = 23;
    const TABLET_PROXIMITY: u32 = 24;
    const OTHER_MOUSE_DOWN: u32 = 25;
    const OTHER_MOUSE_UP: u32 = 26;
    const OTHER_MOUSE_DRAGGED: u32 = 27;

    fn counter(state: i32, event_type: u32) -> u32 {
        // SAFETY: CoreGraphics accepts the documented HID state identifier and
        // concrete CGEventType constants and returns a process-independent count.
        unsafe { CGEventSourceCounterForEventType(state, event_type) }
    }

    let aggregate_before = counter(HID_SYSTEM_STATE, ANY_INPUT);
    let key_held = (0..=MAX_STANDARD_KEY_CODE).any(|key_code| {
        // SAFETY: Every u16 is a valid CGKeyCode value for this read-only query.
        unsafe { CGEventSourceKeyState(HID_SYSTEM_STATE, key_code) }
    });
    let mouse_button_held = (0..=MAX_MOUSE_BUTTON).any(|button| {
        // SAFETY: CoreGraphics accepts CGMouseButton values as uint32_t and
        // returns false for buttons that are not present.
        unsafe { CGEventSourceButtonState(HID_SYSTEM_STATE, button) }
    });
    let counters = InputCounters {
        mouse_moved: counter(HID_SYSTEM_STATE, MOUSE_MOVED),
        left_mouse_down: counter(HID_SYSTEM_STATE, LEFT_MOUSE_DOWN),
        left_mouse_up: counter(HID_SYSTEM_STATE, LEFT_MOUSE_UP),
        right_mouse_down: counter(HID_SYSTEM_STATE, RIGHT_MOUSE_DOWN),
        right_mouse_up: counter(HID_SYSTEM_STATE, RIGHT_MOUSE_UP),
        other_mouse_down: counter(HID_SYSTEM_STATE, OTHER_MOUSE_DOWN),
        other_mouse_up: counter(HID_SYSTEM_STATE, OTHER_MOUSE_UP),
        left_mouse_dragged: counter(HID_SYSTEM_STATE, LEFT_MOUSE_DRAGGED),
        right_mouse_dragged: counter(HID_SYSTEM_STATE, RIGHT_MOUSE_DRAGGED),
        other_mouse_dragged: counter(HID_SYSTEM_STATE, OTHER_MOUSE_DRAGGED),
        key_down: counter(HID_SYSTEM_STATE, KEY_DOWN),
        key_up: counter(HID_SYSTEM_STATE, KEY_UP),
        flags_changed: counter(HID_SYSTEM_STATE, FLAGS_CHANGED),
        scroll_wheel: counter(HID_SYSTEM_STATE, SCROLL_WHEEL),
        tablet_pointer: counter(HID_SYSTEM_STATE, TABLET_POINTER),
        tablet_proximity: counter(HID_SYSTEM_STATE, TABLET_PROXIMITY),
    };
    let aggregate_after = counter(HID_SYSTEM_STATE, ANY_INPUT);
    InputStateRead {
        aggregate_before,
        aggregate_after,
        key_held,
        mouse_button_held,
        counters,
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceCounterForEventType(state_id: i32, event_type: u32) -> u32;
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;

    fn counters(mouse_moved: u32, key_down: u32) -> InputCounters {
        InputCounters {
            mouse_moved,
            key_down,
            ..InputCounters::default()
        }
    }

    fn timed_counters(
        counters: InputCounters,
        started_at_unix_ms: u64,
        completed_at_unix_ms: u64,
    ) -> TimedInputCounters {
        TimedInputCounters {
            counters,
            timing: SampleTiming {
                started_at_unix_ms,
                completed_at_unix_ms,
            },
        }
    }

    fn test_identity() -> GuardSessionIdentity {
        GuardSessionIdentity {
            session_id: "a".repeat(64),
            process_id: 42,
            started_at_unix_ms: 1_788_000_000_000,
        }
    }

    #[test]
    fn counter_delta_is_zero_when_no_hid_input_occurs() {
        let snapshot = counters(10, 20);
        assert_eq!(snapshot.delta_since(snapshot), InputCounters::default());
        assert!(!snapshot.delta_since(snapshot).any());
        assert!(!snapshot.delta_since(snapshot).any_consequential());
    }

    #[test]
    fn counter_delta_detects_input_and_handles_wraparound() {
        let earlier = counters(u32::MAX, 20);
        let current = counters(1, 22);
        let delta = current.delta_since(earlier);
        assert_eq!(delta.mouse_moved, 2);
        assert_eq!(delta.key_down, 2);
        assert!(delta.any());
        assert!(delta.any_consequential());
    }

    #[test]
    fn sampling_bracket_rejects_any_aggregate_input_change() {
        assert!(!input_changed_during_sample(100, 100));
        assert!(input_changed_during_sample(100, 101));
        assert!(input_changed_during_sample(u32::MAX, 0));
    }

    #[test]
    fn sample_timestamps_strictly_bracket_the_sample_call_without_sleeping() {
        let events = RefCell::new(Vec::new());
        let clock_call = Cell::new(0);
        let sample = time_input_counter_sample_with(
            || {
                events.borrow_mut().push("clock");
                let value = [1_000, 1_025][clock_call.get()];
                clock_call.set(clock_call.get() + 1);
                Ok(value)
            },
            || {
                events.borrow_mut().push("sample");
                Ok(counters(10, 20))
            },
        )
        .unwrap();

        assert_eq!(*events.borrow(), ["clock", "sample", "clock"]);
        assert_eq!(sample.counters, counters(10, 20));
        assert_eq!(sample.timing.started_at_unix_ms, 1_000);
        assert_eq!(sample.timing.completed_at_unix_ms, 1_025);
    }

    #[test]
    fn sample_timing_fails_closed_if_the_wall_clock_moves_backwards() {
        let mut timestamps = [1_025, 1_000].into_iter();
        let error = time_input_counter_sample_with(
            || Ok(timestamps.next().unwrap()),
            || Ok(counters(10, 20)),
        )
        .unwrap_err();

        assert_eq!(error.code, "CLOCK_ERROR");
        assert!(error.sample_timing.is_none());
    }

    #[test]
    fn arm_and_check_records_expose_v5_sample_boundaries() {
        let identity = test_identity();
        let session_started_at_unix_ms = identity.started_at_unix_ms;
        let baseline = counters(10, 20);
        let mut state = GuardState::new();
        let ready = decorate_session_record(
            json!({"version": GUARD_PROTOCOL_VERSION, "type": "ready"}),
            &identity,
            1,
            session_started_at_unix_ms + 10,
        );
        let armed = handle_command_with_sampler(
            &mut state,
            Command::Arm {
                action_id: "timed-action".into(),
            },
            || {
                Ok(timed_counters(
                    baseline,
                    session_started_at_unix_ms + 20,
                    session_started_at_unix_ms + 30,
                ))
            },
        )
        .unwrap();
        let armed = decorate_session_record(armed, &identity, 2, session_started_at_unix_ms + 35);

        let ui_pre_recorded_at_unix_ms = session_started_at_unix_ms + 40;
        let ui_post_recorded_at_unix_ms = session_started_at_unix_ms + 90;
        let checked = handle_command_with_sampler(
            &mut state,
            Command::Check {
                action_id: "timed-action".into(),
            },
            || {
                Ok(timed_counters(
                    baseline,
                    session_started_at_unix_ms + 100,
                    session_started_at_unix_ms + 110,
                ))
            },
        )
        .unwrap();
        let checked =
            decorate_session_record(checked, &identity, 3, session_started_at_unix_ms + 115);

        assert_eq!(GUARD_PROTOCOL_VERSION, 5);
        assert_eq!(armed["version"], 5);
        assert_eq!(armed["type"], "armed");
        assert_eq!(
            armed["sample_started_at_unix_ms"],
            session_started_at_unix_ms + 20
        );
        assert_eq!(
            armed["sample_completed_at_unix_ms"],
            session_started_at_unix_ms + 30
        );
        assert_eq!(
            armed["recorded_at_unix_ms"],
            session_started_at_unix_ms + 35
        );
        assert!(ready.get("sample_started_at_unix_ms").is_none());
        assert!(ready.get("sample_completed_at_unix_ms").is_none());
        assert!(
            ready["recorded_at_unix_ms"].as_u64().unwrap()
                <= armed["sample_started_at_unix_ms"].as_u64().unwrap()
        );
        assert!(
            armed["sample_started_at_unix_ms"].as_u64().unwrap()
                <= armed["sample_completed_at_unix_ms"].as_u64().unwrap()
        );
        assert!(
            armed["sample_completed_at_unix_ms"].as_u64().unwrap() <= ui_pre_recorded_at_unix_ms
        );
        assert!(
            armed["sample_completed_at_unix_ms"].as_u64().unwrap()
                <= armed["recorded_at_unix_ms"].as_u64().unwrap()
        );

        assert_eq!(checked["version"], 5);
        assert_eq!(checked["type"], "result");
        assert_eq!(
            checked["sample_started_at_unix_ms"],
            session_started_at_unix_ms + 100
        );
        assert_eq!(
            checked["sample_completed_at_unix_ms"],
            session_started_at_unix_ms + 110
        );
        assert_eq!(
            checked["recorded_at_unix_ms"],
            session_started_at_unix_ms + 115
        );
        assert!(
            ui_post_recorded_at_unix_ms <= checked["sample_started_at_unix_ms"].as_u64().unwrap()
        );
        assert!(
            checked["sample_started_at_unix_ms"].as_u64().unwrap()
                <= checked["sample_completed_at_unix_ms"].as_u64().unwrap()
        );
        assert!(
            armed["recorded_at_unix_ms"].as_u64().unwrap()
                <= checked["sample_started_at_unix_ms"].as_u64().unwrap()
        );
        assert!(
            checked["sample_completed_at_unix_ms"].as_u64().unwrap()
                <= checked["recorded_at_unix_ms"].as_u64().unwrap()
        );
    }

    #[test]
    fn arm_and_check_sampling_errors_retain_timing_and_command_context() {
        let identity = test_identity();
        let cases = [
            ("KEY_HELD", "arm", 300),
            ("MOUSE_BUTTON_HELD", "check", 400),
            ("INPUT_DURING_SAMPLE", "arm", 500),
        ];

        for (index, (code, command_name, sample_offset_ms)) in cases.into_iter().enumerate() {
            let mut state = GuardState::new();
            let started_at_unix_ms = identity.started_at_unix_ms + sample_offset_ms;
            let command = match command_name {
                "arm" => Command::Arm {
                    action_id: "sample-error".into(),
                },
                "check" => Command::Check {
                    action_id: "sample-error".into(),
                },
                _ => unreachable!(),
            };
            let mut timestamps = [started_at_unix_ms, started_at_unix_ms + 5].into_iter();
            let error = handle_command_with_sampler(&mut state, command, || {
                time_input_counter_sample_with(
                    || Ok(timestamps.next().unwrap()),
                    || Err(GuardError::new(code, "sample rejected")),
                )
            })
            .unwrap_err();
            let record = decorate_session_record(
                error.as_json(),
                &identity,
                u64::try_from(index + 1).unwrap(),
                started_at_unix_ms + 10,
            );

            assert_eq!(record["version"], 5);
            assert_eq!(record["type"], "error");
            assert_eq!(record["error"]["code"], code);
            assert_eq!(record["command"], command_name);
            assert_eq!(record["action_id"], "sample-error");
            assert_eq!(record["sample_started_at_unix_ms"], started_at_unix_ms);
            assert_eq!(
                record["sample_completed_at_unix_ms"],
                started_at_unix_ms + 5
            );
            assert_eq!(record["recorded_at_unix_ms"], started_at_unix_ms + 10);
        }
    }

    #[test]
    fn post_sample_state_errors_also_retain_the_sample_boundary() {
        let mut state = GuardState::new();
        handle_command_with_sampler(
            &mut state,
            Command::Arm {
                action_id: "expected".into(),
            },
            || Ok(timed_counters(counters(10, 20), 600, 605)),
        )
        .unwrap();

        let error = handle_command_with_sampler(
            &mut state,
            Command::Check {
                action_id: "wrong".into(),
            },
            || Ok(timed_counters(counters(10, 20), 700, 705)),
        )
        .unwrap_err()
        .as_json();
        assert_eq!(error["error"]["code"], "ACTION_ID_MISMATCH");
        assert_eq!(error["command"], "check");
        assert_eq!(error["action_id"], "wrong");
        assert_eq!(error["sample_started_at_unix_ms"], 700);
        assert_eq!(error["sample_completed_at_unix_ms"], 705);
    }

    #[test]
    fn mouse_movement_is_informational_not_consequential() {
        let movement_only = counters(1, 0);
        assert!(movement_only.any());
        assert!(!movement_only.any_consequential());

        let mut state = GuardState::new();
        state
            .handle(
                Command::Arm {
                    action_id: "coordinate-click".into(),
                },
                counters(40, 50),
            )
            .unwrap();
        let response = state
            .handle(
                Command::Check {
                    action_id: "coordinate-click".into(),
                },
                counters(47, 50),
            )
            .unwrap();
        assert_eq!(response["interference_detected"], false);
        assert_eq!(response["deltas"]["mouse_moved"], 7);
        assert_eq!(response["policy"], GUARD_POLICY);
        assert!(!state.interference_latched);
    }

    #[test]
    fn every_non_movement_counter_is_consequential() {
        let inputs = [
            InputCounters {
                left_mouse_down: 1,
                ..InputCounters::default()
            },
            InputCounters {
                left_mouse_up: 1,
                ..InputCounters::default()
            },
            InputCounters {
                right_mouse_down: 1,
                ..InputCounters::default()
            },
            InputCounters {
                right_mouse_up: 1,
                ..InputCounters::default()
            },
            InputCounters {
                other_mouse_down: 1,
                ..InputCounters::default()
            },
            InputCounters {
                other_mouse_up: 1,
                ..InputCounters::default()
            },
            InputCounters {
                left_mouse_dragged: 1,
                ..InputCounters::default()
            },
            InputCounters {
                right_mouse_dragged: 1,
                ..InputCounters::default()
            },
            InputCounters {
                other_mouse_dragged: 1,
                ..InputCounters::default()
            },
            InputCounters {
                key_down: 1,
                ..InputCounters::default()
            },
            InputCounters {
                key_up: 1,
                ..InputCounters::default()
            },
            InputCounters {
                flags_changed: 1,
                ..InputCounters::default()
            },
            InputCounters {
                scroll_wheel: 1,
                ..InputCounters::default()
            },
            InputCounters {
                tablet_pointer: 1,
                ..InputCounters::default()
            },
            InputCounters {
                tablet_proximity: 1,
                ..InputCounters::default()
            },
        ];

        for input in inputs {
            assert!(input.any_consequential(), "missed {input:?}");
            let with_movement = InputCounters {
                mouse_moved: 99,
                ..input
            };
            assert!(
                with_movement.any_consequential(),
                "missed {with_movement:?}"
            );
        }
    }

    #[test]
    fn commands_are_strict_and_action_ids_are_bounded() {
        assert_eq!(
            parse_command(br#"{"command":"arm","action_id":"S3-delete"}"#).unwrap(),
            Command::Arm {
                action_id: "S3-delete".into()
            }
        );
        assert_eq!(
            parse_command(br#"{"command":"reject","action_id":"S3-delete"}"#).unwrap(),
            Command::Reject {
                action_id: "S3-delete".into()
            }
        );
        assert!(parse_command(br#"{"command":"arm","action_id":""}"#).is_ok());
        assert!(parse_command(br#"{"command":"ping","extra":true}"#).is_err());
        assert!(parse_command(&vec![b'x'; MAX_COMMAND_BYTES + 1]).is_err());
        assert!(validate_action_id("").is_err());
        assert!(validate_action_id("contains space").is_err());
        assert!(validate_action_id(&"x".repeat(MAX_ACTION_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn oversized_input_is_rejected_without_waiting_for_a_newline() {
        let mut input = vec![b'x'; MAX_COMMAND_BYTES + 20];
        input.extend_from_slice(b"\n{\"command\":\"ping\"}\n");
        let mut reader = Cursor::new(input);
        let oversized = read_bounded_line(&mut reader, MAX_COMMAND_BYTES)
            .unwrap()
            .unwrap();
        assert_eq!(oversized.len(), MAX_COMMAND_BYTES + 1);
        assert_eq!(
            parse_command(&oversized).unwrap_err().code,
            "COMMAND_TOO_LARGE"
        );

        let mut unterminated = Cursor::new(vec![b'x'; MAX_COMMAND_BYTES + 20]);
        let oversized = read_bounded_line(&mut unterminated, MAX_COMMAND_BYTES)
            .unwrap()
            .unwrap();
        assert_eq!(oversized.len(), MAX_COMMAND_BYTES + 1);
    }

    #[test]
    fn guard_requires_exact_pair_and_disarms_after_check() {
        let mut state = GuardState::new();
        state
            .handle(
                Command::Arm {
                    action_id: "S6-mute".into(),
                },
                counters(10, 20),
            )
            .unwrap();
        assert_eq!(
            state
                .handle(
                    Command::Check {
                        action_id: "wrong".into()
                    },
                    counters(10, 20)
                )
                .unwrap_err()
                .code,
            "ACTION_ID_MISMATCH"
        );
        let response = state
            .handle(
                Command::Check {
                    action_id: "S6-mute".into(),
                },
                counters(10, 21),
            )
            .unwrap();
        assert_eq!(response["interference_detected"], true);
        assert_eq!(response["coverage"], GUARD_COVERAGE);
        assert_eq!(response["policy"], GUARD_POLICY);
        assert_eq!(response["deltas"]["key_down"], 1);
        assert!(state.armed.is_none());
        assert!(state.interference_latched);
        assert_eq!(
            state
                .handle(
                    Command::Arm {
                        action_id: "next".into()
                    },
                    counters(10, 21)
                )
                .unwrap_err()
                .code,
            "INTERFERENCE_LATCHED"
        );
        assert_eq!(
            state
                .handle(Command::Finish, InputCounters::default())
                .unwrap_err()
                .code,
            "INTERFERENCE_LATCHED"
        );
    }

    #[test]
    fn input_between_actions_is_rebaselined_by_the_next_arm() {
        let baseline = counters(10, 20);
        let mut state = GuardState::new();
        state
            .handle(
                Command::Arm {
                    action_id: "first".into(),
                },
                baseline,
            )
            .unwrap();
        let checked = state
            .handle(
                Command::Check {
                    action_id: "first".into(),
                },
                baseline,
            )
            .unwrap();
        assert_eq!(checked["interference_detected"], false);

        let next_baseline = counters(11, 20);
        state
            .handle(
                Command::Arm {
                    action_id: "second".into(),
                },
                next_baseline,
            )
            .unwrap();
        let checked = state
            .handle(
                Command::Check {
                    action_id: "second".into(),
                },
                next_baseline,
            )
            .unwrap();
        assert_eq!(checked["interference_detected"], false);
    }

    #[test]
    fn clean_result_can_be_rejected_when_the_ui_postcondition_fails() {
        let baseline = counters(10, 20);
        let mut state = GuardState::new();
        assert_eq!(
            state
                .handle(
                    Command::Reject {
                        action_id: "wrong-target".into()
                    },
                    InputCounters::default()
                )
                .unwrap_err()
                .code,
            "NO_CLEAN_RESULT"
        );
        state
            .handle(
                Command::Arm {
                    action_id: "wrong-target".into(),
                },
                baseline,
            )
            .unwrap();
        assert_eq!(
            state
                .handle(
                    Command::Reject {
                        action_id: "wrong-target".into()
                    },
                    InputCounters::default()
                )
                .unwrap_err()
                .code,
            "REJECT_WHILE_ARMED"
        );
        let checked = state
            .handle(
                Command::Check {
                    action_id: "wrong-target".into(),
                },
                baseline,
            )
            .unwrap();
        assert_eq!(checked["interference_detected"], false);
        assert_eq!(
            state
                .handle(
                    Command::Reject {
                        action_id: "other".into()
                    },
                    InputCounters::default()
                )
                .unwrap_err()
                .code,
            "ACTION_ID_MISMATCH"
        );
        let rejected = state
            .handle(
                Command::Reject {
                    action_id: "wrong-target".into(),
                },
                InputCounters::default(),
            )
            .unwrap();
        assert_eq!(rejected["type"], "rejected");
        assert_eq!(rejected["reason"], "postcondition_failed");
        assert_eq!(rejected["after_clean_result"], true);
        assert_eq!(rejected["session_aborted"], true);
        assert_eq!(
            state
                .handle(Command::Finish, InputCounters::default())
                .unwrap_err()
                .code,
            "SESSION_ABORTED"
        );
    }

    #[test]
    fn trusted_session_and_record_time_are_added_to_every_emitted_record() {
        let identity = test_identity();
        let record = decorate_session_record(
            json!({
                "version": GUARD_PROTOCOL_VERSION,
                "type": "ready",
                "recorded_at_unix_ms": 1
            }),
            &identity,
            7,
            1_788_000_000_123,
        );
        assert_eq!(record["guard_session_id"], "a".repeat(64));
        assert_eq!(record["guard_process_id"], 42);
        assert_eq!(record["guard_started_at_unix_ms"], 1_788_000_000_000_u64);
        assert_eq!(record["record_sequence"], 7);
        assert_eq!(record["recorded_at_unix_ms"], 1_788_000_000_123_u64);
    }

    #[test]
    fn generated_session_identity_has_a_bounded_correlation_shape() {
        let identity = GuardSessionIdentity::new().unwrap();
        assert_eq!(identity.session_id.len(), 64);
        assert!(
            identity
                .session_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(identity.process_id, std::process::id());
        assert!(identity.started_at_unix_ms > 0);
    }

    #[test]
    fn finish_requires_a_clean_unarmed_session() {
        let baseline = counters(10, 20);
        let mut state = GuardState::new();
        let response = state.handle(Command::Finish, baseline).unwrap();
        assert_eq!(response["type"], "finished");
        assert_eq!(response["version"], GUARD_PROTOCOL_VERSION);
        assert_eq!(response["coverage"], GUARD_COVERAGE);
        assert_eq!(response["policy"], GUARD_POLICY);

        let mut armed = GuardState::new();
        armed
            .handle(
                Command::Arm {
                    action_id: "active".into(),
                },
                baseline,
            )
            .unwrap();
        assert_eq!(
            armed.handle(Command::Finish, baseline).unwrap_err().code,
            "FINISH_WHILE_ARMED"
        );

        let mut cancelled = GuardState::new();
        cancelled
            .handle(
                Command::Arm {
                    action_id: "cancelled".into(),
                },
                baseline,
            )
            .unwrap();
        let response = cancelled
            .handle(
                Command::Cancel {
                    action_id: "cancelled".into(),
                },
                baseline,
            )
            .unwrap();
        assert_eq!(response["session_aborted"], true);
        assert_eq!(
            cancelled
                .handle(Command::Finish, baseline)
                .unwrap_err()
                .code,
            "SESSION_ABORTED"
        );
    }

    #[test]
    fn idle_input_is_not_sampled_by_ping_or_finish() {
        let mut state = GuardState::new();
        let idle_input = counters(99, 42);
        let pong = state.handle(Command::Ping, idle_input).unwrap();
        assert_eq!(pong["type"], "pong");
        assert_eq!(pong["interference_latched"], false);
        let finished = state.handle(Command::Finish, idle_input).unwrap();
        assert_eq!(finished["type"], "finished");

        assert!(
            Command::Arm {
                action_id: "arm".into()
            }
            .requires_counter_sample()
        );
        assert!(
            Command::Check {
                action_id: "check".into()
            }
            .requires_counter_sample()
        );
        assert!(!Command::Ping.requires_counter_sample());
        assert!(!Command::Finish.requires_counter_sample());
        assert!(
            !Command::Cancel {
                action_id: "cancel".into()
            }
            .requires_counter_sample()
        );
        assert!(
            !Command::Reject {
                action_id: "reject".into()
            }
            .requires_counter_sample()
        );
        let error = GuardError::new("TEST", "test").as_json();
        assert_eq!(error["version"], GUARD_PROTOCOL_VERSION);
        assert_eq!(error["coverage"], GUARD_COVERAGE);
        assert_eq!(error["policy"], GUARD_POLICY);
        assert!(
            !error
                .as_object()
                .unwrap()
                .contains_key("sample_started_at_unix_ms")
        );
        assert!(
            !error
                .as_object()
                .unwrap()
                .contains_key("sample_completed_at_unix_ms")
        );
    }
}
