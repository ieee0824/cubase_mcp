use std::io::{self, BufRead, Write};
use std::process::ExitCode;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const GUARD_PROTOCOL_VERSION: u32 = 1;
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
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Arm { action_id: String },
    Check { action_id: String },
    Cancel { action_id: String },
    Ping,
    Finish,
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
    interference_latched: bool,
    session_aborted: bool,
    last_counters: InputCounters,
}

impl GuardState {
    fn new(initial_counters: InputCounters) -> Self {
        Self {
            armed: None,
            interference_latched: false,
            session_aborted: false,
            last_counters: initial_counters,
        }
    }

    fn handle(&mut self, command: Command, current: InputCounters) -> Result<Value, GuardError> {
        let session_deltas = current.delta_since(self.last_counters);
        self.last_counters = current;
        self.interference_latched |= session_deltas.any();

        match command {
            Command::Arm { action_id } => {
                validate_action_id(&action_id)?;
                if self.session_aborted {
                    return Err(GuardError::new(
                        "SESSION_ABORTED",
                        "an armed action was cancelled earlier in this guard session",
                    ));
                }
                if self.interference_latched {
                    return Err(GuardError::new(
                        "INTERFERENCE_LATCHED",
                        "physical input was detected earlier in this guard session",
                    ));
                }
                if let Some(armed) = &self.armed {
                    return Err(GuardError::new(
                        "ALREADY_ARMED",
                        format!("input guard is already armed for {}", armed.action_id),
                    ));
                }
                self.armed = Some(ArmedAction {
                    action_id: action_id.clone(),
                    counters: current,
                });
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "armed",
                    "action_id": action_id,
                    "source": "hid_system_state"
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
                let interference_detected = deltas.any();
                self.armed = None;
                self.interference_latched |= interference_detected;
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "result",
                    "action_id": action_id,
                    "source": "hid_system_state",
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
                self.session_aborted = true;
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "cancelled",
                    "action_id": action_id,
                    "session_aborted": true
                }))
            }
            Command::Ping => Ok(json!({
                "version": GUARD_PROTOCOL_VERSION,
                "type": "pong",
                "source": "hid_system_state",
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
                        "physical input was detected during this guard session",
                    ));
                }
                if self.session_aborted {
                    return Err(GuardError::new(
                        "SESSION_ABORTED",
                        "an armed action was cancelled earlier in this guard session",
                    ));
                }
                Ok(json!({
                    "version": GUARD_PROTOCOL_VERSION,
                    "type": "finished",
                    "source": "hid_system_state",
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
}

impl GuardError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "version": GUARD_PROTOCOL_VERSION,
            "type": "error",
            "error": {
                "code": self.code,
                "message": self.message
            }
        })
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

fn emit(value: &Value) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value)?;
    lock.write_all(b"\n")?;
    lock.flush()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = emit(&error.as_json());
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), GuardError> {
    if !counter_source_supported() {
        return Err(GuardError::new(
            "NOT_SUPPORTED",
            "the HID-only input counter is available only on macOS",
        ));
    }

    let initial_counters = sample_input_counters()?;
    emit(&json!({
        "version": GUARD_PROTOCOL_VERSION,
        "type": "ready",
        "source": "hid_system_state",
        "privacy": "counts_only",
        "coverage": "session_wide"
    }))
    .map_err(|error| GuardError::new("OUTPUT_ERROR", error.to_string()))?;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut state = GuardState::new(initial_counters);
    while let Some(line) = read_bounded_line(&mut reader, MAX_COMMAND_BYTES)
        .map_err(|error| GuardError::new("INPUT_ERROR", error.to_string()))?
    {
        if line.is_empty() {
            continue;
        }
        let command = parse_command(&line)?;
        let finishing = command == Command::Finish;
        let response = state.handle(command, sample_input_counters()?)?;
        emit(&response).map_err(|error| GuardError::new("OUTPUT_ERROR", error.to_string()))?;
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
    if state.aggregate_before != state.aggregate_after {
        return Err(GuardError::new(
            "INPUT_DURING_SAMPLE",
            "input occurred while the HID counter snapshot was being read",
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
    use std::io::Cursor;

    fn counters(mouse_moved: u32, key_down: u32) -> InputCounters {
        InputCounters {
            mouse_moved,
            key_down,
            ..InputCounters::default()
        }
    }

    #[test]
    fn counter_delta_is_zero_when_no_hid_input_occurs() {
        let snapshot = counters(10, 20);
        assert_eq!(snapshot.delta_since(snapshot), InputCounters::default());
        assert!(!snapshot.delta_since(snapshot).any());
    }

    #[test]
    fn counter_delta_detects_input_and_handles_wraparound() {
        let earlier = counters(u32::MAX, 20);
        let current = counters(1, 22);
        let delta = current.delta_since(earlier);
        assert_eq!(delta.mouse_moved, 2);
        assert_eq!(delta.key_down, 2);
        assert!(delta.any());
    }

    #[test]
    fn commands_are_strict_and_action_ids_are_bounded() {
        assert_eq!(
            parse_command(br#"{"command":"arm","action_id":"S3-delete"}"#).unwrap(),
            Command::Arm {
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
        let mut state = GuardState::new(counters(10, 20));
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
                counters(11, 20),
            )
            .unwrap();
        assert_eq!(response["interference_detected"], true);
        assert!(state.armed.is_none());
        assert!(state.interference_latched);
        assert_eq!(
            state
                .handle(
                    Command::Arm {
                        action_id: "next".into()
                    },
                    counters(11, 20)
                )
                .unwrap_err()
                .code,
            "INTERFERENCE_LATCHED"
        );
    }

    #[test]
    fn input_between_actions_is_latched_before_the_next_arm() {
        let baseline = counters(10, 20);
        let mut state = GuardState::new(baseline);
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

        assert_eq!(
            state
                .handle(
                    Command::Arm {
                        action_id: "second".into(),
                    },
                    counters(11, 20),
                )
                .unwrap_err()
                .code,
            "INTERFERENCE_LATCHED"
        );
    }

    #[test]
    fn finish_requires_a_clean_unarmed_session() {
        let baseline = counters(10, 20);
        let mut state = GuardState::new(baseline);
        let response = state.handle(Command::Finish, baseline).unwrap();
        assert_eq!(response["type"], "finished");

        let mut armed = GuardState::new(baseline);
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

        let mut cancelled = GuardState::new(baseline);
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
}
