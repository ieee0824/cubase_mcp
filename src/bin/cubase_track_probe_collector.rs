use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{
    Ignore, MidiInput, MidiInputConnection, MidiInputPort, MidiOutput, MidiOutputConnection,
    MidiOutputPort,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const TO_CUBASE_PORT: &str = "Cubase MCP Track Probe To Cubase";
const FROM_CUBASE_PORT: &str = "Cubase MCP Track Probe From Cubase";
const PROBE_SYSEX_HEADER: [u8; 7] = [0xF0, 0x7D, b'C', b'M', b'T', b'P', 0x01];
// Cubase sends the Universal Non-Realtime broadcast Identity Request when a
// new virtual MIDI output appears. It is transport discovery traffic, not a
// Track Probe frame. Keep the exception exact so every other foreign SysEx
// still fails closed.
const MIDI_BROADCAST_IDENTITY_REQUEST: [u8; 6] = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
const PROBE_TRANSPORT_VERSION: u32 = 1;
const PROBE_MESSAGE_VERSION: u32 = 1;
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_OUTBOUND_JSON_BYTES: usize = 4 * 1024;
const MAX_SYSEX_BYTES: usize = PROBE_SYSEX_HEADER.len() + MAX_JSON_BYTES * 2 + 1;
const MAX_STDIN_COMMAND_BYTES: usize = MAX_JSON_BYTES;
const MIDI_QUEUE_CAPACITY: usize = 1024;
const MAX_SOURCE_INSTANCES: usize = 16;
const MAX_INSTANCE_ID_BYTES: usize = 128;
const SELECTED_TARGET_ALIAS: &str = "@selected";
const MAX_REQUEST_ID_BYTES: usize = 256;
const MAX_METHOD_BYTES: usize = 128;
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_CHECKPOINT_ID_BYTES: usize = 128;
const MAX_TRACKED_REQUESTS: usize = 4096;
const MAX_TRACKED_SNAPSHOTS: usize = 4096;
const MAX_TRACKED_CHECKPOINTS: usize = 4096;
const MAX_OPEN_SNAPSHOTS: usize = 128;
const MAX_CHUNKS_PER_SNAPSHOT: usize = 4096;
const MAX_ITEMS_PER_CHUNK: usize = 2;
const MAX_ITEMS_PER_SNAPSHOT: usize = 1024;
const MAX_INLINE_HOST_ID_BYTES: usize = 256;
const MAX_HOST_ID_FRAGMENT_BYTES: usize = 256;
const MAX_HOST_ID_BYTES: usize = 4096;
const MAX_HOST_ID_FRAGMENTS: usize = 16;
const COLLECTOR_POLL_INTERVAL: Duration = Duration::from_millis(20);
const INGRESS_BARRIER_TIMEOUT: Duration = Duration::from_millis(5_000);
const CHECKPOINT_QUIET_PERIOD: Duration = Duration::from_millis(1_000);
const DEFAULT_GRACEFUL_DRAIN_MS: u64 = 5_000;
const DEFAULT_DISCOVERY_WINDOW_MS: u64 = 1_000;
const MAX_COLLECTOR_WAIT_MS: u64 = 600_000;
const MAX_OBSERVATION_EPOCH: u64 = 2_147_483_647;
const RECORD_FORMAT_VERSION: u32 = 1;
const HASH_READ_BUFFER_BYTES: usize = 64 * 1024;

fn current_executable_sha256() -> Result<String, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("could not resolve the running collector binary: {error}"))?;
    let file = File::open(executable)
        .map_err(|error| format!("could not open the running collector binary: {error}"))?;
    sha256_reader(file)
        .map_err(|error| format!("could not hash the running collector binary: {error}"))
}

fn sha256_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; HASH_READ_BUFFER_BYTES];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(encoded)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("cubase track probe collector failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let action = Config::from_process()?;
    let config = match action {
        CliAction::Run(config) => config,
        CliAction::PrintHelp => {
            print!("{}", Config::help());
            return Ok(true);
        }
        CliAction::PrintVersion => {
            println!("cubase_track_probe_collector {}", env!("CARGO_PKG_VERSION"));
            return Ok(true);
        }
    };

    let collector_binary_sha256 = current_executable_sha256()?;
    let started_at = Instant::now();
    let integrity_failed = Arc::new(AtomicBool::new(false));
    let runtime = Arc::new(RuntimeTracker::new());
    let MidiConnections {
        mut output,
        input,
        receiver,
        dropped_items,
        ingress_progress,
        mode,
        resolved_input_port,
        resolved_output_port,
    } = connect_midi(&config, Arc::clone(&integrity_failed))?;
    let sink = Arc::new(JsonlSink::stdout(config.run_id.clone(), started_at));
    let session_id = new_session_id();

    sink.emit(&json!({
        "record_type": "collector_started",
        "timestamp_unix_ms": unix_timestamp_ms(),
        "session_id": session_id,
        "collector_version": env!("CARGO_PKG_VERSION"),
        "collector_binary_sha256": collector_binary_sha256,
        "probe_transport_version": PROBE_TRANSPORT_VERSION,
        "midi_mode": mode,
        "virtual_to_cubase_port": (mode == "virtual").then_some(TO_CUBASE_PORT),
        "virtual_from_cubase_port": (mode == "virtual").then_some(FROM_CUBASE_PORT),
        "configured_midi_input_port": if mode == "existing" {
            config.midi_input.as_deref()
        } else {
            None
        },
        "configured_midi_output_port": if mode == "existing" {
            config.midi_output.as_deref()
        } else {
            None
        },
        "resolved_midi_input_port": resolved_input_port,
        "resolved_midi_output_port": resolved_output_port,
        "max_json_bytes": MAX_JSON_BYTES,
        "max_sysex_bytes": MAX_SYSEX_BYTES,
        "max_outbound_json_bytes": MAX_OUTBOUND_JSON_BYTES,
        "queue_capacity": MIDI_QUEUE_CAPACITY,
        "ingress_barrier_timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT),
        "checkpoint_quiet_period_ms": duration_ms(CHECKPOINT_QUIET_PERIOD),
        "graceful_drain_timeout_ms": duration_ms(config.graceful_drain),
        "discovery_window_ms": duration_ms(config.discovery_window)
    }))
    .map_err(|error| format!("could not write collector start record: {error}"))?;

    let collector_sink = Arc::clone(&sink);
    let collector_integrity = Arc::clone(&integrity_failed);
    let collector_dropped = Arc::clone(&dropped_items);
    let collector_runtime = Arc::clone(&runtime);
    let collector_ingress_progress = Arc::clone(&ingress_progress);
    let collector = thread::Builder::new()
        .name("cubase-track-probe-drain".into())
        .spawn(move || {
            collect_incoming(
                receiver,
                collector_dropped,
                collector_integrity,
                collector_sink,
                collector_runtime,
                collector_ingress_progress,
            )
        })
        .map_err(|error| format!("could not start MIDI drain thread: {error}"))?;

    let command_report = process_stdin_commands(
        io::stdin().lock(),
        &mut output,
        &session_id,
        CommandEnvironment {
            integrity_failed: &integrity_failed,
            sink: &sink,
            runtime: &runtime,
            ingress_progress: &ingress_progress,
            discovery_window: config.discovery_window,
        },
    );

    let drain_report = graceful_drain(&runtime, config.graceful_drain, &sink, &integrity_failed);

    let (_input, callback_state) = input.close();
    if callback_state.framer.has_partial_frame() {
        enqueue_ingress(
            &callback_state.sender,
            &callback_state.dropped_items,
            &callback_state.integrity_failed,
            &callback_state.ingress_progress,
            Ingress::FramingFault {
                received_at_unix_ms: unix_timestamp_ms(),
                received_at_monotonic: Instant::now(),
                fault: FramingFault::TruncatedAtShutdown,
            },
        );
    }
    drop(callback_state);
    drop(output);

    let collector_report = match collector.join() {
        Ok(report) => report,
        Err(_) => {
            integrity_failed.store(true, Ordering::Release);
            let _ = emit_diagnostic(
                &sink,
                "COLLECTOR_THREAD_PANIC",
                "fatal",
                "The MIDI drain thread panicked; the observation is invalid",
                json!({}),
            );
            CollectorReport::default()
        }
    };

    let (tracker_summary, orphan_messages, final_quiescent, final_incomplete) = {
        let tracker = runtime.state.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        (
            tracker.summary(),
            tracker.orphan_messages,
            tracker.is_quiescent(),
            tracker.incomplete_details(),
        )
    };
    if orphan_messages > 0 {
        integrity_failed.store(true, Ordering::Release);
        let _ = emit_diagnostic(
            &sink,
            "ORPHAN_MESSAGES_OBSERVED",
            "fatal",
            "one or more probe messages were observed outside a checkpoint",
            json!({"orphan_messages": orphan_messages}),
        );
    }
    if drain_report.completed && !final_quiescent {
        integrity_failed.store(true, Ordering::Release);
        let _ = emit_diagnostic(
            &sink,
            "SHUTDOWN_PROTOCOL_INCOMPLETE",
            "fatal",
            "protocol work arrived after graceful drain completion and remained incomplete",
            json!({"incomplete": final_incomplete}),
        );
    }
    let integrity_ok = !integrity_failed.load(Ordering::Acquire)
        && !sink.failed()
        && collector_report.diagnostics == 0
        && drain_report.completed
        && final_quiescent
        && orphan_messages == 0;
    let commands_ok = command_report.rejected == 0 && !command_report.input_failed;
    let exit_ok = integrity_ok && commands_ok;
    let summary = json!({
        "record_type": "collector_summary",
        "timestamp_unix_ms": unix_timestamp_ms(),
        "session_id": session_id,
        "integrity_ok": integrity_ok,
        "exit_ok": exit_ok,
        "exit_reason": command_report.exit_reason,
        "commands": {
            "received": command_report.received,
            "sent": command_report.sent,
            "local": command_report.local_commands,
            "deferred": command_report.deferred,
            "rejected": command_report.rejected
        },
        "graceful_drain": {
            "completed": drain_report.completed,
            "timed_out": drain_report.timed_out,
            "duration_ms": drain_report.duration_ms
        },
        "orphan_messages": orphan_messages,
        "protocol_tracking": tracker_summary,
        "incoming": {
            "frames": collector_report.frames,
            "messages": collector_report.messages,
            "events": collector_report.events,
            "responses": collector_report.responses,
            "errors": collector_report.errors,
            "diagnostics": collector_report.diagnostics,
            "parse_errors": collector_report.parse_errors,
            "oversize_frames": collector_report.oversize_frames,
            "source_overflows": collector_report.source_overflows,
            "queue_drops": collector_report.queue_drops,
            "sequence_gaps": collector_report.sequence_gaps,
            "sequence_duplicates_or_reorders": collector_report.sequence_duplicates,
            "sources": collector_report.sources
        }
    });
    if let Err(error) = sink.emit(&summary) {
        return Err(format!("could not write collector summary: {error}"));
    }

    Ok(exit_ok)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    run_id: String,
    midi_input: Option<String>,
    midi_output: Option<String>,
    graceful_drain: Duration,
    discovery_window: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Run(Config),
    PrintHelp,
    PrintVersion,
}

impl Config {
    fn from_process() -> Result<CliAction, String> {
        Self::parse(env::args().skip(1), cfg!(unix))
    }

    fn parse(
        arguments: impl IntoIterator<Item = String>,
        virtual_ports_supported: bool,
    ) -> Result<CliAction, String> {
        let mut arguments = arguments.into_iter();
        let mut config = Config {
            run_id: String::new(),
            midi_input: None,
            midi_output: None,
            graceful_drain: Duration::from_millis(DEFAULT_GRACEFUL_DRAIN_MS),
            discovery_window: Duration::from_millis(DEFAULT_DISCOVERY_WINDOW_MS),
        };

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--run-id" => {
                    config.run_id = next_value(&mut arguments, "--run-id")?;
                }
                "--midi-input" => {
                    config.midi_input = Some(next_value(&mut arguments, "--midi-input")?);
                }
                "--midi-output" => {
                    config.midi_output = Some(next_value(&mut arguments, "--midi-output")?);
                }
                "--drain-timeout-ms" => {
                    let value = next_value(&mut arguments, "--drain-timeout-ms")?;
                    config.graceful_drain =
                        Duration::from_millis(parse_wait_ms(&value, "--drain-timeout-ms")?);
                }
                "--discovery-window-ms" => {
                    let value = next_value(&mut arguments, "--discovery-window-ms")?;
                    config.discovery_window =
                        Duration::from_millis(parse_wait_ms(&value, "--discovery-window-ms")?);
                }
                "-h" | "--help" => return Ok(CliAction::PrintHelp),
                "-V" | "--version" => return Ok(CliAction::PrintVersion),
                _ => return Err(format!("unknown argument '{argument}'")),
            }
        }

        if config.run_id.is_empty() || config.run_id.len() > MAX_RUN_ID_BYTES {
            return Err(format!(
                "--run-id is required and must contain 1 to {MAX_RUN_ID_BYTES} UTF-8 bytes"
            ));
        }

        match (&config.midi_input, &config.midi_output) {
            (Some(_), None) | (None, Some(_)) => {
                return Err("--midi-input and --midi-output must be provided together".into());
            }
            (None, None) if !virtual_ports_supported => {
                return Err(
                    "Windows requires explicit --midi-input and --midi-output port names".into(),
                );
            }
            _ => {}
        }

        Ok(CliAction::Run(config))
    }

    fn help() -> &'static str {
        r#"Cubase Track Host API probe collector

Usage: cubase_track_probe_collector [OPTIONS]

Options:
  --run-id <ID>                    Required non-secret identifier written to every JSONL record
  --midi-input <NAME>              Existing 'From Cubase' port (required with --midi-output)
  --midi-output <NAME>             Existing 'To Cubase' port (required with --midi-input)
  --drain-timeout-ms <MILLIS>      EOF graceful-drain deadline (default: 5000)
  --discovery-window-ms <MILLIS>   Multi-instance discovery window (default: 1000)
  -h, --help                       Print help
  -V, --version                    Print version

macOS/Linux default virtual ports:
  Cubase MCP Track Probe To Cubase
  Cubase MCP Track Probe From Cubase

Windows requires both MIDI port options. On macOS/Linux, passing both options
selects existing ports instead of creating the dedicated virtual ports.
Cubase may poll a newly visible output with the exact Universal Identity Request
F0 7E 7F 06 01 F7. The collector ignores only that six-byte transport request;
every other foreign SysEx remains a fatal integrity error.

stdin accepts one JSON command per line. The collector assigns the request id.
This C15/DirectAccess-active example shows the required command order; perform
the indicated host action and waits between lines rather than pasting it as a
batch:
  {"method":"collector.checkpoint.begin","params":{"checkpoint_id":"INIT","window_ms":5000}}
  {"method":"collector.action","params":{"checkpoint_id":"INIT"}}
  [start Cubase and wait for probe.ready]
  {"target_instance_id":null,"method":"probe.discover","params":{}}
  {"target_instance_id":"@selected","method":"probe.capabilities.get","params":{}}
  [wait until 5000 ms after collector.action]
  {"target_instance_id":"@selected","method":"probe.direct_access.snapshot","params":{}}
  {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_ALL"}}
  {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_VISIBLE"}}
  [wait for every follow-up and 1000 ms message-free quiet]
  {"method":"collector.checkpoint.end","params":{"checkpoint_id":"INIT"}}
  {"method":"collector.checkpoint.begin","params":{"checkpoint_id":"E0","window_ms":5000}}
  {"target_instance_id":"@selected","method":"probe.observation.cut","params":{}}
  [wait for the successful cut response and adjacent automatic action marker]
  [perform the E0 UI action immediately, then wait 5000 ms from that marker]
  {"target_instance_id":"@selected","method":"probe.direct_access.snapshot","params":{}}
  {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_ALL"}}
  {"target_instance_id":"@selected","method":"probe.bank.snapshot","params":{"config_id":"MB_CORE_VISIBLE"}}
  [wait for every follow-up and 1000 ms message-free quiet]
  {"method":"collector.checkpoint.end","params":{"checkpoint_id":"E0"}}

Wait for every response/follow-up before the next JSON command that depends on
it. A successful observation cut atomically emits its probe_response followed by
the checkpoint's collector_action marker; do not send collector.action again.
DirectAccess-unsupported runs omit only the DirectAccess snapshot. The revision
2 fixture is authoritative for all 44 checkpoints.

@selected is a collector-local alias. It is accepted only after a completed
exactly-one discovery window and is replaced with that source at the MIDI send
barrier. The alias itself is never sent to Cubase. To avoid blocking MIDI input,
its probe_command and send-result evidence is written to stdout immediately
after the atomic send attempt; `sent` remains authoritative if stdout then fails.

stdout is flushed JSON Lines. Redirect it explicitly if a raw observation file
is required. Send stdin EOF (Ctrl-D) for a final summary. The collector never
creates an observation file itself.
"#
    }
}

fn parse_wait_ms(value: &str, option: &str) -> Result<u64, String> {
    let value = value
        .parse::<u64>()
        .map_err(|_| format!("{option} requires milliseconds as an integer"))?;
    if value == 0 || value > MAX_COLLECTOR_WAIT_MS {
        return Err(format!(
            "{option} must be between 1 and {MAX_COLLECTOR_WAIT_MS} milliseconds"
        ));
    }
    Ok(value)
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing value for {option}"))
}

struct MidiConnections {
    output: MidiOutputConnection,
    input: MidiInputConnection<MidiCallbackState>,
    receiver: Receiver<Ingress>,
    dropped_items: Arc<AtomicU64>,
    ingress_progress: Arc<IngressProgress>,
    mode: &'static str,
    resolved_input_port: String,
    resolved_output_port: String,
}

fn connect_midi(
    config: &Config,
    integrity_failed: Arc<AtomicBool>,
) -> Result<MidiConnections, String> {
    match (&config.midi_input, &config.midi_output) {
        (Some(input), Some(output)) => connect_existing_ports(input, output, integrity_failed),
        (None, None) => {
            #[cfg(unix)]
            {
                connect_virtual_ports(integrity_failed)
            }
            #[cfg(not(unix))]
            {
                Err("virtual MIDI ports are unavailable on this platform".into())
            }
        }
        _ => Err("MIDI input/output ports must be configured as a pair".into()),
    }
}

#[cfg(unix)]
fn connect_virtual_ports(integrity_failed: Arc<AtomicBool>) -> Result<MidiConnections, String> {
    let output = MidiOutput::new("cubase-track-probe-collector-output")
        .map_err(|error| format!("could not initialize MIDI output: {error}"))?
        .create_virtual(TO_CUBASE_PORT)
        .map_err(|error| format!("could not create MIDI output '{TO_CUBASE_PORT}': {error}"))?;

    let mut input = MidiInput::new("cubase-track-probe-collector-input")
        .map_err(|error| format!("could not initialize MIDI input: {error}"))?;
    input.ignore(Ignore::None);
    let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
    let dropped_items = Arc::new(AtomicU64::new(0));
    let ingress_progress = Arc::new(IngressProgress::default());
    let input = input
        .create_virtual(
            FROM_CUBASE_PORT,
            receive_midi,
            MidiCallbackState {
                sender,
                framer: SysexFramer::default(),
                dropped_items: Arc::clone(&dropped_items),
                integrity_failed,
                ingress_progress: Arc::clone(&ingress_progress),
            },
        )
        .map_err(|error| format!("could not create MIDI input '{FROM_CUBASE_PORT}': {error}"))?;

    Ok(MidiConnections {
        output,
        input,
        receiver,
        dropped_items,
        ingress_progress,
        mode: "virtual",
        resolved_input_port: FROM_CUBASE_PORT.into(),
        resolved_output_port: TO_CUBASE_PORT.into(),
    })
}

fn connect_existing_ports(
    input_name: &str,
    output_name: &str,
    integrity_failed: Arc<AtomicBool>,
) -> Result<MidiConnections, String> {
    let mut input = MidiInput::new("cubase-track-probe-collector-input")
        .map_err(|error| format!("could not initialize MIDI input: {error}"))?;
    input.ignore(Ignore::None);
    let (input_port, resolved_input_port) = find_input_port(&input, input_name)?;

    let output = MidiOutput::new("cubase-track-probe-collector-output")
        .map_err(|error| format!("could not initialize MIDI output: {error}"))?;
    let (output_port, resolved_output_port) = find_output_port(&output, output_name)?;

    let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
    let dropped_items = Arc::new(AtomicU64::new(0));
    let ingress_progress = Arc::new(IngressProgress::default());
    let input = input
        .connect(
            &input_port,
            "cubase-track-probe-collector-input-connection",
            receive_midi,
            MidiCallbackState {
                sender,
                framer: SysexFramer::default(),
                dropped_items: Arc::clone(&dropped_items),
                integrity_failed,
                ingress_progress: Arc::clone(&ingress_progress),
            },
        )
        .map_err(|error| format!("could not connect MIDI input '{input_name}': {error}"))?;
    let output = output
        .connect(
            &output_port,
            "cubase-track-probe-collector-output-connection",
        )
        .map_err(|error| format!("could not connect MIDI output '{output_name}': {error}"))?;

    Ok(MidiConnections {
        output,
        input,
        receiver,
        dropped_items,
        ingress_progress,
        mode: "existing",
        resolved_input_port,
        resolved_output_port,
    })
}

fn find_input_port(input: &MidiInput, requested: &str) -> Result<(MidiInputPort, String), String> {
    select_port(
        requested,
        input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok().map(|name| (port.clone(), name))),
        "input",
    )
}

fn find_output_port(
    output: &MidiOutput,
    requested: &str,
) -> Result<(MidiOutputPort, String), String> {
    select_port(
        requested,
        output
            .ports()
            .iter()
            .filter_map(|port| output.port_name(port).ok().map(|name| (port.clone(), name))),
        "output",
    )
}

fn select_port<T>(
    requested: &str,
    ports: impl Iterator<Item = (T, String)>,
    direction: &str,
) -> Result<(T, String), String> {
    let mut exact = None;
    let mut partial = Vec::new();
    let requested_lower = requested.to_lowercase();

    for (port, name) in ports {
        if name == requested {
            exact = Some((port, name));
            break;
        }
        if name.to_lowercase().contains(&requested_lower) {
            partial.push((port, name));
        }
    }

    if let Some(port) = exact {
        return Ok(port);
    }
    match partial.len() {
        0 => Err(format!(
            "MIDI {direction} port matching '{requested}' was not found"
        )),
        1 => Ok(partial.remove(0)),
        _ => Err(format!(
            "MIDI {direction} port name '{requested}' is ambiguous"
        )),
    }
}

struct MidiCallbackState {
    sender: SyncSender<Ingress>,
    framer: SysexFramer,
    dropped_items: Arc<AtomicU64>,
    integrity_failed: Arc<AtomicBool>,
    ingress_progress: Arc<IngressProgress>,
}

#[derive(Default)]
struct IngressProgressState {
    active_callbacks: u64,
    processed: u64,
    partial_frame: bool,
}

#[derive(Default)]
struct IngressProgress {
    // Held only to admit/snapshot callbacks and for the final tracker+MIDI-send cut.
    // Queue draining and JSONL writes happen without this gate so the MIDI callback
    // cannot be blocked by backlog or output backpressure.
    callback_order: Mutex<()>,
    state: Mutex<IngressProgressState>,
    received: AtomicU64,
    changed: Condvar,
}

struct IngressCallbackActivity<'a> {
    progress: &'a IngressProgress,
    integrity_failed: &'a AtomicBool,
}

struct IngressBarrier<'a> {
    _callback_order: MutexGuard<'a, ()>,
    boundary_time: Instant,
}

impl IngressBarrier<'_> {
    fn boundary_time(&self) -> Instant {
        self.boundary_time
    }
}

impl IngressProgress {
    fn begin_callback<'a>(
        &'a self,
        integrity_failed: &'a AtomicBool,
    ) -> Option<IngressCallbackActivity<'a>> {
        let _order = self.callback_order.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        let mut state = self.state.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        let Some(active_callbacks) = state.active_callbacks.checked_add(1) else {
            integrity_failed.store(true, Ordering::Release);
            self.changed.notify_all();
            return None;
        };
        state.active_callbacks = active_callbacks;
        Some(IngressCallbackActivity {
            progress: self,
            integrity_failed,
        })
    }

    fn reserve_received(&self, integrity_failed: &AtomicBool) -> bool {
        if self
            .received
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |received| {
                received.checked_add(1)
            })
            .is_ok()
        {
            true
        } else {
            integrity_failed.store(true, Ordering::Release);
            self.changed.notify_all();
            false
        }
    }

    fn mark_processed(&self, integrity_failed: &AtomicBool) {
        let mut state = self.state.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        let Some(processed) = state.processed.checked_add(1) else {
            integrity_failed.store(true, Ordering::Release);
            self.changed.notify_all();
            return;
        };
        state.processed = processed;
        if processed > self.received.load(Ordering::Acquire) {
            integrity_failed.store(true, Ordering::Release);
        }
        self.changed.notify_all();
    }

    fn set_partial_frame(&self, partial_frame: bool, integrity_failed: &AtomicBool) {
        let mut state = self.state.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        state.partial_frame = partial_frame;
        self.changed.notify_all();
    }

    fn synchronize_until(
        &self,
        integrity_failed: &AtomicBool,
        deadline: Instant,
    ) -> Result<Instant, TrackerFault> {
        let (boundary, boundary_time) = loop {
            let order = self.callback_order.lock().unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
            let mut state = self.state.lock().unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
            while state.active_callbacks > 0 {
                if integrity_failed.load(Ordering::Acquire) {
                    return Err(TrackerFault::new(
                        "INGRESS_BARRIER_INTEGRITY_FAILURE",
                        "MIDI ingress integrity failed while waiting for an active callback",
                        json!({"active_callbacks": state.active_callbacks}),
                    ));
                }
                let now = Instant::now();
                if now >= deadline {
                    integrity_failed.store(true, Ordering::Release);
                    return Err(TrackerFault::new(
                        "INGRESS_BARRIER_TIMEOUT",
                        "an active MIDI callback did not reach the receive barrier deadline",
                        json!({"timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT)}),
                    ));
                }
                let wait = deadline.saturating_duration_since(now);
                let (next_state, _) =
                    self.changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(|error| {
                            integrity_failed.store(true, Ordering::Release);
                            error.into_inner()
                        });
                state = next_state;
            }
            if !state.partial_frame {
                let boundary = self.received.load(Ordering::Acquire);
                let boundary_time = Instant::now();
                drop(state);
                drop(order);
                break (boundary, boundary_time);
            }

            // The next MIDI callback must be admitted to finish the split SysEx,
            // so never retain callback_order while waiting for a partial frame.
            drop(state);
            drop(order);
            let mut state = self.state.lock().unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
            while state.partial_frame {
                if integrity_failed.load(Ordering::Acquire) {
                    return Err(TrackerFault::new(
                        "INGRESS_BARRIER_INTEGRITY_FAILURE",
                        "MIDI ingress integrity failed while a SysEx frame was partial",
                        json!({"partial_frame": true}),
                    ));
                }
                let now = Instant::now();
                if now >= deadline {
                    integrity_failed.store(true, Ordering::Release);
                    return Err(TrackerFault::new(
                        "INGRESS_PARTIAL_FRAME_TIMEOUT",
                        "a split SysEx frame did not complete before the ingress barrier deadline",
                        json!({"timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT)}),
                    ));
                }
                let wait = deadline.saturating_duration_since(now);
                let (next_state, _) =
                    self.changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(|error| {
                            integrity_failed.store(true, Ordering::Release);
                            error.into_inner()
                        });
                state = next_state;
            }
        };
        let mut state = self.state.lock().unwrap_or_else(|error| {
            integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        while state.processed < boundary {
            if integrity_failed.load(Ordering::Acquire) {
                return Err(TrackerFault::new(
                    "INGRESS_BARRIER_INTEGRITY_FAILURE",
                    "MIDI ingress integrity failed before all received items were processed",
                    json!({
                        "received_boundary": boundary,
                        "processed": state.processed
                    }),
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                integrity_failed.store(true, Ordering::Release);
                return Err(TrackerFault::new(
                    "INGRESS_BARRIER_TIMEOUT",
                    "received MIDI ingress did not reach the protocol tracker before the deadline",
                    json!({
                        "timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT),
                        "received_boundary": boundary,
                        "processed": state.processed
                    }),
                ));
            }
            let wait = deadline.saturating_duration_since(now);
            let (next_state, _) = self
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
            state = next_state;
        }
        if integrity_failed.load(Ordering::Acquire) {
            return Err(TrackerFault::new(
                "INGRESS_BARRIER_INTEGRITY_FAILURE",
                "MIDI ingress integrity failed at the receive barrier",
                json!({
                    "received_boundary": boundary,
                    "processed": state.processed
                }),
            ));
        }
        Ok(boundary_time)
    }

    fn synchronize(&self, integrity_failed: &AtomicBool) -> Result<Instant, TrackerFault> {
        self.synchronize_until(integrity_failed, Instant::now() + INGRESS_BARRIER_TIMEOUT)
    }

    fn synchronize_held<'a>(
        &'a self,
        integrity_failed: &AtomicBool,
    ) -> Result<IngressBarrier<'a>, TrackerFault> {
        let deadline = Instant::now() + INGRESS_BARRIER_TIMEOUT;
        loop {
            // Drain without holding callback_order, then close the short race only
            // when both the callback and collector sides are caught up.
            self.synchronize_until(integrity_failed, deadline)?;
            let order = self.callback_order.lock().unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
            let mut state = self.state.lock().unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
            while state.active_callbacks > 0 {
                if integrity_failed.load(Ordering::Acquire) {
                    return Err(TrackerFault::new(
                        "INGRESS_BARRIER_INTEGRITY_FAILURE",
                        "MIDI ingress integrity failed while closing the command barrier",
                        json!({"active_callbacks": state.active_callbacks}),
                    ));
                }
                let now = Instant::now();
                if now >= deadline {
                    integrity_failed.store(true, Ordering::Release);
                    return Err(TrackerFault::new(
                        "INGRESS_BARRIER_TIMEOUT",
                        "MIDI ingress did not quiesce before the command barrier deadline",
                        json!({"timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT)}),
                    ));
                }
                let wait = deadline.saturating_duration_since(now);
                let (next_state, _) =
                    self.changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(|error| {
                            integrity_failed.store(true, Ordering::Release);
                            error.into_inner()
                        });
                state = next_state;
            }
            let received = self.received.load(Ordering::Acquire);
            if state.processed == received
                && !state.partial_frame
                && !integrity_failed.load(Ordering::Acquire)
            {
                let boundary_time = Instant::now();
                drop(state);
                return Ok(IngressBarrier {
                    _callback_order: order,
                    boundary_time,
                });
            }
            let partial_frame = state.partial_frame;
            drop(state);
            drop(order);
            if integrity_failed.load(Ordering::Acquire) {
                return Err(TrackerFault::new(
                    "INGRESS_BARRIER_INTEGRITY_FAILURE",
                    "MIDI ingress integrity failed while closing the command barrier",
                    json!({
                        "received": received,
                        "partial_frame": partial_frame
                    }),
                ));
            }
            if Instant::now() >= deadline {
                integrity_failed.store(true, Ordering::Release);
                return Err(TrackerFault::new(
                    "INGRESS_BARRIER_TIMEOUT",
                    "MIDI ingress remained active before the command barrier deadline",
                    json!({
                        "timeout_ms": duration_ms(INGRESS_BARRIER_TIMEOUT),
                        "received": received
                    }),
                ));
            }
        }
    }
}

impl Drop for IngressCallbackActivity<'_> {
    fn drop(&mut self) {
        let mut state = self.progress.state.lock().unwrap_or_else(|error| {
            self.integrity_failed.store(true, Ordering::Release);
            error.into_inner()
        });
        let Some(active_callbacks) = state.active_callbacks.checked_sub(1) else {
            self.integrity_failed.store(true, Ordering::Release);
            self.progress.changed.notify_all();
            return;
        };
        state.active_callbacks = active_callbacks;
        self.progress.changed.notify_all();
    }
}

#[derive(Debug)]
enum Ingress {
    Frame {
        received_at_unix_ms: u64,
        received_at_monotonic: Instant,
        midi_timestamp: u64,
        bytes: Vec<u8>,
    },
    FramingFault {
        received_at_unix_ms: u64,
        received_at_monotonic: Instant,
        fault: FramingFault,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FramingFault {
    NestedStart,
    Oversize,
    TruncatedAtShutdown,
}

fn receive_midi(timestamp: u64, message: &[u8], state: &mut MidiCallbackState) {
    let Some(_activity) = state
        .ingress_progress
        .begin_callback(&state.integrity_failed)
    else {
        return;
    };
    for item in state.framer.push(message) {
        if matches!(item, FramerItem::Fault(_)) {
            state.integrity_failed.store(true, Ordering::Release);
        }
        let ingress = match item {
            FramerItem::Frame(bytes) if bytes.as_slice() == MIDI_BROADCAST_IDENTITY_REQUEST => {
                continue;
            }
            FramerItem::Frame(bytes) => Ingress::Frame {
                received_at_unix_ms: unix_timestamp_ms(),
                received_at_monotonic: Instant::now(),
                midi_timestamp: timestamp,
                bytes,
            },
            FramerItem::Fault(fault) => Ingress::FramingFault {
                received_at_unix_ms: unix_timestamp_ms(),
                received_at_monotonic: Instant::now(),
                fault,
            },
        };
        enqueue_ingress(
            &state.sender,
            &state.dropped_items,
            &state.integrity_failed,
            &state.ingress_progress,
            ingress,
        );
    }
    state
        .ingress_progress
        .set_partial_frame(state.framer.has_partial_frame(), &state.integrity_failed);
}

fn enqueue_ingress(
    sender: &SyncSender<Ingress>,
    dropped_items: &AtomicU64,
    integrity_failed: &AtomicBool,
    ingress_progress: &IngressProgress,
    ingress: Ingress,
) {
    if !ingress_progress.reserve_received(integrity_failed) {
        dropped_items.fetch_add(1, Ordering::AcqRel);
        return;
    }
    match sender.try_send(ingress) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            integrity_failed.store(true, Ordering::Release);
            dropped_items.fetch_add(1, Ordering::AcqRel);
            ingress_progress.changed.notify_all();
        }
        Err(TrySendError::Disconnected(_)) => {
            integrity_failed.store(true, Ordering::Release);
            dropped_items.fetch_add(1, Ordering::AcqRel);
            ingress_progress.changed.notify_all();
        }
    }
}

#[derive(Default)]
struct SysexFramer {
    frame: Vec<u8>,
    receiving: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum FramerItem {
    Frame(Vec<u8>),
    Fault(FramingFault),
}

impl SysexFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<FramerItem> {
        let mut items = Vec::new();
        for byte in bytes.iter().copied() {
            if byte == 0xF0 {
                if self.receiving {
                    items.push(FramerItem::Fault(FramingFault::NestedStart));
                }
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
                items.push(FramerItem::Fault(FramingFault::Oversize));
                continue;
            }
            if byte == 0xF7 {
                self.receiving = false;
                items.push(FramerItem::Frame(std::mem::take(&mut self.frame)));
            }
        }
        items
    }

    fn has_partial_frame(&self) -> bool {
        self.receiving || !self.frame.is_empty()
    }
}

#[derive(Debug)]
struct CodecError {
    code: &'static str,
    message: String,
    oversize: bool,
}

impl CodecError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            oversize: false,
        }
    }

    fn oversize(message: impl Into<String>) -> Self {
        Self::oversize_with_code("OVERSIZE_FRAME", message)
    }

    fn oversize_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            oversize: true,
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeIncomingEnvelope {
    probe_transport_version: u32,
    source_instance_id: String,
    source_seq: u64,
    message: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Event,
    Response,
    Error,
}

impl MessageKind {
    fn record_type(self) -> &'static str {
        match self {
            Self::Event => "probe_event",
            Self::Response => "probe_response",
            Self::Error => "probe_error",
        }
    }
}

#[cfg(test)]
fn encode_sysex(value: &impl Serialize) -> Result<Vec<u8>, CodecError> {
    encode_sysex_with_limit(value, MAX_JSON_BYTES, "OVERSIZE_FRAME")
}

fn encode_request_sysex(value: &ProbeRequestEnvelope) -> Result<Vec<u8>, CodecError> {
    if value.target_instance_id.as_deref() == Some(SELECTED_TARGET_ALIAS) {
        return Err(CodecError::new(
            "UNRESOLVED_TARGET_ALIAS",
            "collector-local target alias must be resolved before MIDI encoding",
        ));
    }
    encode_sysex_with_limit(value, MAX_OUTBOUND_JSON_BYTES, "OVERSIZE_COMMAND")
}

fn encode_sysex_with_limit(
    value: &impl Serialize,
    maximum_json_bytes: usize,
    oversize_code: &'static str,
) -> Result<Vec<u8>, CodecError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CodecError::new("JSON_ENCODE_ERROR", error.to_string()))?;
    if bytes.len() > maximum_json_bytes {
        return Err(CodecError::oversize_with_code(
            oversize_code,
            format!(
                "encoded JSON is {} bytes; maximum is {maximum_json_bytes}",
                bytes.len()
            ),
        ));
    }

    let mut frame = Vec::with_capacity(PROBE_SYSEX_HEADER.len() + bytes.len() * 2 + 1);
    frame.extend_from_slice(&PROBE_SYSEX_HEADER);
    for byte in bytes {
        frame.push((byte >> 4) & 0x0F);
        frame.push(byte & 0x0F);
    }
    frame.push(0xF7);
    Ok(frame)
}

fn decode_json_value(frame: &[u8]) -> Result<Value, CodecError> {
    if frame.len() > MAX_SYSEX_BYTES {
        return Err(CodecError::oversize(format!(
            "SysEx frame is {} bytes; maximum is {MAX_SYSEX_BYTES}",
            frame.len()
        )));
    }
    if frame.len() < PROBE_SYSEX_HEADER.len() + 1
        || !frame.starts_with(&PROBE_SYSEX_HEADER)
        || frame.last() != Some(&0xF7)
    {
        return Err(CodecError::new(
            "INVALID_FRAME",
            "message does not use the Cubase Track Probe SysEx header",
        ));
    }

    let payload = &frame[PROBE_SYSEX_HEADER.len()..frame.len() - 1];
    if !payload.len().is_multiple_of(2) {
        return Err(CodecError::new(
            "ODD_NIBBLE_COUNT",
            "SysEx payload has an odd number of nibbles",
        ));
    }
    if payload.len() / 2 > MAX_JSON_BYTES {
        return Err(CodecError::oversize(
            "decoded JSON payload exceeds the 64 KiB limit",
        ));
    }

    let mut json_bytes = Vec::with_capacity(payload.len() / 2);
    for nibbles in payload.as_chunks::<2>().0 {
        if nibbles[0] > 0x0F || nibbles[1] > 0x0F {
            return Err(CodecError::new(
                "NON_NIBBLE_BYTE",
                "SysEx payload contains a byte outside 0x00..0x0F",
            ));
        }
        json_bytes.push((nibbles[0] << 4) | nibbles[1]);
    }

    serde_json::from_slice(&json_bytes)
        .map_err(|error| CodecError::new("INVALID_JSON", error.to_string()))
}

fn decode_incoming(frame: &[u8]) -> Result<(ProbeIncomingEnvelope, MessageKind), CodecError> {
    let value = decode_json_value(frame)?;
    let envelope: ProbeIncomingEnvelope = serde_json::from_value(value)
        .map_err(|error| CodecError::new("INVALID_ENVELOPE", error.to_string()))?;

    if envelope.probe_transport_version != PROBE_TRANSPORT_VERSION {
        return Err(CodecError::new(
            "UNSUPPORTED_TRANSPORT_VERSION",
            format!(
                "received probe transport version {}; expected {PROBE_TRANSPORT_VERSION}",
                envelope.probe_transport_version
            ),
        ));
    }
    if envelope.source_instance_id.is_empty()
        || envelope.source_instance_id.len() > MAX_INSTANCE_ID_BYTES
    {
        return Err(CodecError::new(
            "INVALID_SOURCE_INSTANCE_ID",
            format!("source_instance_id must contain 1 to {MAX_INSTANCE_ID_BYTES} UTF-8 bytes"),
        ));
    }
    if envelope.source_seq == 0 {
        return Err(CodecError::new(
            "INVALID_SOURCE_SEQUENCE",
            "source_seq must be a positive integer",
        ));
    }

    let kind = validate_probe_message(&envelope.message)?;
    Ok((envelope, kind))
}

fn validate_probe_message(message: &Value) -> Result<MessageKind, CodecError> {
    let object = message
        .as_object()
        .ok_or_else(|| CodecError::new("INVALID_MESSAGE", "probe message must be a JSON object"))?;
    if object.get("version").and_then(Value::as_u64) != Some(u64::from(PROBE_MESSAGE_VERSION)) {
        return Err(CodecError::new(
            "UNSUPPORTED_MESSAGE_VERSION",
            format!("probe message version must be {PROBE_MESSAGE_VERSION}"),
        ));
    }
    let message_type = required_nonempty_string(object, "type", MAX_METHOD_BYTES)?;
    match message_type {
        "event" => {
            required_nonempty_string(object, "event", MAX_METHOD_BYTES)?;
            if !object.get("data").is_some_and(Value::is_object) {
                return Err(CodecError::new(
                    "INVALID_EVENT",
                    "event data must be an object",
                ));
            }
            Ok(MessageKind::Event)
        }
        "response" => {
            required_nonempty_string(object, "id", MAX_REQUEST_ID_BYTES)?;
            if !object.get("result").is_some_and(Value::is_object) {
                return Err(CodecError::new(
                    "INVALID_RESPONSE",
                    "response result must be an object",
                ));
            }
            Ok(MessageKind::Response)
        }
        "error" => {
            required_nonempty_string(object, "id", MAX_REQUEST_ID_BYTES)?;
            let error = object
                .get("error")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    CodecError::new("INVALID_ERROR", "error payload must be an object")
                })?;
            required_nonempty_string(error, "code", MAX_METHOD_BYTES)?;
            required_nonempty_string(error, "message", MAX_JSON_BYTES)?;
            Ok(MessageKind::Error)
        }
        _ => Err(CodecError::new(
            "INVALID_MESSAGE_TYPE",
            format!("unsupported probe message type '{message_type}'"),
        )),
    }
}

fn source_overflow_data(kind: MessageKind, message: &Value) -> Option<&Value> {
    if kind == MessageKind::Event
        && message.get("event").and_then(Value::as_str) == Some("probe.overflow")
    {
        message.get("data")
    } else {
        None
    }
}

fn required_nonempty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    max_bytes: usize,
) -> Result<&'a str, CodecError> {
    let value = object.get(field).and_then(Value::as_str).ok_or_else(|| {
        CodecError::new(
            "INVALID_MESSAGE",
            format!("probe message field '{field}' must be a string"),
        )
    })?;
    if value.is_empty() || value.len() > max_bytes {
        return Err(CodecError::new(
            "INVALID_MESSAGE",
            format!("probe message field '{field}' has an invalid UTF-8 byte length"),
        ));
    }
    Ok(value)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StdinCommand {
    target_instance_id: Option<String>,
    method: String,
    #[serde(default = "empty_object")]
    params: Value,
}

#[derive(Debug)]
enum ParsedCommand {
    Probe(ProbeRequestEnvelope),
    CheckpointBegin {
        checkpoint_id: String,
        window: Duration,
    },
    Action {
        checkpoint_id: String,
    },
    CheckpointEnd {
        checkpoint_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointBeginParams {
    checkpoint_id: String,
    window_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointEndParams {
    checkpoint_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionParams {
    checkpoint_id: String,
}

fn empty_object() -> Value {
    json!({})
}

#[derive(Debug, Serialize)]
struct ProbeRequestEnvelope {
    probe_transport_version: u32,
    target_instance_id: Option<String>,
    message: ProbeRequest,
}

#[derive(Debug, Serialize)]
struct ProbeRequest {
    version: u32,
    id: String,
    #[serde(rename = "type")]
    message_type: &'static str,
    method: String,
    params: Value,
}

#[derive(Debug)]
struct CommandError {
    code: &'static str,
    message: String,
}

impl CommandError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn parse_command(
    bytes: &[u8],
    session_id: &str,
    command_sequence: u64,
) -> Result<ParsedCommand, CommandError> {
    let command: StdinCommand = serde_json::from_slice(bytes)
        .map_err(|error| CommandError::new("INVALID_COMMAND_JSON", error.to_string()))?;
    if command.method == "collector.checkpoint.begin" {
        if command.target_instance_id.is_some() {
            return Err(CommandError::new(
                "INVALID_COMMAND_TARGET",
                "local collector checkpoint commands must not specify target_instance_id",
            ));
        }
        let params: CheckpointBeginParams = serde_json::from_value(command.params)
            .map_err(|error| CommandError::new("INVALID_CHECKPOINT_PARAMS", error.to_string()))?;
        validate_checkpoint_id(&params.checkpoint_id)?;
        let window_ms = parse_wait_ms(&params.window_ms.to_string(), "checkpoint window_ms")
            .map_err(|message| CommandError::new("INVALID_CHECKPOINT_WINDOW", message))?;
        return Ok(ParsedCommand::CheckpointBegin {
            checkpoint_id: params.checkpoint_id,
            window: Duration::from_millis(window_ms),
        });
    }
    if command.method == "collector.checkpoint.end" {
        if command.target_instance_id.is_some() {
            return Err(CommandError::new(
                "INVALID_COMMAND_TARGET",
                "local collector checkpoint commands must not specify target_instance_id",
            ));
        }
        let params: CheckpointEndParams = serde_json::from_value(command.params)
            .map_err(|error| CommandError::new("INVALID_CHECKPOINT_PARAMS", error.to_string()))?;
        validate_checkpoint_id(&params.checkpoint_id)?;
        return Ok(ParsedCommand::CheckpointEnd {
            checkpoint_id: params.checkpoint_id,
        });
    }
    if command.method == "collector.action" {
        if command.target_instance_id.is_some() {
            return Err(CommandError::new(
                "INVALID_COMMAND_TARGET",
                "local collector action commands must not specify target_instance_id",
            ));
        }
        let params: ActionParams = serde_json::from_value(command.params)
            .map_err(|error| CommandError::new("INVALID_ACTION_PARAMS", error.to_string()))?;
        validate_checkpoint_id(&params.checkpoint_id)?;
        return Ok(ParsedCommand::Action {
            checkpoint_id: params.checkpoint_id,
        });
    }
    if command.method.is_empty()
        || command.method.len() > MAX_METHOD_BYTES
        || !command.method.starts_with("probe.")
    {
        return Err(CommandError::new(
            "INVALID_COMMAND_METHOD",
            format!(
                "method must start with 'probe.' and contain 1 to {MAX_METHOD_BYTES} UTF-8 bytes"
            ),
        ));
    }
    if !command.params.is_object() {
        return Err(CommandError::new(
            "INVALID_COMMAND_PARAMS",
            "params must be an object",
        ));
    }
    match (&*command.method, &command.target_instance_id) {
        ("probe.discover", None) => {}
        ("probe.discover", Some(_)) => {
            return Err(CommandError::new(
                "INVALID_COMMAND_TARGET",
                "probe.discover must use a null or omitted target_instance_id",
            ));
        }
        (_, Some(target)) if !target.is_empty() && target.len() <= MAX_INSTANCE_ID_BYTES => {}
        _ => {
            return Err(CommandError::new(
                "INVALID_COMMAND_TARGET",
                format!(
                    "non-discovery commands require a target_instance_id of 1 to {MAX_INSTANCE_ID_BYTES} UTF-8 bytes"
                ),
            ));
        }
    }

    Ok(ParsedCommand::Probe(ProbeRequestEnvelope {
        probe_transport_version: PROBE_TRANSPORT_VERSION,
        target_instance_id: command.target_instance_id,
        message: ProbeRequest {
            version: PROBE_MESSAGE_VERSION,
            id: format!("{session_id}-{command_sequence}"),
            message_type: "request",
            method: command.method,
            params: command.params,
        },
    }))
}

fn validate_checkpoint_id(checkpoint_id: &str) -> Result<(), CommandError> {
    if checkpoint_id.is_empty() || checkpoint_id.len() > MAX_CHECKPOINT_ID_BYTES {
        return Err(CommandError::new(
            "INVALID_CHECKPOINT_ID",
            format!("checkpoint_id must contain 1 to {MAX_CHECKPOINT_ID_BYTES} UTF-8 bytes"),
        ));
    }
    Ok(())
}

#[derive(Default)]
struct CommandReport {
    received: u64,
    sent: u64,
    local_commands: u64,
    deferred: u64,
    rejected: u64,
    input_failed: bool,
    exit_reason: &'static str,
}

struct CommandEnvironment<'a> {
    integrity_failed: &'a AtomicBool,
    sink: &'a JsonlSink,
    runtime: &'a RuntimeTracker,
    ingress_progress: &'a IngressProgress,
    discovery_window: Duration,
}

fn process_stdin_commands(
    mut reader: impl BufRead,
    output: &mut MidiOutputConnection,
    session_id: &str,
    environment: CommandEnvironment<'_>,
) -> CommandReport {
    let CommandEnvironment {
        integrity_failed,
        sink,
        runtime,
        ingress_progress,
        discovery_window,
    } = environment;
    let mut report = CommandReport {
        exit_reason: "stdin_eof",
        ..CommandReport::default()
    };
    let mut command_sequence = 1_u64;

    loop {
        let bytes = match read_bounded_line(&mut reader, MAX_STDIN_COMMAND_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(error) => {
                report.input_failed = true;
                report.exit_reason = "stdin_error";
                let _ = emit_diagnostic(
                    sink,
                    "STDIN_READ_ERROR",
                    "error",
                    &format!("could not read command input: {error}"),
                    json!({}),
                );
                break;
            }
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        report.received += 1;

        if bytes.len() > MAX_STDIN_COMMAND_BYTES {
            report.rejected += 1;
            let _ = emit_diagnostic(
                sink,
                "OVERSIZE_COMMAND",
                "error",
                "stdin command exceeds the 64 KiB limit and was not sent",
                json!({"command_bytes_at_least": bytes.len()}),
            );
            continue;
        }

        let command = match parse_command(&bytes, session_id, command_sequence) {
            Ok(command) => command,
            Err(error) => {
                report.rejected += 1;
                let _ = emit_diagnostic(sink, error.code, "error", &error.message, json!({}));
                continue;
            }
        };

        match command {
            ParsedCommand::CheckpointBegin {
                checkpoint_id,
                window,
            } => {
                report.local_commands += 1;
                let now = Instant::now();
                let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
                match tracker.begin_checkpoint(checkpoint_id, window, now) {
                    Ok(marker) => {
                        if let Err(error) = sink.emit(&marker) {
                            report.input_failed = true;
                            report.exit_reason = "stdout_error";
                            integrity_failed.store(true, Ordering::Release);
                            eprintln!("could not write checkpoint begin marker: {error}");
                        }
                    }
                    Err(fault) => {
                        report.rejected += 1;
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                    }
                }
                drop(tracker);
                runtime.notify();
                if report.input_failed {
                    break;
                }
            }
            ParsedCommand::Action { checkpoint_id } => {
                report.local_commands += 1;
                let barrier = match ingress_progress.synchronize_held(integrity_failed) {
                    Ok(barrier) => barrier,
                    Err(fault) => {
                        report.rejected += 1;
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        continue;
                    }
                };
                let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
                let marker_result = tracker.mark_action(&checkpoint_id);
                // The barrier defines the action cut, but stdout must not hold up MIDI
                // callback admission. The operator performs the action only after this
                // synchronous marker emission returns.
                drop(tracker);
                drop(barrier);
                runtime.notify();
                match marker_result {
                    Ok(marker) => {
                        if let Err(error) = sink.emit(&marker) {
                            report.input_failed = true;
                            report.exit_reason = "stdout_error";
                            integrity_failed.store(true, Ordering::Release);
                            eprintln!("could not write collector action marker: {error}");
                        }
                    }
                    Err(fault) => {
                        report.rejected += 1;
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                    }
                }
                if report.input_failed {
                    break;
                }
            }
            ParsedCommand::CheckpointEnd { checkpoint_id } => {
                report.local_commands += 1;
                let barrier = match ingress_progress.synchronize_held(integrity_failed) {
                    Ok(barrier) => barrier,
                    Err(fault) => {
                        report.rejected += 1;
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        continue;
                    }
                };
                let now = barrier.boundary_time();
                let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
                let end_result = tracker.end_checkpoint(&checkpoint_id, now);
                drop(tracker);
                drop(barrier);
                runtime.notify();
                match end_result {
                    Ok((marker, fault)) => {
                        if let Err(error) = sink.emit(&marker) {
                            report.input_failed = true;
                            report.exit_reason = "stdout_error";
                            integrity_failed.store(true, Ordering::Release);
                            eprintln!("could not write checkpoint end marker: {error}");
                        }
                        if let Some(fault) = fault {
                            report.rejected += 1;
                            emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        }
                    }
                    Err(fault) => {
                        if fault.code == "CHECKPOINT_PROTOCOL_NOT_QUIESCENT" {
                            report.deferred += 1;
                            let _ = sink.emit(&json!({
                                "record_type": "collector_checkpoint",
                                "phase": "end_deferred",
                                "checkpoint_id": checkpoint_id,
                                "reason": fault.code,
                                "details": fault.details
                            }));
                        } else {
                            report.rejected += 1;
                            emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        }
                    }
                }
                if report.input_failed {
                    break;
                }
            }
            ParsedCommand::Probe(mut envelope) => {
                command_sequence = match command_sequence.checked_add(1) {
                    Some(next) => next,
                    None => {
                        report.input_failed = true;
                        report.exit_reason = "command_sequence_exhausted";
                        integrity_failed.store(true, Ordering::Release);
                        let _ = emit_diagnostic(
                            sink,
                            "COMMAND_SEQUENCE_EXHAUSTED",
                            "fatal",
                            "collector request id sequence is exhausted",
                            json!({}),
                        );
                        break;
                    }
                };

                if observation_integrity_failed(integrity_failed, sink) {
                    report.rejected += 1;
                    let _ = emit_diagnostic(
                        sink,
                        "COMMAND_BLOCKED_BY_INTEGRITY_FAILURE",
                        "error",
                        "observation integrity has failed; probe command was not sent",
                        json!({"request_id": envelope.message.id}),
                    );
                    continue;
                }

                let command_boundary = match ingress_progress.synchronize(integrity_failed) {
                    Ok(boundary) => boundary,
                    Err(fault) => {
                        report.rejected += 1;
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        continue;
                    }
                };
                if observation_integrity_failed(integrity_failed, sink) {
                    report.rejected += 1;
                    let _ = emit_diagnostic(
                        sink,
                        "COMMAND_BLOCKED_BY_INTEGRITY_FAILURE",
                        "error",
                        "observation integrity failed at the MIDI receive barrier; probe command was not sent",
                        json!({"request_id": envelope.message.id}),
                    );
                    continue;
                }

                let uses_selected_target_alias =
                    envelope.target_instance_id.as_deref() == Some(SELECTED_TARGET_ALIAS);
                let frame = if uses_selected_target_alias {
                    None
                } else {
                    match encode_request_sysex(&envelope) {
                        Ok(frame) => Some(frame),
                        Err(error) => {
                            report.rejected += 1;
                            let _ = emit_diagnostic(
                                sink,
                                error.code,
                                "error",
                                &error.message,
                                json!({"request_id": envelope.message.id}),
                            );
                            continue;
                        }
                    }
                };

                let request_id = envelope.message.id.clone();
                let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
                if let Err(fault) =
                    tracker.register_request(&envelope, command_boundary, discovery_window)
                {
                    report.rejected += 1;
                    let _ =
                        emit_diagnostic(sink, fault.code, "error", &fault.message, fault.details);
                    drop(tracker);
                    runtime.notify();
                    continue;
                }
                let request_checkpoint_id = tracker
                    .request_checkpoint_id(&request_id)
                    .expect("registered request has a checkpoint")
                    .to_owned();

                if !uses_selected_target_alias
                    && let Err(error) = sink.emit(&json!({
                        "record_type": "probe_command",
                        "phase": "started",
                        "request_id": request_id,
                        "checkpoint_id": request_checkpoint_id,
                        "request": &envelope
                    }))
                {
                    tracker.cancel_request(&request_id);
                    report.input_failed = true;
                    report.exit_reason = "stdout_error";
                    integrity_failed.store(true, Ordering::Release);
                    eprintln!("could not write probe command start record: {error}");
                    drop(tracker);
                    runtime.notify();
                    break;
                }
                drop(tracker);
                runtime.notify();

                let send_barrier = match ingress_progress.synchronize_held(integrity_failed) {
                    Ok(barrier) => barrier,
                    Err(fault) => {
                        let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                            integrity_failed.store(true, Ordering::Release);
                            error.into_inner()
                        });
                        tracker.cancel_request(&request_id);
                        drop(tracker);
                        runtime.notify();
                        report.rejected += 1;
                        let _ = sink.emit(&json!({
                            "record_type": "probe_command_send_result",
                            "request_id": request_id,
                            "checkpoint_id": request_checkpoint_id,
                            "sent": false,
                            "reason": "ingress_barrier_failure_before_send"
                        }));
                        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
                        continue;
                    }
                };
                let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                    integrity_failed.store(true, Ordering::Release);
                    error.into_inner()
                });
                let validation = if uses_selected_target_alias {
                    tracker.resolve_selected_target_alias_before_send(&mut envelope)
                } else {
                    tracker.validate_request_before_send(&request_id)
                };
                if observation_integrity_failed(integrity_failed, sink) || validation.is_err() {
                    tracker.cancel_request(&request_id);
                    drop(tracker);
                    drop(send_barrier);
                    runtime.notify();
                    report.rejected += 1;
                    let _ = sink.emit(&json!({
                        "record_type": "probe_command_send_result",
                        "request_id": request_id,
                        "checkpoint_id": request_checkpoint_id,
                        "sent": false,
                        "reason": if validation.is_err() {
                            "target_or_protocol_changed_before_send"
                        } else {
                            "integrity_failure_before_send"
                        }
                    }));
                    if let Err(fault) = validation {
                        let _ = emit_diagnostic(
                            sink,
                            fault.code,
                            "error",
                            &fault.message,
                            fault.details,
                        );
                    } else {
                        let _ = emit_diagnostic(
                            sink,
                            "COMMAND_BLOCKED_BY_INTEGRITY_FAILURE",
                            "error",
                            "observation integrity failed after command start; probe command was not sent",
                            json!({"request_id": request_id}),
                        );
                    }
                    continue;
                }

                if uses_selected_target_alias {
                    let frame = match encode_request_sysex(&envelope) {
                        Ok(frame) => frame,
                        Err(error) => {
                            tracker.cancel_request(&request_id);
                            drop(tracker);
                            drop(send_barrier);
                            runtime.notify();
                            report.rejected += 1;
                            let _ = sink.emit(&json!({
                                "record_type": "probe_command_send_result",
                                "request_id": request_id,
                                "checkpoint_id": request_checkpoint_id,
                                "sent": false,
                                "reason": "target_resolution_encoding_failure"
                            }));
                            let _ = emit_diagnostic(
                                sink,
                                error.code,
                                "error",
                                &error.message,
                                json!({"request_id": request_id}),
                            );
                            continue;
                        }
                    };
                    let command_record = json!({
                        "record_type": "probe_command",
                        "phase": "started",
                        "request_id": request_id,
                        "checkpoint_id": request_checkpoint_id,
                        "request": &envelope,
                        "evidence_emission": "after_midi_send_attempt"
                    });
                    let (send_result_record, midi_send_error) = match output.send(&frame) {
                        Ok(()) => {
                            let sent_at = Instant::now();
                            tracker.mark_request_sent(&request_id, sent_at, discovery_window);
                            report.sent += 1;
                            (
                                json!({
                                    "record_type": "probe_command_send_result",
                                    "request_id": request_id,
                                    "checkpoint_id": request_checkpoint_id,
                                    "sent": true,
                                    "sysex_bytes": frame.len(),
                                    "evidence_emission": "after_midi_send_attempt",
                                    "send_completed_monotonic_timestamp_ms": sink.monotonic_timestamp_at(sent_at)
                                }),
                                None,
                            )
                        }
                        Err(error) => {
                            let failed_at = Instant::now();
                            tracker.cancel_request(&request_id);
                            report.rejected += 1;
                            report.input_failed = true;
                            report.exit_reason = "midi_send_error";
                            integrity_failed.store(true, Ordering::Release);
                            (
                                json!({
                                    "record_type": "probe_command_send_result",
                                    "request_id": request_id,
                                    "checkpoint_id": request_checkpoint_id,
                                    "sent": false,
                                    "reason": "midi_send_error",
                                    "evidence_emission": "after_midi_send_attempt",
                                    "send_failed_monotonic_timestamp_ms": sink.monotonic_timestamp_at(failed_at)
                                }),
                                Some(error.to_string()),
                            )
                        }
                    };
                    drop(tracker);
                    drop(send_barrier);
                    runtime.notify();

                    if let Err(error) = sink.emit_pair(&command_record, &send_result_record) {
                        invalidate_after_selected_evidence_failure(
                            &mut report,
                            integrity_failed,
                            midi_send_error.is_some(),
                        );
                        eprintln!(
                            "could not write @selected probe evidence after the MIDI send attempt: {error}; the run is invalid and the request outcome must not be inferred from missing JSONL"
                        );
                    }
                    if let Some(error) = midi_send_error {
                        let _ = emit_diagnostic(
                            sink,
                            "MIDI_SEND_ERROR",
                            "fatal",
                            &format!("could not send probe request: {error}"),
                            json!({"request_id": request_id}),
                        );
                    }
                    if report.input_failed {
                        break;
                    }
                    continue;
                }

                let frame = frame.expect("validated probe request has an encoded MIDI frame");

                match output.send(&frame) {
                    Ok(()) => {
                        let sent_at = Instant::now();
                        tracker.mark_request_sent(&request_id, sent_at, discovery_window);
                        report.sent += 1;
                        drop(tracker);
                        drop(send_barrier);
                        runtime.notify();
                        if let Err(error) = sink.emit(&json!({
                            "record_type": "probe_command_send_result",
                            "request_id": request_id,
                            "checkpoint_id": request_checkpoint_id,
                            "sent": true,
                            "sysex_bytes": frame.len(),
                            "send_completed_monotonic_timestamp_ms": sink.monotonic_timestamp_at(sent_at)
                        })) {
                            report.input_failed = true;
                            report.exit_reason = "stdout_error";
                            integrity_failed.store(true, Ordering::Release);
                            eprintln!("could not write probe command send result: {error}");
                        }
                    }
                    Err(error) => {
                        tracker.cancel_request(&request_id);
                        drop(tracker);
                        drop(send_barrier);
                        runtime.notify();
                        report.rejected += 1;
                        report.input_failed = true;
                        report.exit_reason = "midi_send_error";
                        integrity_failed.store(true, Ordering::Release);
                        let _ = sink.emit(&json!({
                            "record_type": "probe_command_send_result",
                            "request_id": request_id,
                            "checkpoint_id": request_checkpoint_id,
                            "sent": false,
                            "reason": "midi_send_error"
                        }));
                        let _ = emit_diagnostic(
                            sink,
                            "MIDI_SEND_ERROR",
                            "fatal",
                            &format!("could not send probe request: {error}"),
                            json!({"request_id": request_id}),
                        );
                    }
                }
                if report.input_failed {
                    break;
                }
            }
        }
    }

    report
}

fn observation_integrity_failed(integrity_failed: &AtomicBool, sink: &JsonlSink) -> bool {
    integrity_failed.load(Ordering::Acquire) || sink.failed()
}

fn invalidate_after_selected_evidence_failure(
    report: &mut CommandReport,
    integrity_failed: &AtomicBool,
    midi_send_failed: bool,
) {
    if !midi_send_failed {
        report.input_failed = true;
        report.exit_reason = "stdout_error_after_midi_send";
    }
    integrity_failed.store(true, Ordering::Release);
}

fn emit_tracker_faults(
    faults: Vec<TrackerFault>,
    sink: &JsonlSink,
    integrity_failed: &AtomicBool,
    severity: &str,
) -> u64 {
    let count = u64::try_from(faults.len()).unwrap_or(u64::MAX);
    for fault in faults {
        integrity_failed.store(true, Ordering::Release);
        let _ = emit_diagnostic(sink, fault.code, severity, &fault.message, fault.details);
    }
    count
}

fn read_bounded_line(reader: &mut impl BufRead, maximum: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::with_capacity(maximum.min(4096) + 1);
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
            discard_through_newline(reader)?;
            return Ok(Some(bytes));
        }
    }
}

fn discard_through_newline(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        if let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            reader.consume(index + 1);
            return Ok(());
        }
        let length = buffer.len();
        reader.consume(length);
    }
}

#[derive(Debug)]
enum SequenceObservation {
    Contiguous,
    Gap { expected: u64, actual: u64 },
    DuplicateOrReorder { expected: u64, actual: u64 },
    TooManySources,
    Exhausted,
}

#[derive(Default)]
struct SequenceTracker {
    last_by_source: HashMap<String, u64>,
}

impl SequenceTracker {
    fn contains_source(&self, source: &str) -> bool {
        self.last_by_source.contains_key(source)
    }

    fn observe(&mut self, source: &str, actual: u64) -> SequenceObservation {
        let Some(last) = self.last_by_source.get(source).copied() else {
            if self.last_by_source.len() >= MAX_SOURCE_INSTANCES {
                return SequenceObservation::TooManySources;
            }
            self.last_by_source.insert(source.to_owned(), actual);
            return if actual == 1 {
                SequenceObservation::Contiguous
            } else {
                SequenceObservation::Gap {
                    expected: 1,
                    actual,
                }
            };
        };

        let Some(expected) = last.checked_add(1) else {
            return SequenceObservation::Exhausted;
        };
        if actual == expected {
            self.last_by_source.insert(source.to_owned(), actual);
            SequenceObservation::Contiguous
        } else if actual > expected {
            self.last_by_source.insert(source.to_owned(), actual);
            SequenceObservation::Gap { expected, actual }
        } else {
            SequenceObservation::DuplicateOrReorder { expected, actual }
        }
    }

    fn source_summaries(&self) -> Vec<Value> {
        let mut sources: Vec<_> = self.last_by_source.iter().collect();
        sources.sort_by_key(|(source, _)| *source);
        sources
            .into_iter()
            .map(|(source, last)| {
                json!({
                    "source_instance_id": source,
                    "last_source_seq": last
                })
            })
            .collect()
    }
}

struct RuntimeTracker {
    state: Mutex<ProtocolTracker>,
    changed: Condvar,
}

impl RuntimeTracker {
    fn new() -> Self {
        Self {
            state: Mutex::new(ProtocolTracker::default()),
            changed: Condvar::new(),
        }
    }

    fn notify(&self) {
        self.changed.notify_all();
    }
}

#[derive(Debug)]
struct TrackerFault {
    code: &'static str,
    message: String,
    details: Value,
}

impl TrackerFault {
    fn new(code: &'static str, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }
}

#[derive(Debug, Clone)]
enum FollowupTemplate {
    Bank { config_id: String, reason: String },
    DirectAccess,
}

impl FollowupTemplate {
    fn for_request(method: &str, params: &Value) -> Option<Self> {
        let bank_reason = match method {
            "probe.bank.reset" => Some("command_reset"),
            "probe.bank.next" => Some("command_next"),
            "probe.bank.prev" => Some("command_prev"),
            "probe.bank.snapshot" => Some("command_snapshot"),
            _ => None,
        };
        if let Some(reason) = bank_reason {
            return params
                .get("config_id")
                .and_then(Value::as_str)
                .map(|config_id| Self::Bank {
                    config_id: config_id.to_owned(),
                    reason: reason.into(),
                });
        }
        (method == "probe.direct_access.snapshot").then_some(Self::DirectAccess)
    }
}

#[derive(Debug)]
enum PendingMode {
    Targeted {
        target_instance_id: String,
    },
    Discovery {
        deadline: Option<Instant>,
        responders: HashSet<String>,
        observed_sources: HashSet<String>,
        valid: bool,
    },
}

#[derive(Debug)]
struct PendingRequest {
    method: String,
    mode: PendingMode,
    selected_target_alias: bool,
    followup: Option<FollowupTemplate>,
    checkpoint_id: String,
}

#[derive(Debug)]
struct ExpectedFollowup {
    request_id: String,
    source_instance_id: String,
    template: FollowupTemplate,
    checkpoint_id: String,
}

#[derive(Default)]
struct DiscoveryExpiry {
    faults: Vec<TrackerFault>,
    records: Vec<Value>,
    expired: usize,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SnapshotKey {
    source_instance_id: String,
    snapshot_id: String,
}

#[derive(Debug)]
struct HostIdAssembly {
    expected_bytes: usize,
    expected_fragments: usize,
    next_fragment: usize,
    observed_bytes: usize,
}

#[derive(Debug)]
struct ChunkStreamState {
    event_name: String,
    stream: String,
    reason: String,
    config_id: Option<String>,
    chunk_count: usize,
    total_items: usize,
    next_chunk: usize,
    observed_items: usize,
    host_ids: HashMap<String, HostIdAssembly>,
    stable_metadata: Map<String, Value>,
}

#[derive(Debug)]
struct ChunkData {
    snapshot_id: String,
    stream: String,
    reason: String,
    config_id: Option<String>,
    chunk_index: usize,
    chunk_count: usize,
    total_items: usize,
    items: Vec<Value>,
    snapshot_complete: bool,
    truncated: bool,
    overflow_safe: bool,
    stable_metadata: Map<String, Value>,
}

#[derive(Debug)]
struct ActiveCheckpoint {
    checkpoint_id: String,
    started_at: Instant,
    window: Duration,
    message_count: u64,
    last_message_received_at: Option<Instant>,
    action_marked: bool,
}

#[derive(Debug)]
struct CompletedCheckpoint {
    checkpoint_id: String,
    started_at: Instant,
    ended_at: Instant,
    window: Duration,
    message_count: u64,
    messages_processed_after_end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceLifecycle {
    AwaitingMapping,
    Initializing,
    Ready,
    Inactive,
    Invalid,
}

impl SourceLifecycle {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AwaitingMapping => "awaiting_mapping",
            Self::Initializing => "initializing",
            Self::Ready => "ready",
            Self::Inactive => "inactive",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug)]
struct CheckpointContext {
    checkpoint_id: Option<String>,
    orphan: bool,
    checkpoint_elapsed_ms: Option<u64>,
    checkpoint_window_ms: Option<u64>,
    checkpoint_window_expired: Option<bool>,
    processed_after_checkpoint_end: bool,
    checkpoint_quiet_period_violated: bool,
}

#[derive(Default)]
struct ProtocolTracker {
    pending_requests: HashMap<String, PendingRequest>,
    completed_request_ids: HashSet<String>,
    expected_followups: Vec<ExpectedFollowup>,
    open_snapshots: HashMap<SnapshotKey, ChunkStreamState>,
    completed_snapshots: HashSet<SnapshotKey>,
    completed_snapshot_streams: usize,
    completed_feedback_streams: usize,
    active_checkpoint: Option<ActiveCheckpoint>,
    completed_checkpoints: HashSet<String>,
    checkpoint_history: Vec<CompletedCheckpoint>,
    source_lifecycle: HashMap<String, SourceLifecycle>,
    selected_source_instance_id: Option<String>,
    orphan_messages: u64,
}

impl ProtocolTracker {
    fn register_request(
        &mut self,
        request: &ProbeRequestEnvelope,
        _now: Instant,
        _discovery_window: Duration,
    ) -> Result<(), TrackerFault> {
        let request_id = request.message.id.clone();
        if self.active_checkpoint.is_none() {
            return Err(TrackerFault::new(
                "PROBE_COMMAND_REQUIRES_CHECKPOINT",
                "probe commands must be sent inside an active collector checkpoint",
                json!({"request_id": request_id}),
            ));
        }
        if !self.pending_requests.is_empty()
            || !self.expected_followups.is_empty()
            || !self.open_snapshots.is_empty()
        {
            return Err(TrackerFault::new(
                "PROBE_COMMAND_NOT_SEQUENTIAL",
                "wait for the outstanding request and follow-up snapshot before sending another command",
                self.incomplete_details(),
            ));
        }
        if self.pending_requests.contains_key(&request_id)
            || self.completed_request_ids.contains(&request_id)
        {
            return Err(TrackerFault::new(
                "DUPLICATE_REQUEST_ID",
                "collector generated a duplicate request id",
                json!({"request_id": request_id}),
            ));
        }
        if self.pending_requests.len() + self.completed_request_ids.len() >= MAX_TRACKED_REQUESTS {
            return Err(TrackerFault::new(
                "REQUEST_TRACKER_CAPACITY",
                "request tracker reached its bounded capacity",
                json!({"maximum_requests": MAX_TRACKED_REQUESTS}),
            ));
        }

        let mut selected_target_alias = false;
        let mode = if request.message.method == "probe.discover" {
            self.selected_source_instance_id = None;
            PendingMode::Discovery {
                deadline: None,
                responders: HashSet::new(),
                observed_sources: HashSet::new(),
                valid: true,
            }
        } else {
            let requested_target_instance_id = request
                .target_instance_id
                .as_deref()
                .expect("validated targeted probe request has a target");
            let Some(selected_source_instance_id) = self.selected_source_instance_id.clone() else {
                return Err(TrackerFault::new(
                    "DISCOVERY_REQUIRED",
                    "targeted probe commands require a completed exactly-one discovery window",
                    json!({"request_id": request_id}),
                ));
            };
            let target_instance_id = if requested_target_instance_id == SELECTED_TARGET_ALIAS {
                selected_target_alias = true;
                selected_source_instance_id.clone()
            } else {
                requested_target_instance_id.to_owned()
            };
            if target_instance_id != selected_source_instance_id {
                return Err(TrackerFault::new(
                    "TARGET_NOT_DISCOVERED_SOURCE",
                    "target_instance_id does not match the source confirmed by discovery",
                    json!({
                        "request_id": request_id,
                        "target_instance_id": requested_target_instance_id,
                        "selected_source_instance_id": selected_source_instance_id
                    }),
                ));
            }
            if self.source_lifecycle.get(&target_instance_id) != Some(&SourceLifecycle::Ready) {
                return Err(TrackerFault::new(
                    "TARGET_SOURCE_NOT_ACTIVE",
                    "the discovered target source is no longer active",
                    json!({"target_instance_id": target_instance_id}),
                ));
            }
            PendingMode::Targeted { target_instance_id }
        };
        self.pending_requests.insert(
            request_id,
            PendingRequest {
                method: request.message.method.clone(),
                mode,
                selected_target_alias,
                followup: FollowupTemplate::for_request(
                    &request.message.method,
                    &request.message.params,
                ),
                checkpoint_id: self
                    .active_checkpoint
                    .as_ref()
                    .expect("active checkpoint validated")
                    .checkpoint_id
                    .clone(),
            },
        );
        Ok(())
    }

    fn cancel_request(&mut self, request_id: &str) {
        self.pending_requests.remove(request_id);
    }

    fn request_checkpoint_id(&self, request_id: &str) -> Option<&str> {
        self.pending_requests
            .get(request_id)
            .map(|request| request.checkpoint_id.as_str())
    }

    fn request_method(&self, request_id: &str) -> Option<&str> {
        self.pending_requests
            .get(request_id)
            .map(|request| request.method.as_str())
    }

    fn validate_request_before_send(&self, request_id: &str) -> Result<(), TrackerFault> {
        let Some(pending) = self.pending_requests.get(request_id) else {
            return Err(TrackerFault::new(
                "REQUEST_NOT_PENDING_BEFORE_SEND",
                "the registered request disappeared before MIDI send",
                json!({"request_id": request_id}),
            ));
        };
        if self.pending_requests.len() != 1
            || !self.expected_followups.is_empty()
            || !self.open_snapshots.is_empty()
        {
            return Err(TrackerFault::new(
                "PROBE_COMMAND_NOT_SEQUENTIAL_BEFORE_SEND",
                "protocol work arrived after command registration and before MIDI send",
                self.incomplete_details(),
            ));
        }
        if self
            .active_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str())
            != Some(pending.checkpoint_id.as_str())
        {
            return Err(TrackerFault::new(
                "REQUEST_CHECKPOINT_CHANGED_BEFORE_SEND",
                "the active checkpoint changed after command registration",
                json!({
                    "request_id": request_id,
                    "request_checkpoint_id": pending.checkpoint_id,
                    "active_checkpoint_id": self.active_checkpoint.as_ref().map(|checkpoint| &checkpoint.checkpoint_id)
                }),
            ));
        }
        if let PendingMode::Targeted { target_instance_id } = &pending.mode
            && (self.selected_source_instance_id.as_deref() != Some(target_instance_id)
                || self.source_lifecycle.get(target_instance_id) != Some(&SourceLifecycle::Ready))
        {
            return Err(TrackerFault::new(
                "TARGET_CHANGED_BEFORE_SEND",
                "the discovered target changed or became inactive before MIDI send",
                json!({
                    "request_id": request_id,
                    "target_instance_id": target_instance_id,
                    "selected_source_instance_id": self.selected_source_instance_id,
                    "target_lifecycle": self.source_lifecycle.get(target_instance_id).map(SourceLifecycle::as_str)
                }),
            ));
        }
        Ok(())
    }

    fn resolve_selected_target_alias_before_send(
        &self,
        request: &mut ProbeRequestEnvelope,
    ) -> Result<(), TrackerFault> {
        let request_id = request.message.id.as_str();
        self.validate_request_before_send(request_id)?;
        let pending = self
            .pending_requests
            .get(request_id)
            .expect("validated request remains pending");
        if !pending.selected_target_alias
            || request.target_instance_id.as_deref() != Some(SELECTED_TARGET_ALIAS)
            || request.message.method != pending.method
        {
            return Err(TrackerFault::new(
                "TARGET_ALIAS_STATE_MISMATCH",
                "the pending request does not match an unresolved selected-target alias",
                json!({"request_id": request_id}),
            ));
        }
        let PendingMode::Targeted { target_instance_id } = &pending.mode else {
            return Err(TrackerFault::new(
                "TARGET_ALIAS_STATE_MISMATCH",
                "a discovery request cannot use the selected-target alias",
                json!({"request_id": request_id}),
            ));
        };
        if target_instance_id == SELECTED_TARGET_ALIAS {
            return Err(TrackerFault::new(
                "TARGET_ALIAS_COLLISION",
                "the discovered source identifier collides with the reserved collector alias",
                json!({"request_id": request_id}),
            ));
        }
        request.target_instance_id = Some(target_instance_id.clone());
        Ok(())
    }

    fn mark_request_sent(
        &mut self,
        request_id: &str,
        sent_at: Instant,
        discovery_window: Duration,
    ) {
        if let Some(PendingRequest {
            mode: PendingMode::Discovery { deadline, .. },
            ..
        }) = self.pending_requests.get_mut(request_id)
        {
            *deadline = Some(sent_at + discovery_window);
        }
    }

    fn observe_reply_message(
        &mut self,
        source_instance_id: &str,
        request_id: &str,
        kind: MessageKind,
        message: &Value,
        checkpoint_id: Option<&str>,
    ) -> Vec<TrackerFault> {
        let mut faults = Vec::new();
        let source_ready =
            self.source_lifecycle.get(source_instance_id) == Some(&SourceLifecycle::Ready);
        let Some(pending) = self.pending_requests.get_mut(request_id) else {
            let (code, message) = if self.completed_request_ids.contains(request_id) {
                (
                    "DUPLICATE_RESPONSE",
                    "a completed request received another response or error",
                )
            } else {
                (
                    "UNMATCHED_RESPONSE",
                    "a response or error did not match a sent request",
                )
            };
            faults.push(TrackerFault::new(
                code,
                message,
                json!({
                    "request_id": request_id,
                    "source_instance_id": source_instance_id
                }),
            ));
            return faults;
        };
        if checkpoint_id != Some(pending.checkpoint_id.as_str()) {
            if let PendingMode::Discovery { valid, .. } = &mut pending.mode {
                *valid = false;
            }
            faults.push(TrackerFault::new(
                "RESPONSE_CHECKPOINT_MISMATCH",
                "response or error receive time is outside the request's checkpoint window",
                json!({
                    "request_id": request_id,
                    "request_checkpoint_id": pending.checkpoint_id,
                    "observed_checkpoint_id": checkpoint_id
                }),
            ));
            return faults;
        }

        match &mut pending.mode {
            PendingMode::Discovery {
                responders,
                observed_sources,
                valid,
                ..
            } => {
                if !observed_sources.contains(source_instance_id)
                    && observed_sources.len() >= MAX_SOURCE_INSTANCES
                {
                    *valid = false;
                    faults.push(TrackerFault::new(
                        "DISCOVERY_RESPONDER_CAPACITY",
                        "discovery responder tracking reached its bounded capacity",
                        json!({
                            "request_id": request_id,
                            "maximum_sources": MAX_SOURCE_INSTANCES
                        }),
                    ));
                    return faults;
                }
                if !observed_sources.insert(source_instance_id.to_owned()) {
                    *valid = false;
                    faults.push(TrackerFault::new(
                        "DUPLICATE_DISCOVERY_RESPONSE",
                        "one probe instance responded more than once in a discovery window",
                        json!({
                            "request_id": request_id,
                            "source_instance_id": source_instance_id
                        }),
                    ));
                }
                let response_valid = source_ready
                    && kind == MessageKind::Response
                    && message
                        .get("result")
                        .and_then(Value::as_object)
                        .is_some_and(|result| {
                            result.get("instance_id").and_then(Value::as_str)
                                == Some(source_instance_id)
                                && result.get("ready").and_then(Value::as_bool) == Some(true)
                                && result.get("read_only").and_then(Value::as_bool) == Some(true)
                        });
                if response_valid {
                    responders.insert(source_instance_id.to_owned());
                } else {
                    *valid = false;
                    faults.push(TrackerFault::new(
                        "DISCOVERY_RESPONSE_INVALID",
                        "discovery requires a ready read-only response whose instance_id matches its source",
                        json!({
                            "request_id": request_id,
                            "source_instance_id": source_instance_id
                        }),
                    ));
                }
            }
            PendingMode::Targeted { target_instance_id } => {
                if target_instance_id != source_instance_id {
                    faults.push(TrackerFault::new(
                        "RESPONSE_SOURCE_MISMATCH",
                        "targeted response came from a different probe instance",
                        json!({
                            "request_id": request_id,
                            "expected_source_instance_id": target_instance_id,
                            "actual_source_instance_id": source_instance_id
                        }),
                    ));
                    return faults;
                }

                let pending = self
                    .pending_requests
                    .remove(request_id)
                    .expect("targeted request still exists");
                self.completed_request_ids.insert(request_id.to_owned());
                if kind == MessageKind::Response
                    && let Some(template) = pending.followup
                {
                    self.expected_followups.push(ExpectedFollowup {
                        request_id: request_id.to_owned(),
                        source_instance_id: source_instance_id.to_owned(),
                        template,
                        checkpoint_id: pending.checkpoint_id,
                    });
                }
            }
        }
        faults
    }

    #[cfg(test)]
    fn observe_reply(
        &mut self,
        source_instance_id: &str,
        request_id: &str,
        kind: MessageKind,
    ) -> Vec<TrackerFault> {
        let message = match kind {
            MessageKind::Response => json!({
                "result": {
                    "instance_id": source_instance_id,
                    "ready": true,
                    "read_only": true
                }
            }),
            MessageKind::Error => json!({"error": {"code": "TEST", "message": "test"}}),
            MessageKind::Event => unreachable!("events are not replies"),
        };
        let checkpoint_id = self
            .pending_requests
            .get(request_id)
            .map(|request| request.checkpoint_id.clone());
        self.observe_reply_message(
            source_instance_id,
            request_id,
            kind,
            &message,
            checkpoint_id.as_deref(),
        )
    }

    fn expire_discoveries(&mut self, now: Instant) -> DiscoveryExpiry {
        let expired: Vec<String> = self
            .pending_requests
            .iter()
            .filter_map(|(request_id, pending)| match &pending.mode {
                PendingMode::Discovery {
                    deadline: Some(deadline),
                    ..
                } if now >= *deadline => Some(request_id.clone()),
                _ => None,
            })
            .collect();
        let mut result = DiscoveryExpiry::default();
        for request_id in expired {
            let pending = self
                .pending_requests
                .remove(&request_id)
                .expect("expired discovery request exists");
            let checkpoint_id = pending.checkpoint_id;
            let PendingMode::Discovery {
                responders,
                observed_sources,
                valid,
                ..
            } = pending.mode
            else {
                unreachable!("only discovery requests are expired here");
            };
            self.completed_request_ids.insert(request_id.clone());
            self.selected_source_instance_id = None;
            result.expired += 1;
            let mut source_instance_ids: Vec<_> = responders.into_iter().collect();
            source_instance_ids.sort();
            let mut observed_source_instance_ids: Vec<_> = observed_sources.into_iter().collect();
            observed_source_instance_ids.sort();
            let outcome = match (valid, source_instance_ids.len()) {
                (false, _) => "invalid_response",
                (true, 0) => {
                    result.faults.push(TrackerFault::new(
                        "DISCOVERY_NO_RESPONSE",
                        "no probe instance responded before the discovery window deadline",
                        json!({"request_id": request_id}),
                    ));
                    "no_response"
                }
                (true, 1) => {
                    let source_instance_id = source_instance_ids[0].clone();
                    if self.source_lifecycle.get(&source_instance_id)
                        == Some(&SourceLifecycle::Ready)
                    {
                        self.selected_source_instance_id = Some(source_instance_id.clone());
                        "selected"
                    } else {
                        result.faults.push(TrackerFault::new(
                            "DISCOVERY_RESPONDER_NOT_ACTIVE",
                            "the sole discovery responder is not an active loaded source",
                            json!({
                                "request_id": request_id,
                                "source_instance_id": source_instance_id
                            }),
                        ));
                        "inactive_responder"
                    }
                }
                (true, responder_count) => {
                    result.faults.push(TrackerFault::new(
                        "DISCOVERY_MULTIPLE_RESPONDERS",
                        "more than one probe instance responded in the discovery window",
                        json!({
                            "request_id": request_id,
                            "responder_count": responder_count,
                            "source_instance_ids": source_instance_ids
                        }),
                    ));
                    "multiple_responders"
                }
            };
            result.records.push(json!({
                "record_type": "collector_discovery_completed",
                "request_id": request_id,
                "checkpoint_id": checkpoint_id,
                "responder_count": source_instance_ids.len(),
                "source_instance_ids": source_instance_ids,
                "observed_source_instance_ids": observed_source_instance_ids,
                "selected_source_instance_id": self.selected_source_instance_id,
                "outcome": outcome,
                "window_closed": true
            }));
        }
        result
    }

    fn observe_source_message(
        &mut self,
        source_instance_id: &str,
        message: &Value,
        first_seen: bool,
    ) -> Vec<TrackerFault> {
        let mut faults = Vec::new();
        let is_loaded = message.get("type").and_then(Value::as_str) == Some("event")
            && message.get("event").and_then(Value::as_str) == Some("probe.loaded");

        if first_seen {
            self.selected_source_instance_id = None;
            if self.source_lifecycle.len() >= MAX_SOURCE_INSTANCES {
                faults.push(TrackerFault::new(
                    "SOURCE_LIFECYCLE_CAPACITY",
                    "source lifecycle tracker reached its bounded capacity",
                    json!({"maximum_sources": MAX_SOURCE_INSTANCES}),
                ));
                return faults;
            }
            if let Err(fault) = validate_first_loaded_message(source_instance_id, message) {
                self.source_lifecycle
                    .insert(source_instance_id.to_owned(), SourceLifecycle::Invalid);
                faults.push(fault);
                return faults;
            }
            let live_sources: Vec<_> = self
                .source_lifecycle
                .iter()
                .filter_map(|(source, lifecycle)| {
                    matches!(
                        *lifecycle,
                        SourceLifecycle::AwaitingMapping
                            | SourceLifecycle::Initializing
                            | SourceLifecycle::Ready
                    )
                    .then_some(source.clone())
                })
                .collect();
            self.source_lifecycle.insert(
                source_instance_id.to_owned(),
                SourceLifecycle::AwaitingMapping,
            );
            if !live_sources.is_empty() {
                faults.push(TrackerFault::new(
                    "MULTIPLE_ACTIVE_PROBE_SOURCES",
                    "a new probe source loaded while another source was still active",
                    json!({
                        "new_source_instance_id": source_instance_id,
                        "already_live_source_instance_ids": live_sources
                    }),
                ));
            }
            return faults;
        }

        if is_loaded {
            faults.push(TrackerFault::new(
                "PROBE_LOADED_REAPPEARED",
                "probe.loaded may only be the first message of a source session",
                json!({"source_instance_id": source_instance_id}),
            ));
        }
        let is_mapping_active = message.get("type").and_then(Value::as_str) == Some("event")
            && message.get("event").and_then(Value::as_str) == Some("probe.mapping_active");
        match self.source_lifecycle.get(source_instance_id).copied() {
            Some(SourceLifecycle::AwaitingMapping) | Some(SourceLifecycle::Inactive) => {
                if is_mapping_active {
                    match validate_mapping_active_message(source_instance_id, message) {
                        Ok(()) => {
                            self.selected_source_instance_id = None;
                            let other_live_sources: Vec<_> = self
                                .source_lifecycle
                                .iter()
                                .filter_map(|(source, lifecycle)| {
                                    (source != source_instance_id
                                        && matches!(
                                            *lifecycle,
                                            SourceLifecycle::AwaitingMapping
                                                | SourceLifecycle::Initializing
                                                | SourceLifecycle::Ready
                                        ))
                                    .then_some(source.clone())
                                })
                                .collect();
                            self.source_lifecycle.insert(
                                source_instance_id.to_owned(),
                                SourceLifecycle::Initializing,
                            );
                            if !other_live_sources.is_empty() {
                                faults.push(TrackerFault::new(
                                    "MULTIPLE_ACTIVE_PROBE_SOURCES",
                                    "a source mapping activated while another source was live",
                                    json!({
                                        "activated_source_instance_id": source_instance_id,
                                        "other_live_source_instance_ids": other_live_sources
                                    }),
                                ));
                            }
                        }
                        Err(fault) => faults.push(fault),
                    }
                    return faults;
                }
                let code = if self.source_lifecycle.get(source_instance_id)
                    == Some(&SourceLifecycle::Inactive)
                {
                    "INACTIVE_SOURCE_MESSAGE"
                } else {
                    "SOURCE_MAPPING_NOT_ACTIVE"
                };
                faults.push(TrackerFault::new(
                    code,
                    "only a valid probe.mapping_active event may follow load or probe.ready(false)",
                    json!({"source_instance_id": source_instance_id}),
                ));
                return faults;
            }
            Some(SourceLifecycle::Invalid) | None => {
                faults.push(TrackerFault::new(
                    "SOURCE_NOT_VALIDLY_LOADED",
                    "a message arrived from a source without a valid first probe.loaded event",
                    json!({"source_instance_id": source_instance_id}),
                ));
                return faults;
            }
            Some(SourceLifecycle::Initializing) | Some(SourceLifecycle::Ready) => {
                if is_mapping_active {
                    faults.push(TrackerFault::new(
                        "PROBE_MAPPING_ACTIVE_REAPPEARED",
                        "probe.mapping_active appeared while the source was already active",
                        json!({"source_instance_id": source_instance_id}),
                    ));
                    return faults;
                }
            }
        }

        if message.get("type").and_then(Value::as_str) == Some("event")
            && message.get("event").and_then(Value::as_str) == Some("probe.ready")
        {
            let ready = message
                .get("data")
                .and_then(|data| data.get("ready"))
                .and_then(Value::as_bool);
            match ready {
                Some(false) => {
                    if let Err(fault) = validate_ready_message(source_instance_id, message, false) {
                        faults.push(fault);
                    }
                    self.source_lifecycle
                        .insert(source_instance_id.to_owned(), SourceLifecycle::Inactive);
                    if self.selected_source_instance_id.as_deref() == Some(source_instance_id) {
                        self.selected_source_instance_id = None;
                    }
                }
                Some(true) => {
                    if self.source_lifecycle.get(source_instance_id)
                        == Some(&SourceLifecycle::Ready)
                    {
                        faults.push(TrackerFault::new(
                            "PROBE_READY_REAPPEARED",
                            "probe.ready(true) appeared after the source was already ready",
                            json!({"source_instance_id": source_instance_id}),
                        ));
                    } else if let Err(fault) =
                        validate_ready_message(source_instance_id, message, true)
                    {
                        faults.push(fault);
                    } else {
                        self.source_lifecycle
                            .insert(source_instance_id.to_owned(), SourceLifecycle::Ready);
                    }
                }
                None => faults.push(TrackerFault::new(
                    "PROBE_READY_STATE_INVALID",
                    "probe.ready data.ready must be a boolean",
                    json!({"source_instance_id": source_instance_id}),
                )),
            }
        }
        faults
    }

    fn observe_chunk_event_at(
        &mut self,
        source_instance_id: &str,
        event_name: &str,
        data: &Value,
        checkpoint_id: Option<&str>,
    ) -> Vec<TrackerFault> {
        let mut faults = Vec::new();
        let chunk = match parse_chunk_data(event_name, data) {
            Ok(chunk) => chunk,
            Err(fault) => {
                faults.push(fault);
                return faults;
            }
        };
        let key = SnapshotKey {
            source_instance_id: source_instance_id.to_owned(),
            snapshot_id: chunk.snapshot_id.clone(),
        };
        if self.completed_snapshots.contains(&key) {
            faults.push(TrackerFault::new(
                "DUPLICATE_COMPLETED_SNAPSHOT",
                "a completed snapshot id was reused",
                json!({
                    "source_instance_id": source_instance_id,
                    "snapshot_id": chunk.snapshot_id
                }),
            ));
            return faults;
        }

        if !self.open_snapshots.contains_key(&key) {
            if self.open_snapshots.len() >= MAX_OPEN_SNAPSHOTS
                || self.open_snapshots.len() + self.completed_snapshots.len()
                    >= MAX_TRACKED_SNAPSHOTS
            {
                faults.push(TrackerFault::new(
                    "SNAPSHOT_TRACKER_CAPACITY",
                    "snapshot tracker reached its bounded capacity",
                    json!({
                        "maximum_open_snapshots": MAX_OPEN_SNAPSHOTS,
                        "maximum_completed_snapshots": MAX_TRACKED_SNAPSHOTS
                    }),
                ));
                return faults;
            }
            if chunk.chunk_index != 0 {
                faults.push(TrackerFault::new(
                    "SNAPSHOT_FIRST_CHUNK_MISSING",
                    "the first observed snapshot chunk did not have chunk_index 0",
                    json!({
                        "snapshot_id": chunk.snapshot_id,
                        "actual_chunk_index": chunk.chunk_index
                    }),
                ));
            }
            self.open_snapshots.insert(
                key.clone(),
                ChunkStreamState {
                    event_name: event_name.to_owned(),
                    stream: chunk.stream.clone(),
                    reason: chunk.reason.clone(),
                    config_id: chunk.config_id.clone(),
                    chunk_count: chunk.chunk_count,
                    total_items: chunk.total_items,
                    next_chunk: chunk.chunk_index,
                    observed_items: 0,
                    host_ids: HashMap::new(),
                    stable_metadata: chunk.stable_metadata.clone(),
                },
            );
        }

        let mut stream_complete = false;
        if let Some(state) = self.open_snapshots.get_mut(&key) {
            faults.extend(state.observe_chunk(&key, event_name, &chunk));
            stream_complete =
                chunk.snapshot_complete || chunk.chunk_index.saturating_add(1) >= chunk.chunk_count;
        }

        if stream_complete {
            let state = self
                .open_snapshots
                .remove(&key)
                .expect("completed snapshot is open");
            faults.extend(state.finish(&key));
            self.completed_snapshots.insert(key.clone());
            match chunk.stream.as_str() {
                "mixer_bank_snapshot" | "direct_access_snapshot" => {
                    self.completed_snapshot_streams =
                        self.completed_snapshot_streams.saturating_add(1);
                }
                "mixer_bank_feedback" | "direct_access_feedback" => {
                    self.completed_feedback_streams =
                        self.completed_feedback_streams.saturating_add(1);
                }
                _ => {
                    faults.push(TrackerFault::new(
                        "CHUNK_STREAM_KIND_INVALID",
                        "completed chunk stream had an unsupported kind",
                        json!({"stream": &chunk.stream}),
                    ));
                }
            }
            if let Some(index) = self.expected_followups.iter().position(|followup| {
                followup.source_instance_id == source_instance_id
                    && followup_matches(
                        &followup.template,
                        event_name,
                        &chunk.stream,
                        &chunk.reason,
                        chunk.config_id.as_deref(),
                    )
            }) {
                if checkpoint_id == Some(self.expected_followups[index].checkpoint_id.as_str()) {
                    self.expected_followups.remove(index);
                } else {
                    faults.push(TrackerFault::new(
                        "FOLLOWUP_CHECKPOINT_MISMATCH",
                        "follow-up snapshot receive time is outside the request's checkpoint window",
                        json!({
                            "request_id": self.expected_followups[index].request_id,
                            "request_checkpoint_id": self.expected_followups[index].checkpoint_id,
                            "observed_checkpoint_id": checkpoint_id
                        }),
                    ));
                }
            }
        }
        faults
    }

    #[cfg(test)]
    fn observe_chunk_event(
        &mut self,
        source_instance_id: &str,
        event_name: &str,
        data: &Value,
    ) -> Vec<TrackerFault> {
        let checkpoint_id = self
            .active_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.clone());
        self.observe_chunk_event_at(
            source_instance_id,
            event_name,
            data,
            checkpoint_id.as_deref(),
        )
    }

    fn begin_checkpoint(
        &mut self,
        checkpoint_id: String,
        window: Duration,
        now: Instant,
    ) -> Result<Value, TrackerFault> {
        if let Some(active) = &self.active_checkpoint {
            return Err(TrackerFault::new(
                "CHECKPOINT_ALREADY_ACTIVE",
                "a checkpoint must end before another checkpoint begins",
                json!({"active_checkpoint_id": active.checkpoint_id}),
            ));
        }
        if self.completed_checkpoints.contains(&checkpoint_id) {
            return Err(TrackerFault::new(
                "DUPLICATE_CHECKPOINT_ID",
                "checkpoint ids must be unique within one collector run",
                json!({"checkpoint_id": checkpoint_id}),
            ));
        }
        if self.completed_checkpoints.len() >= MAX_TRACKED_CHECKPOINTS {
            return Err(TrackerFault::new(
                "CHECKPOINT_TRACKER_CAPACITY",
                "checkpoint tracker reached its bounded capacity",
                json!({"maximum_checkpoints": MAX_TRACKED_CHECKPOINTS}),
            ));
        }
        self.active_checkpoint = Some(ActiveCheckpoint {
            checkpoint_id: checkpoint_id.clone(),
            started_at: now,
            window,
            message_count: 0,
            last_message_received_at: None,
            action_marked: false,
        });
        Ok(json!({
            "record_type": "collector_checkpoint",
            "phase": "begin",
            "checkpoint_id": checkpoint_id,
            "window_ms": duration_ms(window)
        }))
    }

    fn mark_action(&mut self, checkpoint_id: &str) -> Result<Value, TrackerFault> {
        let Some(active) = self.active_checkpoint.as_ref() else {
            return Err(TrackerFault::new(
                "ACTION_CHECKPOINT_NOT_ACTIVE",
                "collector.action requires an active checkpoint",
                json!({"checkpoint_id": checkpoint_id}),
            ));
        };
        if active.checkpoint_id != checkpoint_id {
            return Err(TrackerFault::new(
                "ACTION_CHECKPOINT_MISMATCH",
                "collector.action checkpoint_id does not match the active checkpoint",
                json!({
                    "expected_checkpoint_id": active.checkpoint_id,
                    "actual_checkpoint_id": checkpoint_id
                }),
            ));
        }
        if !self.is_quiescent() {
            return Err(TrackerFault::new(
                "ACTION_PROTOCOL_NOT_QUIESCENT",
                "collector.action requires all requests, follow-ups, and snapshots to be complete",
                self.incomplete_details(),
            ));
        }
        if active.action_marked {
            return Err(TrackerFault::new(
                "ACTION_ALREADY_MARKED",
                "collector.action may occur only once in a checkpoint",
                json!({"checkpoint_id": checkpoint_id}),
            ));
        }
        self.active_checkpoint
            .as_mut()
            .expect("active checkpoint was validated")
            .action_marked = true;
        Ok(json!({
            "record_type": "collector_action",
            "phase": "marked",
            "checkpoint_id": checkpoint_id
        }))
    }

    fn mark_observation_cut_action(
        &mut self,
        checkpoint_id: &str,
        request_id: &str,
        observation_epoch: u64,
    ) -> Result<Value, TrackerFault> {
        let mut marker = self.mark_action(checkpoint_id)?;
        let object = marker
            .as_object_mut()
            .expect("collector action marker is an object");
        object.insert(
            "boundary_source".into(),
            Value::String("probe.observation.cut_response".into()),
        );
        object.insert("request_id".into(), Value::String(request_id.to_owned()));
        object.insert("observation_epoch".into(), json!(observation_epoch));
        Ok(marker)
    }

    fn end_checkpoint(
        &mut self,
        checkpoint_id: &str,
        now: Instant,
    ) -> Result<(Value, Option<TrackerFault>), TrackerFault> {
        if !self.is_quiescent() {
            return Err(TrackerFault::new(
                "CHECKPOINT_PROTOCOL_NOT_QUIESCENT",
                "checkpoint cannot end while a request, follow-up, or snapshot is incomplete",
                self.incomplete_details(),
            ));
        }
        let Some(active) = self.active_checkpoint.take() else {
            return Err(TrackerFault::new(
                "CHECKPOINT_NOT_ACTIVE",
                "checkpoint end marker has no active begin marker",
                json!({"checkpoint_id": checkpoint_id}),
            ));
        };
        if active.checkpoint_id != checkpoint_id {
            self.active_checkpoint = Some(active);
            return Err(TrackerFault::new(
                "CHECKPOINT_ID_MISMATCH",
                "checkpoint end id does not match the active checkpoint",
                json!({
                    "expected_checkpoint_id": self
                        .active_checkpoint
                        .as_ref()
                        .map(|checkpoint| &checkpoint.checkpoint_id),
                    "actual_checkpoint_id": checkpoint_id
                }),
            ));
        }
        let elapsed = now.saturating_duration_since(active.started_at);
        let window_satisfied = elapsed >= active.window;
        let quiet_period_observed = active
            .last_message_received_at
            .map_or(elapsed, |last| now.saturating_duration_since(last));
        let quiet_period_satisfied = quiet_period_observed >= CHECKPOINT_QUIET_PERIOD;
        self.completed_checkpoints.insert(checkpoint_id.to_owned());
        self.checkpoint_history.push(CompletedCheckpoint {
            checkpoint_id: checkpoint_id.to_owned(),
            started_at: active.started_at,
            ended_at: now,
            window: active.window,
            message_count: active.message_count,
            messages_processed_after_end: 0,
        });
        let marker = json!({
            "record_type": "collector_checkpoint",
            "phase": "end",
            "checkpoint_id": checkpoint_id,
            "window_ms": duration_ms(active.window),
            "observed_duration_ms": duration_ms(elapsed),
            "window_satisfied": window_satisfied,
            "quiet_period_required_ms": duration_ms(CHECKPOINT_QUIET_PERIOD),
            "quiet_period_observed_ms": duration_ms(quiet_period_observed),
            "quiet_period_satisfied": quiet_period_satisfied,
            "messages_processed_before_end_marker": active.message_count,
            "late_received_frames_may_be_classified_by_receive_timestamp": true
        });
        let fault = if !window_satisfied {
            Some(TrackerFault::new(
                "CHECKPOINT_WINDOW_TOO_SHORT",
                "checkpoint ended before its declared observation window elapsed",
                json!({
                    "checkpoint_id": checkpoint_id,
                    "required_window_ms": duration_ms(active.window),
                    "observed_duration_ms": duration_ms(elapsed)
                }),
            ))
        } else if !quiet_period_satisfied {
            Some(TrackerFault::new(
                "CHECKPOINT_QUIET_PERIOD_NOT_SATISFIED",
                "the checkpoint ended without the required callback-free quiet period",
                json!({
                    "checkpoint_id": checkpoint_id,
                    "required_quiet_period_ms": duration_ms(CHECKPOINT_QUIET_PERIOD),
                    "observed_quiet_period_ms": duration_ms(quiet_period_observed)
                }),
            ))
        } else {
            None
        };
        Ok((marker, fault))
    }

    fn abort_checkpoint_at_eof(&mut self, now: Instant) -> Option<(Value, TrackerFault)> {
        let active = self.active_checkpoint.take()?;
        let elapsed = now.saturating_duration_since(active.started_at);
        self.completed_checkpoints
            .insert(active.checkpoint_id.clone());
        self.checkpoint_history.push(CompletedCheckpoint {
            checkpoint_id: active.checkpoint_id.clone(),
            started_at: active.started_at,
            ended_at: now,
            window: active.window,
            message_count: active.message_count,
            messages_processed_after_end: 0,
        });
        Some((
            json!({
                "record_type": "collector_checkpoint",
                "phase": "aborted_eof",
                "checkpoint_id": active.checkpoint_id.clone(),
                "window_ms": duration_ms(active.window),
                "observed_duration_ms": duration_ms(elapsed),
                "window_satisfied": elapsed >= active.window,
                "quiet_period_required_ms": duration_ms(CHECKPOINT_QUIET_PERIOD),
                "quiet_period_observed_ms": duration_ms(active
                    .last_message_received_at
                    .map_or(elapsed, |last| now.saturating_duration_since(last))),
                "messages_processed_before_abort_marker": active.message_count,
                "late_received_frames_may_be_classified_by_receive_timestamp": true
            }),
            TrackerFault::new(
                "CHECKPOINT_NOT_ENDED",
                "stdin reached EOF before the active checkpoint ended",
                json!({"checkpoint_id": active.checkpoint_id}),
            ),
        ))
    }

    fn checkpoint_context(&mut self, now: Instant) -> CheckpointContext {
        if let Some(active) = &mut self.active_checkpoint
            && now >= active.started_at
        {
            active.message_count = active.message_count.saturating_add(1);
            if active
                .last_message_received_at
                .is_none_or(|last| now > last)
            {
                active.last_message_received_at = Some(now);
            }
            let elapsed = now.saturating_duration_since(active.started_at);
            return CheckpointContext {
                checkpoint_id: Some(active.checkpoint_id.clone()),
                orphan: false,
                checkpoint_elapsed_ms: Some(duration_ms(elapsed)),
                checkpoint_window_ms: Some(duration_ms(active.window)),
                checkpoint_window_expired: Some(elapsed >= active.window),
                processed_after_checkpoint_end: false,
                checkpoint_quiet_period_violated: false,
            };
        }
        if let Some(checkpoint) = self
            .checkpoint_history
            .iter_mut()
            .rev()
            .find(|checkpoint| now >= checkpoint.started_at && now <= checkpoint.ended_at)
        {
            checkpoint.message_count = checkpoint.message_count.saturating_add(1);
            checkpoint.messages_processed_after_end =
                checkpoint.messages_processed_after_end.saturating_add(1);
            let elapsed = now.saturating_duration_since(checkpoint.started_at);
            let quiet_period_start = checkpoint
                .ended_at
                .checked_sub(CHECKPOINT_QUIET_PERIOD)
                .unwrap_or(checkpoint.started_at)
                .max(checkpoint.started_at);
            return CheckpointContext {
                checkpoint_id: Some(checkpoint.checkpoint_id.clone()),
                orphan: false,
                checkpoint_elapsed_ms: Some(duration_ms(elapsed)),
                checkpoint_window_ms: Some(duration_ms(checkpoint.window)),
                checkpoint_window_expired: Some(elapsed >= checkpoint.window),
                processed_after_checkpoint_end: true,
                checkpoint_quiet_period_violated: now > quiet_period_start,
            };
        }
        self.orphan_messages = self.orphan_messages.saturating_add(1);
        CheckpointContext {
            checkpoint_id: None,
            orphan: true,
            checkpoint_elapsed_ms: None,
            checkpoint_window_ms: None,
            checkpoint_window_expired: None,
            processed_after_checkpoint_end: false,
            checkpoint_quiet_period_violated: false,
        }
    }

    fn is_quiescent(&self) -> bool {
        self.pending_requests.is_empty()
            && self.expected_followups.is_empty()
            && self.open_snapshots.is_empty()
    }

    fn summary(&self) -> Value {
        let mut active_source_instance_ids: Vec<_> = self
            .source_lifecycle
            .iter()
            .filter_map(|(source, lifecycle)| {
                (*lifecycle == SourceLifecycle::Ready).then_some(source)
            })
            .collect();
        active_source_instance_ids.sort();
        json!({
            "completed_requests": self.completed_request_ids.len(),
            "completed_chunk_streams": self.completed_snapshots.len(),
            "completed_snapshot_streams": self.completed_snapshot_streams,
            "completed_feedback_streams": self.completed_feedback_streams,
            "completed_checkpoints": self.completed_checkpoints.len(),
            "checkpoint_messages": self
                .checkpoint_history
                .iter()
                .map(|checkpoint| checkpoint.message_count)
                .fold(0_u64, u64::saturating_add),
            "checkpoint_messages_processed_after_end": self
                .checkpoint_history
                .iter()
                .map(|checkpoint| checkpoint.messages_processed_after_end)
                .fold(0_u64, u64::saturating_add),
            "orphan_messages": self.orphan_messages,
            "pending_requests": self.pending_requests.len(),
            "expected_followups": self.expected_followups.len(),
            "open_snapshots": self.open_snapshots.len(),
            "selected_source_instance_id": self.selected_source_instance_id,
            "active_source_instance_ids": active_source_instance_ids
        })
    }

    fn incomplete_details(&self) -> Value {
        let mut pending: Vec<_> = self
            .pending_requests
            .iter()
            .map(|(request_id, request)| {
                json!({
                    "request_id": request_id,
                    "method": request.method,
                    "checkpoint_id": request.checkpoint_id,
                    "mode": match &request.mode {
                        PendingMode::Targeted { .. } => "targeted",
                        PendingMode::Discovery { .. } => "discovery"
                    }
                })
            })
            .collect();
        pending.sort_by_key(|value| {
            value
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
        let followups: Vec<_> = self
            .expected_followups
            .iter()
            .map(|followup| {
                json!({
                    "request_id": followup.request_id,
                    "source_instance_id": followup.source_instance_id,
                    "checkpoint_id": followup.checkpoint_id,
                    "kind": match &followup.template {
                        FollowupTemplate::Bank { .. } => "bank",
                        FollowupTemplate::DirectAccess => "direct_access"
                    }
                })
            })
            .collect();
        let snapshots: Vec<_> = self
            .open_snapshots
            .iter()
            .map(|(key, snapshot)| {
                json!({
                    "source_instance_id": key.source_instance_id,
                    "snapshot_id": key.snapshot_id,
                    "next_chunk_index": snapshot.next_chunk,
                    "chunk_count": snapshot.chunk_count,
                    "observed_items": snapshot.observed_items,
                    "total_items": snapshot.total_items
                })
            })
            .collect();
        json!({
            "pending_requests": pending,
            "expected_followups": followups,
            "open_snapshots": snapshots
        })
    }
}

fn validate_first_loaded_message(
    source_instance_id: &str,
    message: &Value,
) -> Result<(), TrackerFault> {
    let data = message
        .as_object()
        .filter(|message| message.get("type").and_then(Value::as_str) == Some("event"))
        .filter(|message| message.get("event").and_then(Value::as_str) == Some("probe.loaded"))
        .and_then(|message| message.get("data"))
        .and_then(Value::as_object);
    let valid = data.is_some_and(|data| {
        data.get("probe_session_id").and_then(Value::as_str) == Some(source_instance_id)
            && data.get("read_only").and_then(Value::as_bool) == Some(true)
            && data.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(PROBE_TRANSPORT_VERSION))
    });
    if valid {
        Ok(())
    } else {
        Err(TrackerFault::new(
            "SOURCE_FIRST_MESSAGE_INVALID",
            "the first source message must be source_seq 1 probe.loaded with matching session and read-only protocol v1 metadata",
            json!({"source_instance_id": source_instance_id}),
        ))
    }
}

fn validate_mapping_active_message(
    source_instance_id: &str,
    message: &Value,
) -> Result<(), TrackerFault> {
    let data = message.get("data").and_then(Value::as_object);
    let valid = data.is_some_and(|data| {
        data.get("probe_session_id").and_then(Value::as_str) == Some(source_instance_id)
            && data.get("mapping_active").and_then(Value::as_bool) == Some(true)
            && data.get("read_only").and_then(Value::as_bool) == Some(true)
            && data.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(PROBE_TRANSPORT_VERSION))
    });
    if valid {
        Ok(())
    } else {
        Err(TrackerFault::new(
            "PROBE_MAPPING_ACTIVE_INVALID",
            "probe.mapping_active must match the source and declare active read-only protocol v1 metadata",
            json!({"source_instance_id": source_instance_id}),
        ))
    }
}

fn validate_ready_message(
    source_instance_id: &str,
    message: &Value,
    expected_ready: bool,
) -> Result<(), TrackerFault> {
    let data = message.get("data").and_then(Value::as_object);
    let valid = data.is_some_and(|data| {
        data.get("ready").and_then(Value::as_bool) == Some(expected_ready)
            && data.get("probe_session_id").and_then(Value::as_str) == Some(source_instance_id)
            && data.get("read_only").and_then(Value::as_bool) == Some(true)
            && data.get("protocol_version").and_then(Value::as_u64)
                == Some(u64::from(PROBE_TRANSPORT_VERSION))
            && (!expected_ready
                || data
                    .get("initial_snapshots_complete")
                    .and_then(Value::as_bool)
                    == Some(true))
    });
    if valid {
        Ok(())
    } else {
        Err(TrackerFault::new(
            "PROBE_READY_METADATA_INVALID",
            "probe.ready metadata must match the source and readiness lifecycle contract",
            json!({
                "source_instance_id": source_instance_id,
                "expected_ready": expected_ready
            }),
        ))
    }
}

impl ChunkStreamState {
    fn observe_chunk(
        &mut self,
        key: &SnapshotKey,
        event_name: &str,
        chunk: &ChunkData,
    ) -> Vec<TrackerFault> {
        let mut faults = Vec::new();
        if self.event_name != event_name
            || self.stream != chunk.stream
            || self.reason != chunk.reason
            || self.config_id != chunk.config_id
            || self.chunk_count != chunk.chunk_count
            || self.total_items != chunk.total_items
            || self.stable_metadata != chunk.stable_metadata
        {
            faults.push(snapshot_fault(
                "SNAPSHOT_METADATA_CHANGED",
                "snapshot metadata changed between chunks",
                key,
                json!({"chunk_index": chunk.chunk_index}),
            ));
        }
        if chunk.chunk_index != self.next_chunk {
            faults.push(snapshot_fault(
                "SNAPSHOT_CHUNK_DUPLICATE_OR_REORDER",
                "snapshot chunk index was duplicated, skipped, or reordered",
                key,
                json!({
                    "expected_chunk_index": self.next_chunk,
                    "actual_chunk_index": chunk.chunk_index
                }),
            ));
        }
        self.next_chunk = chunk.chunk_index.saturating_add(1);
        self.observed_items = self.observed_items.saturating_add(chunk.items.len());
        if chunk.truncated {
            faults.push(snapshot_fault(
                "SOURCE_SNAPSHOT_TRUNCATED",
                "probe marked a snapshot as truncated",
                key,
                json!({"chunk_index": chunk.chunk_index}),
            ));
        }
        if !chunk.overflow_safe {
            faults.push(snapshot_fault(
                "SOURCE_SNAPSHOT_NOT_OVERFLOW_SAFE",
                "probe did not mark the snapshot chunk overflow-safe",
                key,
                json!({"chunk_index": chunk.chunk_index}),
            ));
        }
        let expected_complete = chunk.chunk_index + 1 == chunk.chunk_count;
        if chunk.snapshot_complete != expected_complete {
            faults.push(snapshot_fault(
                "SNAPSHOT_COMPLETE_FLAG_MISMATCH",
                "snapshot_complete did not match the final chunk index",
                key,
                json!({
                    "chunk_index": chunk.chunk_index,
                    "chunk_count": chunk.chunk_count,
                    "snapshot_complete": chunk.snapshot_complete
                }),
            ));
        }

        for item in &chunk.items {
            if self.event_name == "probe.bank.chunk"
                && self.stream == "mixer_bank_snapshot"
                && item.get("record_kind").and_then(Value::as_str) != Some("host_id_fragment")
                && item.get("config_id").and_then(Value::as_str) != self.config_id.as_deref()
            {
                faults.push(snapshot_fault(
                    "BANK_ITEM_CONFIG_ID_MISMATCH",
                    "bank snapshot item config_id does not match the chunk config_id",
                    key,
                    json!({"chunk_index": chunk.chunk_index}),
                ));
            }
            faults.extend(self.observe_host_id_item(key, item));
        }
        faults
    }

    fn observe_host_id_item(&mut self, key: &SnapshotKey, item: &Value) -> Vec<TrackerFault> {
        let Some(object) = item.as_object() else {
            return vec![snapshot_fault(
                "SNAPSHOT_ITEM_INVALID",
                "snapshot item must be an object",
                key,
                json!({}),
            )];
        };
        if object.get("record_kind").and_then(Value::as_str) == Some("host_id_fragment") {
            return self.observe_host_id_fragment(key, object);
        }
        if !object.contains_key("host_id_raw") {
            return Vec::new();
        }

        let raw = object.get("host_id_raw").expect("field exists");
        let reference = object.get("host_id_ref");
        let fragment_count = object
            .get("host_id_fragment_count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let byte_length = object
            .get("host_id_byte_length")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        if let Some(raw) = raw.as_str() {
            if byte_length != Some(raw.len())
                || raw.len() > MAX_INLINE_HOST_ID_BYTES
                || fragment_count != Some(0)
                || !reference.is_none_or(Value::is_null)
            {
                return vec![snapshot_fault(
                    "INLINE_HOST_ID_METADATA_INVALID",
                    "inline host id metadata is inconsistent",
                    key,
                    json!({}),
                )];
            }
            return Vec::new();
        }
        if !raw.is_null() {
            return vec![snapshot_fault(
                "HOST_ID_TYPE_INVALID",
                "host_id_raw must be a string or null",
                key,
                json!({}),
            )];
        }

        match reference.and_then(Value::as_str) {
            Some(reference) if !reference.is_empty() && reference.len() <= MAX_REQUEST_ID_BYTES => {
                let Some(expected_fragments) = fragment_count.filter(|count| *count > 0) else {
                    return vec![snapshot_fault(
                        "HOST_ID_FRAGMENT_COUNT_INVALID",
                        "fragmented host id must declare a positive fragment count",
                        key,
                        json!({"host_id_ref": reference}),
                    )];
                };
                let Some(expected_bytes) = byte_length else {
                    return vec![snapshot_fault(
                        "HOST_ID_BYTE_LENGTH_INVALID",
                        "fragmented host id must declare its UTF-8 byte length",
                        key,
                        json!({"host_id_ref": reference}),
                    )];
                };
                if expected_fragments > MAX_HOST_ID_FRAGMENTS || expected_bytes > MAX_HOST_ID_BYTES
                {
                    return vec![snapshot_fault(
                        "HOST_ID_FRAGMENT_BOUNDS_EXCEEDED",
                        "fragmented host id exceeds the probe's declared bounds",
                        key,
                        json!({
                            "host_id_ref": reference,
                            "fragment_count": expected_fragments,
                            "byte_length": expected_bytes
                        }),
                    )];
                }
                if self.host_ids.contains_key(reference) {
                    return vec![snapshot_fault(
                        "DUPLICATE_HOST_ID_REF",
                        "host_id_ref was declared more than once in one snapshot",
                        key,
                        json!({"host_id_ref": reference}),
                    )];
                }
                self.host_ids.insert(
                    reference.to_owned(),
                    HostIdAssembly {
                        expected_bytes,
                        expected_fragments,
                        next_fragment: 0,
                        observed_bytes: 0,
                    },
                );
                Vec::new()
            }
            None => {
                if fragment_count != Some(0) || byte_length.is_some() {
                    vec![snapshot_fault(
                        "NULL_HOST_ID_METADATA_INVALID",
                        "unavailable host id metadata is inconsistent",
                        key,
                        json!({}),
                    )]
                } else {
                    Vec::new()
                }
            }
            Some(_) => vec![snapshot_fault(
                "HOST_ID_REF_INVALID",
                "host_id_ref must be null or a bounded non-empty string",
                key,
                json!({}),
            )],
        }
    }

    fn observe_host_id_fragment(
        &mut self,
        key: &SnapshotKey,
        object: &Map<String, Value>,
    ) -> Vec<TrackerFault> {
        let Some(reference) = object
            .get("host_id_ref")
            .and_then(Value::as_str)
            .filter(|reference| !reference.is_empty() && reference.len() <= MAX_REQUEST_ID_BYTES)
        else {
            return vec![snapshot_fault(
                "HOST_ID_FRAGMENT_REF_INVALID",
                "host id fragment is missing host_id_ref",
                key,
                json!({}),
            )];
        };
        let Some(assembly) = self.host_ids.get_mut(reference) else {
            return vec![snapshot_fault(
                "ORPHAN_HOST_ID_FRAGMENT",
                "host id fragment appeared before its observation record",
                key,
                json!({"host_id_ref": reference}),
            )];
        };
        let fragment_index = object
            .get("fragment_index")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let fragment_count = object
            .get("fragment_count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let byte_length = object
            .get("host_id_byte_length")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok());
        let fragment = object.get("fragment").and_then(Value::as_str);
        if fragment_index != Some(assembly.next_fragment)
            || fragment_count != Some(assembly.expected_fragments)
            || byte_length != Some(assembly.expected_bytes)
            || fragment.is_none()
            || fragment.is_some_and(|value| value.len() > MAX_HOST_ID_FRAGMENT_BYTES)
        {
            return vec![snapshot_fault(
                "HOST_ID_FRAGMENT_DUPLICATE_OR_REORDER",
                "host id fragment metadata was missing, duplicated, or reordered",
                key,
                json!({
                    "host_id_ref": reference,
                    "expected_fragment_index": assembly.next_fragment,
                    "actual_fragment_index": fragment_index
                }),
            )];
        }
        assembly.next_fragment += 1;
        assembly.observed_bytes = assembly
            .observed_bytes
            .saturating_add(fragment.expect("checked fragment string").len());
        Vec::new()
    }

    fn finish(self, key: &SnapshotKey) -> Vec<TrackerFault> {
        let mut faults = Vec::new();
        if self.next_chunk != self.chunk_count || self.observed_items != self.total_items {
            faults.push(snapshot_fault(
                "SNAPSHOT_INCOMPLETE",
                "snapshot ended with missing chunks or items",
                key,
                json!({
                    "observed_chunks": self.next_chunk,
                    "chunk_count": self.chunk_count,
                    "observed_items": self.observed_items,
                    "total_items": self.total_items
                }),
            ));
        }
        for (reference, assembly) in self.host_ids {
            if assembly.next_fragment != assembly.expected_fragments
                || assembly.observed_bytes != assembly.expected_bytes
            {
                faults.push(snapshot_fault(
                    "HOST_ID_FRAGMENT_STREAM_INCOMPLETE",
                    "host id fragment stream ended incomplete",
                    key,
                    json!({
                        "host_id_ref": reference,
                        "observed_fragments": assembly.next_fragment,
                        "expected_fragments": assembly.expected_fragments,
                        "observed_bytes": assembly.observed_bytes,
                        "expected_bytes": assembly.expected_bytes
                    }),
                ));
            }
        }
        faults
    }
}

fn parse_chunk_data(event_name: &str, data: &Value) -> Result<ChunkData, TrackerFault> {
    let object = data.as_object().ok_or_else(|| {
        TrackerFault::new(
            "CHUNK_DATA_INVALID",
            "chunk event data must be an object",
            json!({"event": event_name}),
        )
    })?;
    let string = |field: &str| {
        object
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= MAX_REQUEST_ID_BYTES)
            .map(str::to_owned)
            .ok_or_else(|| {
                TrackerFault::new(
                    "CHUNK_DATA_INVALID",
                    format!("chunk field '{field}' must be a bounded non-empty string"),
                    json!({"event": event_name}),
                )
            })
    };
    let integer = |field: &str, allow_zero: bool, maximum: usize| {
        let value = object
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (allow_zero || *value > 0) && *value <= maximum);
        value.ok_or_else(|| {
            TrackerFault::new(
                "CHUNK_DATA_INVALID",
                format!("chunk field '{field}' is outside the bounded integer range"),
                json!({"event": event_name}),
            )
        })
    };
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            TrackerFault::new(
                "CHUNK_DATA_INVALID",
                "chunk items must be an array",
                json!({"event": event_name}),
            )
        })?;
    if items.len() > MAX_ITEMS_PER_CHUNK {
        return Err(TrackerFault::new(
            "CHUNK_DATA_INVALID",
            "chunk items exceed the probe's bounded per-chunk limit",
            json!({
                "event": event_name,
                "items": items.len(),
                "maximum_items": MAX_ITEMS_PER_CHUNK
            }),
        ));
    }
    let boolean = |field: &str| {
        object.get(field).and_then(Value::as_bool).ok_or_else(|| {
            TrackerFault::new(
                "CHUNK_DATA_INVALID",
                format!("chunk field '{field}' must be a boolean"),
                json!({"event": event_name}),
            )
        })
    };
    let chunk_index = integer("chunk_index", true, MAX_CHUNKS_PER_SNAPSHOT)?;
    let chunk_count = integer("chunk_count", false, MAX_CHUNKS_PER_SNAPSHOT)?;
    if chunk_index >= chunk_count {
        return Err(TrackerFault::new(
            "CHUNK_DATA_INVALID",
            "chunk_index must be less than chunk_count",
            json!({
                "event": event_name,
                "chunk_index": chunk_index,
                "chunk_count": chunk_count
            }),
        ));
    }
    let total_items = integer("total_items", true, MAX_ITEMS_PER_SNAPSHOT)?;
    let minimum_chunks = total_items.div_ceil(MAX_ITEMS_PER_CHUNK).max(1);
    let maximum_chunks = total_items.max(1);
    if chunk_count < minimum_chunks || chunk_count > maximum_chunks {
        return Err(TrackerFault::new(
            "CHUNK_DATA_INVALID",
            "chunk_count is inconsistent with total_items and the per-chunk item bound",
            json!({
                "event": event_name,
                "chunk_count": chunk_count,
                "total_items": total_items,
                "minimum_chunks": minimum_chunks,
                "maximum_chunks": maximum_chunks
            }),
        ));
    }
    if (total_items == 0 && (chunk_count != 1 || !items.is_empty()))
        || (total_items > 0 && items.is_empty())
    {
        return Err(TrackerFault::new(
            "CHUNK_DATA_INVALID",
            "empty chunk layout is inconsistent with total_items",
            json!({
                "event": event_name,
                "chunk_index": chunk_index,
                "chunk_count": chunk_count,
                "total_items": total_items,
                "chunk_items": items.len()
            }),
        ));
    }
    let stream = string("stream")?;
    let config_id = match object.get("config_id") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= MAX_REQUEST_ID_BYTES)
                .ok_or_else(|| {
                    TrackerFault::new(
                        "CHUNK_DATA_INVALID",
                        "config_id must be a bounded non-empty string when present",
                        json!({"event": event_name}),
                    )
                })?
                .to_owned(),
        ),
    };
    let stream_valid = match event_name {
        "probe.bank.chunk" => match stream.as_str() {
            "mixer_bank_snapshot" => config_id.is_some(),
            "mixer_bank_feedback" => config_id.is_none(),
            _ => false,
        },
        "probe.direct_access.chunk" => {
            matches!(
                stream.as_str(),
                "direct_access_snapshot" | "direct_access_feedback"
            ) && config_id.is_none()
        }
        _ => false,
    };
    if !stream_valid {
        return Err(TrackerFault::new(
            "CHUNK_STREAM_INVALID",
            "chunk event, stream, and config_id do not match the probe schema",
            json!({
                "event": event_name,
                "stream": stream,
                "config_id": config_id
            }),
        ));
    }
    let mut stable_metadata = object.clone();
    stable_metadata.remove("items");
    stable_metadata.remove("chunk_index");
    stable_metadata.remove("snapshot_complete");
    Ok(ChunkData {
        snapshot_id: string("snapshot_id")?,
        stream,
        reason: string("reason")?,
        config_id,
        chunk_index,
        chunk_count,
        total_items,
        items,
        snapshot_complete: boolean("snapshot_complete")?,
        truncated: boolean("truncated")?,
        overflow_safe: boolean("overflow_safe")?,
        stable_metadata,
    })
}

fn followup_matches(
    template: &FollowupTemplate,
    event_name: &str,
    stream: &str,
    reason: &str,
    config_id: Option<&str>,
) -> bool {
    match template {
        FollowupTemplate::Bank {
            config_id: expected_config,
            reason: expected_reason,
        } => {
            event_name == "probe.bank.chunk"
                && stream == "mixer_bank_snapshot"
                && reason == expected_reason
                && config_id == Some(expected_config.as_str())
        }
        FollowupTemplate::DirectAccess => {
            event_name == "probe.direct_access.chunk"
                && stream == "direct_access_snapshot"
                && reason == "command_snapshot"
        }
    }
}

fn snapshot_fault(
    code: &'static str,
    message: impl Into<String>,
    key: &SnapshotKey,
    extra: Value,
) -> TrackerFault {
    TrackerFault::new(
        code,
        message,
        json!({
            "source_instance_id": key.source_instance_id,
            "snapshot_id": key.snapshot_id,
            "extra": extra
        }),
    )
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn observation_cut_response_epoch(message: &Value) -> Option<u64> {
    let result = message.get("result")?.as_object()?;
    if result.len() != 1 {
        return None;
    }
    result
        .get("observation_epoch")?
        .as_u64()
        .filter(|epoch| (1..=MAX_OBSERVATION_EPOCH).contains(epoch))
}

#[derive(Default)]
struct DrainReport {
    completed: bool,
    timed_out: bool,
    duration_ms: u64,
}

fn graceful_drain(
    runtime: &RuntimeTracker,
    timeout: Duration,
    sink: &JsonlSink,
    integrity_failed: &AtomicBool,
) -> DrainReport {
    let started_at = Instant::now();
    let deadline = started_at + timeout;
    let _ = sink.emit(&json!({
        "record_type": "collector_drain_started",
        "timeout_ms": duration_ms(timeout),
        "deadline_monotonic_timestamp_ms": sink.monotonic_timestamp_at(deadline)
    }));

    let mut state = runtime.state.lock().unwrap_or_else(|error| {
        integrity_failed.store(true, Ordering::Release);
        error.into_inner()
    });
    if let Some((marker, fault)) = state.abort_checkpoint_at_eof(Instant::now()) {
        let _ = sink.emit(&marker);
        emit_tracker_faults(vec![fault], sink, integrity_failed, "fatal");
    }

    let mut timed_out = false;
    loop {
        let now = Instant::now();
        let expiry = state.expire_discoveries(now);
        for record in &expiry.records {
            let _ = sink.emit(record);
        }
        if !expiry.faults.is_empty() {
            emit_tracker_faults(expiry.faults, sink, integrity_failed, "fatal");
        }
        if expiry.expired > 0 {
            runtime.notify();
        }
        if now >= deadline {
            if !state.is_quiescent() {
                timed_out = true;
                let details = state.incomplete_details();
                emit_tracker_faults(
                    vec![TrackerFault::new(
                        "GRACEFUL_DRAIN_TIMEOUT",
                        "stdin EOF graceful drain expired with unresolved requests or snapshot streams",
                        json!({
                            "timeout_ms": duration_ms(timeout),
                            "incomplete": details
                        }),
                    )],
                    sink,
                    integrity_failed,
                    "fatal",
                );
            }
            break;
        }
        let wait = deadline.saturating_duration_since(now);
        let (next_state, _) = runtime
            .changed
            .wait_timeout(state, wait)
            .unwrap_or_else(|error| {
                integrity_failed.store(true, Ordering::Release);
                error.into_inner()
            });
        state = next_state;
    }
    drop(state);

    let duration = started_at.elapsed();
    let report = DrainReport {
        completed: !timed_out,
        timed_out,
        duration_ms: duration_ms(duration),
    };
    let _ = sink.emit(&json!({
        "record_type": "collector_drain_completed",
        "completed": report.completed,
        "timed_out": report.timed_out,
        "duration_ms": report.duration_ms
    }));
    report
}

#[derive(Default)]
struct CollectorReport {
    frames: u64,
    messages: u64,
    events: u64,
    responses: u64,
    errors: u64,
    diagnostics: u64,
    parse_errors: u64,
    oversize_frames: u64,
    source_overflows: u64,
    queue_drops: u64,
    sequence_gaps: u64,
    sequence_duplicates: u64,
    sources: Vec<Value>,
}

fn collect_incoming(
    receiver: Receiver<Ingress>,
    dropped_items: Arc<AtomicU64>,
    integrity_failed: Arc<AtomicBool>,
    sink: Arc<JsonlSink>,
    runtime: Arc<RuntimeTracker>,
    ingress_progress: Arc<IngressProgress>,
) -> CollectorReport {
    let mut report = CollectorReport::default();
    let mut sequences = SequenceTracker::default();

    loop {
        record_queue_drops(&dropped_items, &integrity_failed, &sink, &mut report);
        match receiver.recv_timeout(COLLECTOR_POLL_INTERVAL) {
            Ok(Ingress::Frame {
                received_at_unix_ms,
                received_at_monotonic,
                midi_timestamp,
                bytes,
            }) => {
                expire_runtime_discoveries(
                    &runtime,
                    received_at_monotonic,
                    &integrity_failed,
                    &sink,
                    &mut report,
                );
                report.frames += 1;
                match decode_incoming(&bytes) {
                    Ok((envelope, kind)) => {
                        let first_seen = !sequences.contains_source(&envelope.source_instance_id);
                        let observation =
                            sequences.observe(&envelope.source_instance_id, envelope.source_seq);
                        match observation {
                            SequenceObservation::Contiguous => {}
                            SequenceObservation::Gap { expected, actual } => {
                                integrity_failed.store(true, Ordering::Release);
                                report.diagnostics += 1;
                                report.sequence_gaps += 1;
                                let _ = emit_diagnostic(
                                    &sink,
                                    "SOURCE_SEQUENCE_GAP",
                                    "fatal",
                                    "one or more probe messages were not observed",
                                    json!({
                                        "source_instance_id": envelope.source_instance_id,
                                        "expected_source_seq": expected,
                                        "actual_source_seq": actual
                                    }),
                                );
                            }
                            SequenceObservation::DuplicateOrReorder { expected, actual } => {
                                integrity_failed.store(true, Ordering::Release);
                                report.diagnostics += 1;
                                report.sequence_duplicates += 1;
                                let _ = emit_diagnostic(
                                    &sink,
                                    "SOURCE_SEQUENCE_DUPLICATE_OR_REORDER",
                                    "fatal",
                                    "a duplicate or out-of-order probe message was observed",
                                    json!({
                                        "source_instance_id": envelope.source_instance_id,
                                        "expected_source_seq": expected,
                                        "actual_source_seq": actual
                                    }),
                                );
                            }
                            SequenceObservation::TooManySources => {
                                integrity_failed.store(true, Ordering::Release);
                                report.diagnostics += 1;
                                let _ = emit_diagnostic(
                                    &sink,
                                    "TOO_MANY_SOURCE_INSTANCES",
                                    "fatal",
                                    "the number of probe source instances exceeds the bounded limit",
                                    json!({"maximum_sources": MAX_SOURCE_INSTANCES}),
                                );
                            }
                            SequenceObservation::Exhausted => {
                                integrity_failed.store(true, Ordering::Release);
                                report.diagnostics += 1;
                                let _ = emit_diagnostic(
                                    &sink,
                                    "SOURCE_SEQUENCE_EXHAUSTED",
                                    "fatal",
                                    "source_seq reached the maximum integer value",
                                    json!({"source_instance_id": envelope.source_instance_id}),
                                );
                            }
                        }

                        if let Some(overflow_data) = source_overflow_data(kind, &envelope.message) {
                            integrity_failed.store(true, Ordering::Release);
                            report.diagnostics += 1;
                            report.source_overflows += 1;
                            let _ = emit_diagnostic(
                                &sink,
                                "SOURCE_REPORTED_OVERFLOW",
                                "fatal",
                                "the probe reported a dropped or oversize source message",
                                json!({
                                    "source_instance_id": envelope.source_instance_id,
                                    "source_seq": envelope.source_seq,
                                    "source_overflow": overflow_data
                                }),
                            );
                        }

                        report.messages += 1;
                        match kind {
                            MessageKind::Event => report.events += 1,
                            MessageKind::Response => report.responses += 1,
                            MessageKind::Error => report.errors += 1,
                        }
                        let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
                            integrity_failed.store(true, Ordering::Release);
                            error.into_inner()
                        });
                        let mut faults = tracker.observe_source_message(
                            &envelope.source_instance_id,
                            &envelope.message,
                            first_seen,
                        );
                        let checkpoint = tracker.checkpoint_context(received_at_monotonic);
                        let mut automatic_action_marker = None;
                        match kind {
                            MessageKind::Response | MessageKind::Error => {
                                let request_id = envelope
                                    .message
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .expect("validated response or error has an id");
                                let is_observation_cut = kind == MessageKind::Response
                                    && tracker.request_method(request_id)
                                        == Some("probe.observation.cut");
                                let observation_epoch = is_observation_cut
                                    .then(|| observation_cut_response_epoch(&envelope.message))
                                    .flatten();
                                let reply_faults = tracker.observe_reply_message(
                                    &envelope.source_instance_id,
                                    request_id,
                                    kind,
                                    &envelope.message,
                                    checkpoint.checkpoint_id.as_deref(),
                                );
                                let can_mark_cut_action = is_observation_cut
                                    && observation_epoch.is_some()
                                    && faults.is_empty()
                                    && reply_faults.is_empty()
                                    && !checkpoint.orphan
                                    && !checkpoint.checkpoint_quiet_period_violated
                                    && !integrity_failed.load(Ordering::Acquire);
                                faults.extend(reply_faults);
                                if is_observation_cut && observation_epoch.is_none() {
                                    faults.push(TrackerFault::new(
                                        "OBSERVATION_CUT_RESPONSE_INVALID",
                                        "a successful observation cut response must contain only a bounded observation_epoch",
                                        json!({"request_id": request_id}),
                                    ));
                                } else if can_mark_cut_action {
                                    match tracker.mark_observation_cut_action(
                                        checkpoint
                                            .checkpoint_id
                                            .as_deref()
                                            .expect("non-orphan cut response has a checkpoint"),
                                        request_id,
                                        observation_epoch
                                            .expect("validated observation cut epoch exists"),
                                    ) {
                                        Ok(marker) => automatic_action_marker = Some(marker),
                                        Err(fault) => faults.push(fault),
                                    }
                                }
                            }
                            MessageKind::Event => {
                                let event_name = envelope
                                    .message
                                    .get("event")
                                    .and_then(Value::as_str)
                                    .expect("validated event has a name");
                                let event_faults = if matches!(
                                    event_name,
                                    "probe.bank.chunk" | "probe.direct_access.chunk"
                                ) {
                                    tracker.observe_chunk_event_at(
                                        &envelope.source_instance_id,
                                        event_name,
                                        envelope
                                            .message
                                            .get("data")
                                            .expect("validated event has data"),
                                        checkpoint.checkpoint_id.as_deref(),
                                    )
                                } else {
                                    Vec::new()
                                };
                                faults.extend(event_faults);
                            }
                        }
                        if checkpoint.orphan {
                            faults.push(TrackerFault::new(
                                "ORPHAN_PROBE_MESSAGE",
                                "a probe message was observed outside every checkpoint receive-time window",
                                json!({
                                    "source_instance_id": envelope.source_instance_id,
                                    "source_seq": envelope.source_seq,
                                    "message_type": envelope.message.get("type"),
                                    "event": envelope.message.get("event"),
                                    "request_id": envelope.message.get("id")
                                }),
                            ));
                        }
                        if checkpoint.checkpoint_quiet_period_violated {
                            faults.push(TrackerFault::new(
                                "CHECKPOINT_QUIET_PERIOD_VIOLATED",
                                "a callback received during the checkpoint's final quiet period was processed after its end marker",
                                json!({
                                    "checkpoint_id": checkpoint.checkpoint_id.as_deref(),
                                    "source_instance_id": envelope.source_instance_id,
                                    "source_seq": envelope.source_seq,
                                    "required_quiet_period_ms": duration_ms(CHECKPOINT_QUIET_PERIOD)
                                }),
                            ));
                        }
                        report.diagnostics = report.diagnostics.saturating_add(
                            emit_tracker_faults(faults, &sink, &integrity_failed, "fatal"),
                        );
                        let probe_record = json!({
                            "record_type": kind.record_type(),
                            "received_at_unix_ms": received_at_unix_ms,
                            "received_at_monotonic_timestamp_ms": sink
                                .monotonic_timestamp_at(received_at_monotonic),
                            "midi_timestamp": midi_timestamp,
                            "integrity_ok_at_emit": !integrity_failed.load(Ordering::Acquire),
                            "probe_transport_version": envelope.probe_transport_version,
                            "source_instance_id": envelope.source_instance_id,
                            "source_seq": envelope.source_seq,
                            "checkpoint_id": checkpoint.checkpoint_id,
                            "orphan": checkpoint.orphan,
                            "checkpoint_elapsed_ms": checkpoint.checkpoint_elapsed_ms,
                            "checkpoint_window_ms": checkpoint.checkpoint_window_ms,
                            "checkpoint_window_expired": checkpoint.checkpoint_window_expired,
                            "processed_after_checkpoint_end": checkpoint.processed_after_checkpoint_end,
                            "checkpoint_quiet_period_violated": checkpoint.checkpoint_quiet_period_violated,
                            "message": envelope.message
                        });
                        let evidence_result = if let Some(marker) = automatic_action_marker {
                            sink.emit_pair(&probe_record, &marker)
                        } else {
                            sink.emit(&probe_record)
                        };
                        if evidence_result.is_err() {
                            integrity_failed.store(true, Ordering::Release);
                        }
                        drop(tracker);
                        runtime.notify();
                    }
                    Err(error) => {
                        integrity_failed.store(true, Ordering::Release);
                        report.diagnostics += 1;
                        if error.oversize {
                            report.oversize_frames += 1;
                        } else {
                            report.parse_errors += 1;
                        }
                        let _ = emit_diagnostic(
                            &sink,
                            error.code,
                            "fatal",
                            &error.message,
                            json!({"received_at_unix_ms": received_at_unix_ms}),
                        );
                    }
                }
                // The command barrier may advance only after every tracker effect
                // and its evidence record for this received frame are complete.
                ingress_progress.mark_processed(&integrity_failed);
            }
            Ok(Ingress::FramingFault {
                received_at_unix_ms,
                received_at_monotonic,
                fault,
            }) => {
                expire_runtime_discoveries(
                    &runtime,
                    received_at_monotonic,
                    &integrity_failed,
                    &sink,
                    &mut report,
                );
                integrity_failed.store(true, Ordering::Release);
                report.diagnostics += 1;
                let (code, message, oversize) = match fault {
                    FramingFault::NestedStart => (
                        "NESTED_SYSEX_START",
                        "a new SysEx start byte interrupted an incomplete frame",
                        false,
                    ),
                    FramingFault::Oversize => (
                        "OVERSIZE_FRAME",
                        "an incoming SysEx frame exceeded the 64 KiB JSON limit",
                        true,
                    ),
                    FramingFault::TruncatedAtShutdown => (
                        "TRUNCATED_FRAME_AT_SHUTDOWN",
                        "stdin closed while an incoming SysEx frame was incomplete",
                        false,
                    ),
                };
                if oversize {
                    report.oversize_frames += 1;
                } else {
                    report.parse_errors += 1;
                }
                let _ = emit_diagnostic(
                    &sink,
                    code,
                    "fatal",
                    message,
                    json!({"received_at_unix_ms": received_at_unix_ms}),
                );
                ingress_progress.mark_processed(&integrity_failed);
            }
            Err(RecvTimeoutError::Timeout) => {
                expire_runtime_discoveries(
                    &runtime,
                    Instant::now(),
                    &integrity_failed,
                    &sink,
                    &mut report,
                );
            }
            Err(RecvTimeoutError::Disconnected) => {
                record_queue_drops(&dropped_items, &integrity_failed, &sink, &mut report);
                break;
            }
        }
    }

    report.sources = sequences.source_summaries();
    report
}

fn expire_runtime_discoveries(
    runtime: &RuntimeTracker,
    now: Instant,
    integrity_failed: &AtomicBool,
    sink: &JsonlSink,
    report: &mut CollectorReport,
) {
    let mut tracker = runtime.state.lock().unwrap_or_else(|error| {
        integrity_failed.store(true, Ordering::Release);
        error.into_inner()
    });
    let expiry = tracker.expire_discoveries(now);
    for record in &expiry.records {
        let _ = sink.emit(record);
    }
    report.diagnostics = report.diagnostics.saturating_add(emit_tracker_faults(
        expiry.faults,
        sink,
        integrity_failed,
        "fatal",
    ));
    drop(tracker);
    if expiry.expired > 0 {
        runtime.notify();
    }
}

fn record_queue_drops(
    dropped_items: &AtomicU64,
    integrity_failed: &AtomicBool,
    sink: &JsonlSink,
    report: &mut CollectorReport,
) {
    let dropped = dropped_items.swap(0, Ordering::AcqRel);
    if dropped == 0 {
        return;
    }
    integrity_failed.store(true, Ordering::Release);
    report.diagnostics += 1;
    report.queue_drops = report.queue_drops.saturating_add(dropped);
    let _ = emit_diagnostic(
        sink,
        "MIDI_QUEUE_OVERFLOW",
        "fatal",
        "the bounded MIDI receive queue overflowed; the observation is incomplete",
        json!({"dropped_items": dropped, "queue_capacity": MIDI_QUEUE_CAPACITY}),
    );
}

struct JsonlSink {
    writer: Mutex<Box<dyn Write + Send>>,
    failed: AtomicBool,
    run_id: String,
    started_at: Instant,
    #[cfg(test)]
    record_prepare_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl JsonlSink {
    fn stdout(run_id: String, started_at: Instant) -> Self {
        Self {
            writer: Mutex::new(Box::new(io::stdout())),
            failed: AtomicBool::new(false),
            run_id,
            started_at,
            #[cfg(test)]
            record_prepare_hook: None,
        }
    }

    fn emit(&self, value: &Value) -> io::Result<()> {
        let mut writer = self.lock_writer()?;
        let record = self.prepare_record(value)?;
        self.write_records_locked(&mut **writer, std::slice::from_ref(&record))
    }

    fn emit_pair(&self, first: &Value, second: &Value) -> io::Result<()> {
        let mut writer = self.lock_writer()?;
        let records = [self.prepare_record(first)?, self.prepare_record(second)?];
        self.write_records_locked(&mut **writer, &records)
    }

    fn prepare_record(&self, value: &Value) -> io::Result<Value> {
        #[cfg(test)]
        if let Some(hook) = &self.record_prepare_hook {
            hook();
        }
        let mut record = value.clone();
        let object = record
            .as_object_mut()
            .ok_or_else(|| io::Error::other("JSONL record must be an object"))?;
        object.insert("record_format_version".into(), json!(RECORD_FORMAT_VERSION));
        object.insert("run_id".into(), Value::String(self.run_id.clone()));
        object.insert("timestamp_unix_ms".into(), json!(unix_timestamp_ms()));
        object.insert(
            "monotonic_timestamp_ms".into(),
            json!(self.monotonic_timestamp_ms()),
        );
        Ok(record)
    }

    fn lock_writer(&self) -> io::Result<MutexGuard<'_, Box<dyn Write + Send>>> {
        self.writer.lock().map_err(|_| {
            self.failed.store(true, Ordering::Release);
            io::Error::other("JSONL output lock was poisoned")
        })
    }

    fn write_records_locked(
        &self,
        writer: &mut (dyn Write + Send),
        records: &[Value],
    ) -> io::Result<()> {
        let result = records.iter().try_for_each(|record| {
            serde_json::to_writer(&mut *writer, record)
                .map_err(|error| {
                    io::Error::other(format!("could not encode JSONL record: {error}"))
                })
                .and_then(|()| writer.write_all(b"\n"))
        });
        if let Err(error) = result.and_then(|()| writer.flush()) {
            self.failed.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    fn monotonic_timestamp_ms(&self) -> u64 {
        self.monotonic_timestamp_at(Instant::now())
    }

    fn monotonic_timestamp_at(&self, instant: Instant) -> u64 {
        instant
            .saturating_duration_since(self.started_at)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

fn emit_diagnostic(
    sink: &JsonlSink,
    code: &str,
    severity: &str,
    message: &str,
    details: Value,
) -> io::Result<()> {
    sink.emit(&json!({
        "record_type": "collector_diagnostic",
        "timestamp_unix_ms": unix_timestamp_ms(),
        "severity": severity,
        "code": code,
        "message": message,
        "details": details
    }))
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn new_session_id() -> String {
    format!("probe-{}-{}", std::process::id(), unix_timestamp_ms())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Barrier, TryLockError, Weak};

    use super::*;

    #[derive(Clone)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("deterministic read failure"))
        }
    }

    fn incoming(source: &str, sequence: u64, message: Value) -> Value {
        json!({
            "probe_transport_version": 1,
            "source_instance_id": source,
            "source_seq": sequence,
            "message": message
        })
    }

    fn enqueue_test_incoming(
        sender: &SyncSender<Ingress>,
        dropped: &AtomicU64,
        failed: &AtomicBool,
        progress: &IngressProgress,
        envelope: Value,
        received_at: Instant,
    ) {
        let activity = progress.begin_callback(failed).unwrap();
        let bytes = encode_sysex(&envelope).unwrap();
        enqueue_ingress(
            sender,
            dropped,
            failed,
            progress,
            Ingress::Frame {
                received_at_unix_ms: unix_timestamp_ms(),
                received_at_monotonic: received_at,
                midi_timestamp: 0,
                bytes,
            },
        );
        drop(activity);
    }

    fn event_message(data: Value) -> Value {
        json!({
            "version": 1,
            "type": "event",
            "event": "probe.bank.chunk",
            "data": data
        })
    }

    fn lifecycle_event(event: &str, data: Value) -> Value {
        json!({
            "version": 1,
            "type": "event",
            "event": event,
            "data": data
        })
    }

    fn loaded_message(source: &str) -> Value {
        lifecycle_event(
            "probe.loaded",
            json!({
                "probe_session_id": source,
                "mapping_active": true,
                "read_only": true,
                "protocol_version": 1
            }),
        )
    }

    fn mapping_active_message(source: &str) -> Value {
        lifecycle_event(
            "probe.mapping_active",
            json!({
                "probe_session_id": source,
                "mapping_active": true,
                "read_only": true,
                "protocol_version": 1
            }),
        )
    }

    fn ready_message(source: &str, ready: bool) -> Value {
        lifecycle_event(
            "probe.ready",
            json!({
                "ready": ready,
                "probe_session_id": source,
                "read_only": true,
                "protocol_version": 1,
                "initial_snapshots_complete": ready
            }),
        )
    }

    fn make_source_ready(tracker: &mut ProtocolTracker, source: &str) {
        assert!(
            tracker
                .observe_source_message(source, &loaded_message(source), true)
                .is_empty()
        );
        assert!(
            tracker
                .observe_source_message(source, &mapping_active_message(source), false)
                .is_empty()
        );
        assert!(
            tracker
                .observe_source_message(source, &ready_message(source, true), false)
                .is_empty()
        );
    }

    fn begin_test_checkpoint(tracker: &mut ProtocolTracker, base: Instant) {
        if tracker.active_checkpoint.is_none() {
            tracker
                .begin_checkpoint("test-checkpoint".into(), Duration::from_secs(5), base)
                .unwrap();
        }
    }

    fn select_source(tracker: &mut ProtocolTracker, source: &str, base: Instant) {
        begin_test_checkpoint(tracker, base);
        let discover = request("test-discover", None, "probe.discover", json!({}));
        tracker
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        tracker.mark_request_sent("test-discover", base, Duration::from_secs(1));
        assert!(
            tracker
                .observe_reply(source, "test-discover", MessageKind::Response)
                .is_empty()
        );
        assert!(
            tracker
                .expire_discoveries(base + Duration::from_secs(1))
                .faults
                .is_empty()
        );
        assert_eq!(tracker.selected_source_instance_id.as_deref(), Some(source));
    }

    fn parsed_probe(command: &[u8], sequence: u64) -> ProbeRequestEnvelope {
        match parse_command(command, "session", sequence).unwrap() {
            ParsedCommand::Probe(envelope) => envelope,
            command => panic!("expected probe command, got {command:?}"),
        }
    }

    fn request(
        request_id: &str,
        target_instance_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> ProbeRequestEnvelope {
        ProbeRequestEnvelope {
            probe_transport_version: PROBE_TRANSPORT_VERSION,
            target_instance_id: target_instance_id.map(str::to_owned),
            message: ProbeRequest {
                version: PROBE_MESSAGE_VERSION,
                id: request_id.to_owned(),
                message_type: "request",
                method: method.to_owned(),
                params,
            },
        }
    }

    fn chunk_data(
        snapshot_id: &str,
        chunk_index: usize,
        chunk_count: usize,
        total_items: usize,
        mut items: Vec<Value>,
    ) -> Value {
        for item in &mut items {
            if item.get("record_kind").and_then(Value::as_str) != Some("host_id_fragment")
                && let Some(item) = item.as_object_mut()
            {
                item.entry("config_id").or_insert(json!("MB_CORE_ALL"));
            }
        }
        json!({
            "snapshot_id": snapshot_id,
            "stream": "mixer_bank_snapshot",
            "reason": "command_snapshot",
            "config_id": "MB_CORE_ALL",
            "chunk_index": chunk_index,
            "chunk_count": chunk_count,
            "total_items": total_items,
            "items": items,
            "snapshot_complete": chunk_index + 1 == chunk_count,
            "truncated": false,
            "overflow_safe": true
        })
    }

    #[test]
    fn codec_round_trips_unicode_probe_event_with_unique_header() {
        let value = incoming(
            "track-probe-日本語",
            1,
            event_message(json!({"track": "ボーカル🎹"})),
        );
        let frame = encode_sysex(&value).unwrap();
        assert!(frame.starts_with(&PROBE_SYSEX_HEADER));
        assert_eq!(frame.last(), Some(&0xF7));

        let (decoded, kind) = decode_incoming(&frame).unwrap();
        assert_eq!(decoded.source_instance_id, "track-probe-日本語");
        assert_eq!(decoded.source_seq, 1);
        assert_eq!(decoded.message["data"]["track"], "ボーカル🎹");
        assert_eq!(kind, MessageKind::Event);
    }

    #[test]
    fn codec_rejects_foreign_header_odd_nibbles_and_non_nibbles() {
        let value = incoming("track-probe-1", 1, event_message(json!({})));
        let mut foreign = encode_sysex(&value).unwrap();
        foreign[3] = b'X';
        assert_eq!(decode_incoming(&foreign).unwrap_err().code, "INVALID_FRAME");

        let mut odd = PROBE_SYSEX_HEADER.to_vec();
        odd.extend_from_slice(&[0x01, 0xF7]);
        assert_eq!(decode_incoming(&odd).unwrap_err().code, "ODD_NIBBLE_COUNT");

        let mut non_nibble = PROBE_SYSEX_HEADER.to_vec();
        non_nibble.extend_from_slice(&[0x10, 0x00, 0xF7]);
        assert_eq!(
            decode_incoming(&non_nibble).unwrap_err().code,
            "NON_NIBBLE_BYTE"
        );
    }

    #[test]
    fn codec_rejects_oversize_json_without_truncating() {
        let value = json!({"value": "x".repeat(MAX_JSON_BYTES)});
        let error = encode_sysex(&value).unwrap_err();
        assert_eq!(error.code, "OVERSIZE_FRAME");
        assert!(error.oversize);
    }

    #[test]
    fn incoming_envelope_and_message_shape_are_strict() {
        let missing_sequence = json!({
            "probe_transport_version": 1,
            "source_instance_id": "track-probe-1",
            "message": event_message(json!({}))
        });
        let frame = encode_sysex(&missing_sequence).unwrap();
        assert_eq!(
            decode_incoming(&frame).unwrap_err().code,
            "INVALID_ENVELOPE"
        );

        let invalid_event = incoming(
            "track-probe-1",
            1,
            json!({"version": 1, "type": "event", "event": "probe.ready"}),
        );
        let frame = encode_sysex(&invalid_event).unwrap();
        assert_eq!(decode_incoming(&frame).unwrap_err().code, "INVALID_EVENT");
    }

    #[test]
    fn source_overflow_event_is_recognized_as_an_integrity_fault() {
        let overflow = json!({
            "version": 1,
            "type": "event",
            "event": "probe.overflow",
            "data": {"stream": "outbound_frame", "attempted_json_bytes": 4097}
        });
        assert_eq!(
            source_overflow_data(MessageKind::Event, &overflow).unwrap()["stream"],
            "outbound_frame"
        );
        assert!(source_overflow_data(MessageKind::Event, &event_message(json!({}))).is_none());
        assert!(source_overflow_data(MessageKind::Response, &overflow).is_none());
    }

    #[test]
    fn framer_reassembles_split_frames_and_reports_nested_start() {
        let frame = encode_sysex(&incoming("track-probe-1", 1, event_message(json!({})))).unwrap();
        let split = frame.len() / 2;
        let mut framer = SysexFramer::default();
        assert!(framer.push(&[0x90, 60, 127]).is_empty());
        assert!(framer.push(&frame[..split]).is_empty());
        assert_eq!(
            framer.push(&frame[split..]),
            vec![FramerItem::Frame(frame.clone())]
        );

        assert!(framer.push(&frame[..split]).is_empty());
        let nested = framer.push(&frame);
        assert_eq!(nested[0], FramerItem::Fault(FramingFault::NestedStart));
        assert_eq!(nested[1], FramerItem::Frame(frame));
    }

    #[test]
    fn framer_bounds_oversize_input_and_remembers_partial_shutdown() {
        let mut framer = SysexFramer::default();
        assert!(framer.push(&PROBE_SYSEX_HEADER).is_empty());
        assert!(framer.has_partial_frame());
        let padding = vec![0; MAX_SYSEX_BYTES];
        let items = framer.push(&padding);
        assert_eq!(items, vec![FramerItem::Fault(FramingFault::Oversize)]);
        assert!(!framer.has_partial_frame());
    }

    #[test]
    fn receive_ignores_only_the_exact_broadcast_identity_request() {
        let (sender, receiver) = mpsc::sync_channel(16);
        let dropped = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(IngressProgress::default());
        let mut state = MidiCallbackState {
            sender,
            framer: SysexFramer::default(),
            dropped_items: Arc::clone(&dropped),
            integrity_failed: Arc::clone(&failed),
            ingress_progress: Arc::clone(&progress),
        };

        for split in 1..MIDI_BROADCAST_IDENTITY_REQUEST.len() {
            receive_midi(0, &MIDI_BROADCAST_IDENTITY_REQUEST[..split], &mut state);
            assert!(state.framer.has_partial_frame());
            assert!(receiver.try_recv().is_err());
            receive_midi(0, &MIDI_BROADCAST_IDENTITY_REQUEST[split..], &mut state);
            assert!(!state.framer.has_partial_frame());
            assert!(receiver.try_recv().is_err());
        }
        receive_midi(0, &MIDI_BROADCAST_IDENTITY_REQUEST, &mut state);

        assert!(!state.framer.has_partial_frame());
        assert!(receiver.try_recv().is_err());
        assert_eq!(progress.received.load(Ordering::Acquire), 0);
        assert_eq!(dropped.load(Ordering::Acquire), 0);
        assert!(!failed.load(Ordering::Acquire));
        progress.synchronize(&failed).unwrap();

        let near_matches = [
            [0xF0, 0x7E, 0x01, 0x06, 0x01, 0xF7].as_slice(),
            [0xF0, 0x7E, 0x7F, 0x06, 0x02, 0xF7].as_slice(),
            [0xF0, 0x7F, 0x7F, 0x06, 0x01, 0xF7].as_slice(),
            [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0x00, 0xF7].as_slice(),
        ];
        for near_match in near_matches {
            receive_midi(0, near_match, &mut state);
            let Ingress::Frame { bytes, .. } = receiver.try_recv().unwrap() else {
                panic!("a near-match must remain normal fail-closed ingress");
            };
            assert_eq!(bytes, near_match);
            assert_eq!(decode_incoming(&bytes).unwrap_err().code, "INVALID_FRAME");
            progress.mark_processed(&failed);
        }
        assert_eq!(progress.received.load(Ordering::Acquire), 4);
        progress.synchronize(&failed).unwrap();

        let first_probe =
            encode_sysex(&incoming("source-a", 1, loaded_message("source-a"))).unwrap();
        let second_probe =
            encode_sysex(&incoming("source-a", 2, mapping_active_message("source-a"))).unwrap();
        let mixed = [
            MIDI_BROADCAST_IDENTITY_REQUEST.as_slice(),
            first_probe.as_slice(),
            MIDI_BROADCAST_IDENTITY_REQUEST.as_slice(),
            second_probe.as_slice(),
        ]
        .concat();
        receive_midi(0, &mixed, &mut state);
        for expected in [&first_probe, &second_probe] {
            let Ingress::Frame { bytes, .. } = receiver.try_recv().unwrap() else {
                panic!("probe frames surrounding identity noise must be retained");
            };
            assert_eq!(&bytes, expected);
            progress.mark_processed(&failed);
        }
        assert!(receiver.try_recv().is_err());
        assert_eq!(progress.received.load(Ordering::Acquire), 6);
        progress.synchronize(&failed).unwrap();

        receive_midi(0, &first_probe[..3], &mut state);
        assert!(state.framer.has_partial_frame());
        receive_midi(0, &MIDI_BROADCAST_IDENTITY_REQUEST, &mut state);
        let Ingress::FramingFault { fault, .. } = receiver.try_recv().unwrap() else {
            panic!("identity traffic must not hide a nested SysEx fault");
        };
        assert_eq!(fault, FramingFault::NestedStart);
        assert!(receiver.try_recv().is_err());
        assert!(failed.load(Ordering::Acquire));
        progress.mark_processed(&failed);
        assert!(!state.framer.has_partial_frame());
    }

    #[test]
    fn identity_request_does_not_consume_a_full_ingress_queue() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(IngressProgress::default());
        let mut state = MidiCallbackState {
            sender,
            framer: SysexFramer::default(),
            dropped_items: Arc::clone(&dropped),
            integrity_failed: Arc::clone(&failed),
            ingress_progress: Arc::clone(&progress),
        };
        let probe = encode_sysex(&incoming("source-a", 1, loaded_message("source-a"))).unwrap();

        receive_midi(0, &probe, &mut state);
        receive_midi(0, &MIDI_BROADCAST_IDENTITY_REQUEST, &mut state);

        assert_eq!(dropped.load(Ordering::Acquire), 0);
        assert!(!failed.load(Ordering::Acquire));
        assert_eq!(progress.received.load(Ordering::Acquire), 1);
        let Ingress::Frame { bytes, .. } = receiver.try_recv().unwrap() else {
            panic!("the queued probe frame must remain intact");
        };
        assert_eq!(bytes, probe);
        progress.mark_processed(&failed);
        progress.synchronize(&failed).unwrap();
    }

    #[test]
    fn bounded_ingress_queue_records_drop_and_fails_closed() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let dropped = AtomicU64::new(0);
        let failed = AtomicBool::new(false);
        let progress = IngressProgress::default();
        let ingress = || Ingress::FramingFault {
            received_at_unix_ms: 1,
            received_at_monotonic: Instant::now(),
            fault: FramingFault::NestedStart,
        };

        enqueue_ingress(&sender, &dropped, &failed, &progress, ingress());
        enqueue_ingress(&sender, &dropped, &failed, &progress, ingress());

        assert_eq!(dropped.load(Ordering::Acquire), 1);
        assert!(failed.load(Ordering::Acquire));
    }

    #[test]
    fn ingress_barrier_waits_for_active_callback_and_tracker_processing() {
        let progress = Arc::new(IngressProgress::default());
        let failed = Arc::new(AtomicBool::new(false));
        let activity = progress.begin_callback(&failed).unwrap();
        assert!(progress.reserve_received(&failed));

        let (started_sender, started_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let worker_failed = Arc::clone(&failed);
        let worker = thread::spawn(move || {
            started_sender.send(()).unwrap();
            done_sender
                .send(worker_progress.synchronize(&worker_failed))
                .unwrap();
        });
        started_receiver.recv().unwrap();
        assert!(done_receiver.try_recv().is_err());

        drop(activity);
        assert!(
            done_receiver
                .recv_timeout(Duration::from_millis(20))
                .is_err()
        );
        progress.mark_processed(&failed);
        assert!(
            done_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_ok()
        );
        worker.join().unwrap();
        assert!(!failed.load(Ordering::Acquire));
    }

    #[test]
    fn held_ingress_barrier_revalidates_target_after_queued_deactivation() {
        let base = Instant::now();
        let runtime = Arc::new(RuntimeTracker::new());
        let request = request(
            "queued-deactivation",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        {
            let mut tracker = runtime.state.lock().unwrap();
            make_source_ready(&mut tracker, "source-a");
            select_source(&mut tracker, "source-a", base);
            tracker
                .register_request(&request, base, Duration::from_secs(1))
                .unwrap();
        }

        let progress = Arc::new(IngressProgress::default());
        let failed = Arc::new(AtomicBool::new(false));
        let activity = progress.begin_callback(&failed).unwrap();
        assert!(progress.reserve_received(&failed));
        let (started_sender, started_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let worker_failed = Arc::clone(&failed);
        let worker_runtime = Arc::clone(&runtime);
        let worker = thread::spawn(move || {
            started_sender.send(()).unwrap();
            let barrier = worker_progress.synchronize_held(&worker_failed).unwrap();
            let tracker = worker_runtime.state.lock().unwrap();
            let code = tracker
                .validate_request_before_send("queued-deactivation")
                .unwrap_err()
                .code;
            drop(tracker);
            drop(barrier);
            done_sender.send(code).unwrap();
        });
        started_receiver.recv().unwrap();
        assert!(done_receiver.try_recv().is_err());

        drop(activity);
        {
            let mut tracker = runtime.state.lock().unwrap();
            assert!(
                tracker
                    .observe_source_message("source-a", &ready_message("source-a", false), false,)
                    .is_empty()
            );
            tracker.checkpoint_context(base + Duration::from_secs(2));
        }
        progress.mark_processed(&failed);
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            "TARGET_CHANGED_BEFORE_SEND"
        );
        worker.join().unwrap();
        assert!(!failed.load(Ordering::Acquire));
    }

    #[test]
    fn collector_applies_queued_deactivation_before_send_barrier_completes() {
        let base = Instant::now() - Duration::from_millis(1);
        let runtime = Arc::new(RuntimeTracker::new());
        runtime
            .state
            .lock()
            .unwrap()
            .begin_checkpoint("integration".into(), Duration::from_secs(5), base)
            .unwrap();
        let progress = Arc::new(IngressProgress::default());
        let failed = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::new(JsonlSink {
            writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
            failed: AtomicBool::new(false),
            run_id: "integration".into(),
            started_at: base,
            record_prepare_hook: None,
        });
        let collector_runtime = Arc::clone(&runtime);
        let collector_progress = Arc::clone(&progress);
        let collector_failed = Arc::clone(&failed);
        let collector_dropped = Arc::clone(&dropped);
        let collector_sink = Arc::clone(&sink);
        let collector = thread::spawn(move || {
            collect_incoming(
                receiver,
                collector_dropped,
                collector_failed,
                collector_sink,
                collector_runtime,
                collector_progress,
            )
        });

        enqueue_test_incoming(
            &sender,
            &dropped,
            &failed,
            &progress,
            incoming("source-a", 1, loaded_message("source-a")),
            Instant::now(),
        );
        enqueue_test_incoming(
            &sender,
            &dropped,
            &failed,
            &progress,
            incoming("source-a", 2, mapping_active_message("source-a")),
            Instant::now(),
        );
        enqueue_test_incoming(
            &sender,
            &dropped,
            &failed,
            &progress,
            incoming("source-a", 3, ready_message("source-a", true)),
            Instant::now(),
        );
        progress.synchronize(&failed).unwrap();

        let discover = request("integration-discover", None, "probe.discover", json!({}));
        let sent_at = Instant::now();
        {
            let mut tracker = runtime.state.lock().unwrap();
            tracker
                .register_request(&discover, sent_at, Duration::from_secs(1))
                .unwrap();
            tracker.mark_request_sent("integration-discover", sent_at, Duration::from_secs(1));
        }
        enqueue_test_incoming(
            &sender,
            &dropped,
            &failed,
            &progress,
            incoming(
                "source-a",
                4,
                json!({
                    "version": 1,
                    "id": "integration-discover",
                    "type": "response",
                    "result": {
                        "instance_id": "source-a",
                        "ready": true,
                        "read_only": true
                    }
                }),
            ),
            Instant::now(),
        );
        progress.synchronize(&failed).unwrap();
        {
            let mut tracker = runtime.state.lock().unwrap();
            assert!(
                tracker
                    .expire_discoveries(sent_at + Duration::from_secs(1))
                    .faults
                    .is_empty()
            );
            let targeted = request(
                "integration-targeted",
                Some("source-a"),
                "probe.capabilities.get",
                json!({}),
            );
            tracker
                .register_request(&targeted, Instant::now(), Duration::from_secs(1))
                .unwrap();
        }

        let tracker_guard = runtime.state.lock().unwrap();
        let split_frame =
            encode_sysex(&incoming("source-a", 5, ready_message("source-a", false))).unwrap();
        let split = split_frame.len() / 2;
        let mut callback_state = MidiCallbackState {
            sender: sender.clone(),
            framer: SysexFramer::default(),
            dropped_items: Arc::clone(&dropped),
            integrity_failed: Arc::clone(&failed),
            ingress_progress: Arc::clone(&progress),
        };
        receive_midi(0, &split_frame[..split], &mut callback_state);
        assert!(callback_state.framer.has_partial_frame());
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let worker_failed = Arc::clone(&failed);
        let worker_runtime = Arc::clone(&runtime);
        let worker = thread::spawn(move || {
            let barrier = worker_progress.synchronize_held(&worker_failed).unwrap();
            let tracker = worker_runtime.state.lock().unwrap();
            let code = tracker
                .validate_request_before_send("integration-targeted")
                .unwrap_err()
                .code;
            drop(tracker);
            drop(barrier);
            done_sender.send(code).unwrap();
        });
        assert!(done_receiver.try_recv().is_err());
        receive_midi(0, &split_frame[split..], &mut callback_state);
        assert!(!callback_state.framer.has_partial_frame());
        drop(tracker_guard);
        assert_eq!(
            done_receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            "TARGET_CHANGED_BEFORE_SEND"
        );
        worker.join().unwrap();
        runtime
            .state
            .lock()
            .unwrap()
            .cancel_request("integration-targeted");
        drop(callback_state);
        drop(sender);
        let report = collector.join().unwrap();
        assert_eq!(report.diagnostics, 0);
        assert!(!failed.load(Ordering::Acquire));
    }

    #[test]
    fn sequence_tracker_detects_first_gap_duplicate_and_isolates_sources() {
        let mut tracker = SequenceTracker::default();
        assert!(matches!(
            tracker.observe("source-a", 1),
            SequenceObservation::Contiguous
        ));
        assert!(matches!(
            tracker.observe("source-a", 2),
            SequenceObservation::Contiguous
        ));
        assert!(matches!(
            tracker.observe("source-a", 4),
            SequenceObservation::Gap {
                expected: 3,
                actual: 4
            }
        ));
        assert!(matches!(
            tracker.observe("source-a", 4),
            SequenceObservation::DuplicateOrReorder {
                expected: 5,
                actual: 4
            }
        ));
        assert!(matches!(
            tracker.observe("source-b", 2),
            SequenceObservation::Gap {
                expected: 1,
                actual: 2
            }
        ));
    }

    #[test]
    fn collector_adds_request_id_and_enforces_discovery_target_rules() {
        let discover = parsed_probe(br#"{"method":"probe.discover","params":{}}"#, 7);
        assert_eq!(discover.message.id, "session-7");
        assert_eq!(discover.target_instance_id, None);
        assert_eq!(discover.message.message_type, "request");

        let reset = parsed_probe(
            br#"{"target_instance_id":"track-probe-1","method":"probe.bank.reset"}"#,
            8,
        );
        assert_eq!(reset.message.params, json!({}));
        let selected = parsed_probe(
            br#"{"target_instance_id":"@selected","method":"probe.bank.reset"}"#,
            9,
        );
        assert_eq!(
            selected.target_instance_id.as_deref(),
            Some(SELECTED_TARGET_ALIAS)
        );

        assert!(
            parse_command(
                br#"{"method":"probe.bank.next","params":{}}"#,
                "session",
                10
            )
            .is_err()
        );
        assert!(
            parse_command(
                br#"{"target_instance_id":"track-probe-1","method":"probe.discover"}"#,
                "session",
                11
            )
            .is_err()
        );
        assert!(
            parse_command(
                br#"{"id":"caller-id","method":"probe.discover"}"#,
                "session",
                12
            )
            .is_err()
        );
    }

    #[test]
    fn streaming_sha256_matches_lowercase_standard_vectors() {
        assert_eq!(
            sha256_reader(Cursor::new([])).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let digest = sha256_reader(Cursor::new(b"abc")).unwrap();
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn streaming_sha256_propagates_reader_failure() {
        assert_eq!(
            sha256_reader(FailingReader).unwrap_err().to_string(),
            "deterministic read failure"
        );
    }

    #[test]
    fn selected_target_alias_resolves_only_after_exactly_one_discovery() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        begin_test_checkpoint(&mut tracker, base);
        let mut selected = request(
            "selected",
            Some(SELECTED_TARGET_ALIAS),
            "probe.capabilities.get",
            json!({}),
        );

        assert_eq!(
            tracker
                .register_request(&selected, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "DISCOVERY_REQUIRED"
        );

        tracker
            .source_lifecycle
            .insert("source-b".into(), SourceLifecycle::Ready);
        assert_eq!(
            tracker
                .register_request(&selected, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "DISCOVERY_REQUIRED"
        );
        tracker.source_lifecycle.remove("source-b");

        select_source(&mut tracker, "source-a", base);
        tracker
            .register_request(&selected, base, Duration::from_secs(1))
            .unwrap();
        tracker
            .resolve_selected_target_alias_before_send(&mut selected)
            .unwrap();
        assert_eq!(selected.target_instance_id.as_deref(), Some("source-a"));

        let frame = encode_request_sysex(&selected).unwrap();
        let encoded = decode_json_value(&frame).unwrap();
        assert_eq!(encoded["target_instance_id"], "source-a");
        assert!(
            !serde_json::to_string(&encoded)
                .unwrap()
                .contains(SELECTED_TARGET_ALIAS)
        );
    }

    #[test]
    fn observation_cut_uses_selected_target_and_completes_without_chunk_followup() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);

        let mut cut = parsed_probe(
            br#"{"target_instance_id":"@selected","method":"probe.observation.cut"}"#,
            10,
        );
        assert_eq!(cut.message.method, "probe.observation.cut");
        assert_eq!(cut.message.params, json!({}));
        assert_eq!(
            cut.target_instance_id.as_deref(),
            Some(SELECTED_TARGET_ALIAS)
        );

        let before = tracker.summary();
        tracker
            .register_request(&cut, base, Duration::from_secs(1))
            .unwrap();
        assert!(
            tracker
                .pending_requests
                .get("session-10")
                .is_some_and(|pending| pending.followup.is_none())
        );
        tracker
            .resolve_selected_target_alias_before_send(&mut cut)
            .unwrap();
        assert_eq!(cut.target_instance_id.as_deref(), Some("source-a"));
        let encoded = decode_json_value(&encode_request_sysex(&cut).unwrap()).unwrap();
        assert_eq!(encoded["message"]["method"], "probe.observation.cut");
        assert_eq!(encoded["message"]["params"], json!({}));

        assert!(
            tracker
                .observe_reply_message(
                    "source-a",
                    "session-10",
                    MessageKind::Response,
                    &json!({"result": {"observation_epoch": 7}}),
                    Some("test-checkpoint"),
                )
                .is_empty()
        );
        assert!(tracker.pending_requests.is_empty());
        assert!(tracker.expected_followups.is_empty());
        assert!(tracker.is_quiescent());
        assert_eq!(
            tracker
                .mark_observation_cut_action("test-checkpoint", "session-10", 7)
                .unwrap(),
            json!({
                "record_type": "collector_action",
                "phase": "marked",
                "checkpoint_id": "test-checkpoint",
                "boundary_source": "probe.observation.cut_response",
                "request_id": "session-10",
                "observation_epoch": 7
            })
        );
        assert_eq!(
            tracker.mark_action("test-checkpoint").unwrap_err().code,
            "ACTION_ALREADY_MARKED"
        );

        let after = tracker.summary();
        assert_eq!(
            after["completed_requests"].as_u64(),
            before["completed_requests"].as_u64().map(|count| count + 1)
        );
        assert_eq!(after["completed_chunk_streams"], 0);
        assert_eq!(after["completed_snapshot_streams"], 0);
        assert_eq!(after["completed_feedback_streams"], 0);
    }

    #[test]
    fn observation_cut_response_epoch_is_exact_and_bounded() {
        assert_eq!(
            observation_cut_response_epoch(&json!({
                "result": {"observation_epoch": 1}
            })),
            Some(1)
        );
        assert_eq!(
            observation_cut_response_epoch(&json!({
                "result": {"observation_epoch": MAX_OBSERVATION_EPOCH}
            })),
            Some(MAX_OBSERVATION_EPOCH)
        );
        for invalid in [
            json!({"result": {"observation_epoch": 0}}),
            json!({"result": {"observation_epoch": MAX_OBSERVATION_EPOCH + 1}}),
            json!({"result": {"observation_epoch": 1, "unexpected": true}}),
            json!({"result": {}}),
            json!({"result": {"observation_epoch": "1"}}),
        ] {
            assert_eq!(observation_cut_response_epoch(&invalid), None);
        }
    }

    #[test]
    fn unresolved_selected_target_alias_can_never_be_encoded_for_midi() {
        let selected = request(
            "selected",
            Some(SELECTED_TARGET_ALIAS),
            "probe.capabilities.get",
            json!({}),
        );
        let error = encode_request_sysex(&selected).unwrap_err();
        assert_eq!(error.code, "UNRESOLVED_TARGET_ALIAS");
    }

    #[test]
    fn post_send_evidence_failure_preserves_sent_count_and_invalidates_run() {
        let mut report = CommandReport {
            sent: 1,
            ..CommandReport::default()
        };
        let integrity_failed = AtomicBool::new(false);
        invalidate_after_selected_evidence_failure(&mut report, &integrity_failed, false);
        assert_eq!(report.sent, 1);
        assert!(report.input_failed);
        assert_eq!(report.exit_reason, "stdout_error_after_midi_send");
        assert!(integrity_failed.load(Ordering::Acquire));

        let mut failed_send_report = CommandReport {
            input_failed: true,
            exit_reason: "midi_send_error",
            ..CommandReport::default()
        };
        invalidate_after_selected_evidence_failure(
            &mut failed_send_report,
            &integrity_failed,
            true,
        );
        assert_eq!(failed_send_report.sent, 0);
        assert_eq!(failed_send_report.exit_reason, "midi_send_error");
    }

    #[test]
    fn selected_target_alias_fails_closed_when_lifecycle_changes_before_send() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        let mut selected = request(
            "selected",
            Some(SELECTED_TARGET_ALIAS),
            "probe.capabilities.get",
            json!({}),
        );
        tracker
            .register_request(&selected, base, Duration::from_secs(1))
            .unwrap();
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", false), false)
                .is_empty()
        );
        assert_eq!(
            tracker
                .resolve_selected_target_alias_before_send(&mut selected)
                .unwrap_err()
                .code,
            "TARGET_CHANGED_BEFORE_SEND"
        );
        assert_eq!(
            selected.target_instance_id.as_deref(),
            Some(SELECTED_TARGET_ALIAS)
        );
        assert_eq!(
            encode_request_sysex(&selected).unwrap_err().code,
            "UNRESOLVED_TARGET_ALIAS"
        );
    }

    #[test]
    fn outbound_probe_request_respects_script_four_kibibyte_limit() {
        let command = json!({
            "method": "probe.discover",
            "params": {"padding": "x".repeat(MAX_OUTBOUND_JSON_BYTES)}
        });
        let envelope = parsed_probe(command.to_string().as_bytes(), 1);
        let error = encode_request_sysex(&envelope).unwrap_err();
        assert_eq!(error.code, "OVERSIZE_COMMAND");
    }

    #[test]
    fn cli_defaults_to_virtual_ports_but_windows_requires_explicit_pair() {
        assert_eq!(
            Config::parse(["--run-id".into(), "run-1".into()], true).unwrap(),
            CliAction::Run(Config {
                run_id: "run-1".into(),
                midi_input: None,
                midi_output: None,
                graceful_drain: Duration::from_millis(DEFAULT_GRACEFUL_DRAIN_MS),
                discovery_window: Duration::from_millis(DEFAULT_DISCOVERY_WINDOW_MS)
            })
        );
        assert!(Config::parse(Vec::<String>::new(), false).is_err());

        let action = Config::parse(
            [
                "--run-id".into(),
                "run-2".into(),
                "--midi-input".into(),
                "From Cubase".into(),
                "--midi-output".into(),
                "To Cubase".into(),
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            action,
            CliAction::Run(Config {
                run_id: "run-2".into(),
                midi_input: Some("From Cubase".into()),
                midi_output: Some("To Cubase".into()),
                graceful_drain: Duration::from_millis(DEFAULT_GRACEFUL_DRAIN_MS),
                discovery_window: Duration::from_millis(DEFAULT_DISCOVERY_WINDOW_MS)
            })
        );
    }

    #[test]
    fn cli_requires_run_id_and_bounds_explicit_windows() {
        assert!(Config::parse(Vec::<String>::new(), true).is_err());
        assert!(
            Config::parse(
                [
                    "--run-id".into(),
                    "run".into(),
                    "--drain-timeout-ms".into(),
                    "0".into(),
                ],
                true,
            )
            .is_err()
        );
        let action = Config::parse(
            [
                "--run-id".into(),
                "run".into(),
                "--drain-timeout-ms".into(),
                "17".into(),
                "--discovery-window-ms".into(),
                "23".into(),
            ],
            true,
        )
        .unwrap();
        let CliAction::Run(config) = action else {
            panic!("expected run action");
        };
        assert_eq!(config.graceful_drain, Duration::from_millis(17));
        assert_eq!(config.discovery_window, Duration::from_millis(23));
    }

    #[test]
    fn existing_port_selection_preserves_the_resolved_name() {
        let (port, resolved) = select_port(
            "Probe From",
            vec![(7, "Driver Probe From Cubase 2".to_owned())].into_iter(),
            "input",
        )
        .unwrap();
        assert_eq!(port, 7);
        assert_eq!(resolved, "Driver Probe From Cubase 2");
    }

    #[test]
    fn checkpoint_commands_are_local_strict_and_bounded() {
        let begin = parse_command(
            br#"{"method":"collector.checkpoint.begin","params":{"checkpoint_id":"take-1","window_ms":5000}}"#,
            "session",
            1,
        )
        .unwrap();
        assert!(matches!(
            begin,
            ParsedCommand::CheckpointBegin {
                checkpoint_id,
                window
            } if checkpoint_id == "take-1" && window == Duration::from_secs(5)
        ));
        let end = parse_command(
            br#"{"method":"collector.checkpoint.end","params":{"checkpoint_id":"take-1"}}"#,
            "session",
            1,
        )
        .unwrap();
        assert!(matches!(
            end,
            ParsedCommand::CheckpointEnd { checkpoint_id } if checkpoint_id == "take-1"
        ));
        let action = parse_command(
            br#"{"method":"collector.action","params":{"checkpoint_id":"take-1"}}"#,
            "session",
            1,
        )
        .unwrap();
        assert!(matches!(
            action,
            ParsedCommand::Action { checkpoint_id } if checkpoint_id == "take-1"
        ));
        assert!(
            parse_command(
                br#"{"target_instance_id":"probe","method":"collector.checkpoint.end","params":{"checkpoint_id":"take-1"}}"#,
                "session",
                1,
            )
            .is_err()
        );
        assert!(
            parse_command(
                br#"{"method":"collector.checkpoint.begin","params":{"checkpoint_id":"take-1","window_ms":0}}"#,
                "session",
                1,
            )
            .is_err()
        );
        assert!(
            parse_command(
                br#"{"target_instance_id":"probe","method":"collector.action","params":{"checkpoint_id":"take-1"}}"#,
                "session",
                1,
            )
            .is_err()
        );
        assert!(
            parse_command(
                br#"{"method":"collector.action","params":{"checkpoint_id":"take-1","unexpected":true}}"#,
                "session",
                1,
            )
            .is_err()
        );
    }

    #[test]
    fn action_marker_requires_matching_quiescent_checkpoint_and_is_exactly_once() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        assert_eq!(
            tracker.mark_action("take-1").unwrap_err().code,
            "ACTION_CHECKPOINT_NOT_ACTIVE"
        );
        tracker
            .begin_checkpoint("take-1".into(), Duration::from_secs(5), base)
            .unwrap();
        assert_eq!(
            tracker.mark_action("take-2").unwrap_err().code,
            "ACTION_CHECKPOINT_MISMATCH"
        );
        let marker = tracker.mark_action("take-1").unwrap();
        assert_eq!(marker["record_type"], "collector_action");
        assert_eq!(marker["phase"], "marked");
        assert_eq!(marker["checkpoint_id"], "take-1");
        assert_eq!(
            tracker.mark_action("take-1").unwrap_err().code,
            "ACTION_ALREADY_MARKED"
        );

        let mut busy = ProtocolTracker::default();
        make_source_ready(&mut busy, "source-a");
        select_source(&mut busy, "source-a", base);
        let request = request(
            "busy-request",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        busy.register_request(&request, base, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            busy.mark_action("test-checkpoint").unwrap_err().code,
            "ACTION_PROTOCOL_NOT_QUIESCENT"
        );
    }

    #[test]
    fn targeted_requests_are_exactly_once_and_source_bound() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        let first_request = request(
            "request-1",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        tracker
            .register_request(&first_request, base, Duration::from_secs(1))
            .unwrap();
        let mismatch = tracker.observe_reply("source-b", "request-1", MessageKind::Response);
        assert_eq!(mismatch[0].code, "RESPONSE_SOURCE_MISMATCH");
        assert!(!tracker.is_quiescent());
        assert!(
            tracker
                .observe_reply("source-a", "request-1", MessageKind::Response)
                .is_empty()
        );
        assert!(tracker.is_quiescent());
        assert_eq!(
            tracker.observe_reply("source-a", "request-1", MessageKind::Error)[0].code,
            "DUPLICATE_RESPONSE"
        );
        assert_eq!(
            tracker.observe_reply("source-a", "unknown", MessageKind::Response)[0].code,
            "UNMATCHED_RESPONSE"
        );
    }

    #[test]
    fn discovery_collects_window_then_requires_exactly_one_source() {
        let base = Instant::now();
        let mut one = ProtocolTracker::default();
        make_source_ready(&mut one, "source-a");
        begin_test_checkpoint(&mut one, base);
        let discover = request("discover-1", None, "probe.discover", json!({}));
        one.register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        one.mark_request_sent("discover-1", base, Duration::from_secs(1));
        assert!(
            one.observe_reply("source-a", "discover-1", MessageKind::Response)
                .is_empty()
        );
        assert!(!one.is_quiescent());
        assert!(
            one.expire_discoveries(base + Duration::from_secs(1))
                .faults
                .is_empty()
        );
        assert!(one.is_quiescent());

        let mut multiple = ProtocolTracker::default();
        multiple
            .source_lifecycle
            .insert("source-a".into(), SourceLifecycle::Ready);
        multiple
            .source_lifecycle
            .insert("source-b".into(), SourceLifecycle::Ready);
        begin_test_checkpoint(&mut multiple, base);
        multiple
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        multiple.mark_request_sent("discover-1", base, Duration::from_secs(1));
        assert!(
            multiple
                .observe_reply("source-a", "discover-1", MessageKind::Response)
                .is_empty()
        );
        assert!(
            multiple
                .observe_reply("source-b", "discover-1", MessageKind::Response)
                .is_empty()
        );
        let expiry = multiple.expire_discoveries(base + Duration::from_secs(1));
        assert_eq!(expiry.faults[0].code, "DISCOVERY_MULTIPLE_RESPONDERS");

        let mut none = ProtocolTracker::default();
        begin_test_checkpoint(&mut none, base);
        none.register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        none.mark_request_sent("discover-1", base, Duration::from_secs(1));
        assert_eq!(
            none.expire_discoveries(base + Duration::from_secs(1))
                .faults[0]
                .code,
            "DISCOVERY_NO_RESPONSE"
        );
    }

    #[test]
    fn discovery_window_is_armed_only_after_successful_send() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        begin_test_checkpoint(&mut tracker, base);
        let discover = request("unsent-discover", None, "probe.discover", json!({}));
        tracker
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            tracker
                .expire_discoveries(base + Duration::from_secs(60))
                .expired,
            0
        );
        assert!(tracker.pending_requests.contains_key("unsent-discover"));

        let sent_at = base + Duration::from_secs(60);
        tracker.mark_request_sent("unsent-discover", sent_at, Duration::from_secs(1));
        let expiry = tracker.expire_discoveries(sent_at + Duration::from_secs(1));
        assert_eq!(expiry.expired, 1);
        assert_eq!(expiry.faults[0].code, "DISCOVERY_NO_RESPONSE");
    }

    #[test]
    fn discovery_rejects_duplicate_response_from_one_source() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        begin_test_checkpoint(&mut tracker, base);
        let discover = request("discover-1", None, "probe.discover", json!({}));
        tracker
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        tracker.mark_request_sent("discover-1", base, Duration::from_secs(1));
        assert!(
            tracker
                .observe_reply("source-a", "discover-1", MessageKind::Response)
                .is_empty()
        );
        assert_eq!(
            tracker.observe_reply("source-a", "discover-1", MessageKind::Response)[0].code,
            "DUPLICATE_DISCOVERY_RESPONSE"
        );

        let mut error = ProtocolTracker::default();
        make_source_ready(&mut error, "source-a");
        begin_test_checkpoint(&mut error, base);
        error
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        error.mark_request_sent("discover-1", base, Duration::from_secs(1));
        assert_eq!(
            error.observe_reply("source-a", "discover-1", MessageKind::Error)[0].code,
            "DISCOVERY_RESPONSE_INVALID"
        );
    }

    #[test]
    fn discovery_responder_tracking_is_bounded() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        begin_test_checkpoint(&mut tracker, base);
        let discover = request("discover", None, "probe.discover", json!({}));
        tracker
            .register_request(&discover, base, Duration::from_secs(600))
            .unwrap();
        tracker.mark_request_sent("discover", base, Duration::from_secs(600));
        for index in 0..MAX_SOURCE_INSTANCES {
            let source = format!("source-{index}");
            tracker
                .source_lifecycle
                .insert(source.clone(), SourceLifecycle::Ready);
            tracker.observe_reply(&source, "discover", MessageKind::Response);
        }
        let faults = tracker.observe_reply("source-overflow", "discover", MessageKind::Response);
        assert_eq!(faults[0].code, "DISCOVERY_RESPONDER_CAPACITY");
        let pending = tracker.pending_requests.get("discover").unwrap();
        let PendingMode::Discovery {
            observed_sources, ..
        } = &pending.mode
        else {
            panic!("expected discovery");
        };
        assert_eq!(observed_sources.len(), MAX_SOURCE_INSTANCES);
    }

    #[test]
    fn chunk_tracker_accepts_complete_ordered_snapshot_and_resolves_followup() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        let request = request(
            "snapshot-1",
            Some("source-a"),
            "probe.bank.snapshot",
            json!({"config_id": "MB_CORE_ALL"}),
        );
        tracker
            .register_request(&request, base, Duration::from_secs(1))
            .unwrap();
        assert!(
            tracker
                .observe_reply("source-a", "snapshot-1", MessageKind::Response)
                .is_empty()
        );
        assert_eq!(tracker.expected_followups.len(), 1);
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-1", 0, 2, 2, vec![json!({"track": "A"})]),
                )
                .is_empty()
        );
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-1", 1, 2, 2, vec![json!({"track": "B"})]),
                )
                .is_empty()
        );
        assert!(tracker.is_quiescent());
        assert!(tracker.completed_snapshots.contains(&SnapshotKey {
            source_instance_id: "source-a".into(),
            snapshot_id: "snap-1".into()
        }));
        assert_eq!(tracker.completed_snapshot_streams, 1);
        assert_eq!(tracker.completed_feedback_streams, 0);

        let mut feedback = chunk_data(
            "feedback-1",
            0,
            1,
            1,
            vec![json!({"record_kind": "observation", "config_id": "MB_CORE_ALL"})],
        );
        let feedback = feedback.as_object_mut().unwrap();
        feedback.insert("stream".into(), json!("mixer_bank_feedback"));
        feedback.insert("reason".into(), json!("feedback"));
        feedback.remove("config_id");
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &Value::Object(feedback.clone()),
                )
                .is_empty()
        );
        assert_eq!(tracker.completed_snapshot_streams, 1);
        assert_eq!(tracker.completed_feedback_streams, 1);
        let summary = tracker.summary();
        assert_eq!(summary["completed_chunk_streams"], 2);
        assert_eq!(summary["completed_snapshot_streams"], 1);
        assert_eq!(summary["completed_feedback_streams"], 1);
    }

    #[test]
    fn chunk_tracker_fails_closed_on_missing_duplicate_and_reordered_chunks() {
        let mut missing = ProtocolTracker::default();
        assert_eq!(
            missing.observe_chunk_event(
                "source-a",
                "probe.bank.chunk",
                &chunk_data("snap-missing", 1, 2, 2, vec![json!({"track": "B"})]),
            )[0]
            .code,
            "SNAPSHOT_FIRST_CHUNK_MISSING"
        );

        let mut duplicate = ProtocolTracker::default();
        assert!(
            duplicate
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-duplicate", 0, 3, 3, vec![json!({"track": "A"})]),
                )
                .is_empty()
        );
        let faults = duplicate.observe_chunk_event(
            "source-a",
            "probe.bank.chunk",
            &chunk_data("snap-duplicate", 0, 3, 3, vec![json!({"track": "A"})]),
        );
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "SNAPSHOT_CHUNK_DUPLICATE_OR_REORDER")
        );

        let mut reordered = ProtocolTracker::default();
        assert!(
            reordered
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-reordered", 0, 3, 3, vec![json!({"track": "A"})]),
                )
                .is_empty()
        );
        let faults = reordered.observe_chunk_event(
            "source-a",
            "probe.bank.chunk",
            &chunk_data("snap-reordered", 2, 3, 3, vec![json!({"track": "C"})]),
        );
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "SNAPSHOT_CHUNK_DUPLICATE_OR_REORDER")
        );
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "SNAPSHOT_INCOMPLETE")
        );
    }

    #[test]
    fn host_id_fragments_are_reconstructed_across_chunks() {
        let main = json!({
            "host_id_raw": null,
            "host_id_byte_length": 4,
            "host_id_ref": "host-1",
            "host_id_fragment_count": 2
        });
        let first = json!({
            "record_kind": "host_id_fragment",
            "host_id_ref": "host-1",
            "host_id_byte_length": 4,
            "fragment_index": 0,
            "fragment_count": 2,
            "fragment": "あ"
        });
        let second = json!({
            "record_kind": "host_id_fragment",
            "host_id_ref": "host-1",
            "host_id_byte_length": 4,
            "fragment_index": 1,
            "fragment_count": 2,
            "fragment": "b"
        });
        let mut tracker = ProtocolTracker::default();
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-host", 0, 2, 3, vec![main, first]),
                )
                .is_empty()
        );
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("snap-host", 1, 2, 3, vec![second]),
                )
                .is_empty()
        );

        let mut incomplete = ProtocolTracker::default();
        let main = json!({
            "host_id_raw": null,
            "host_id_byte_length": 4,
            "host_id_ref": "host-2",
            "host_id_fragment_count": 2
        });
        let first = json!({
            "record_kind": "host_id_fragment",
            "host_id_ref": "host-2",
            "host_id_byte_length": 4,
            "fragment_index": 0,
            "fragment_count": 2,
            "fragment": "あ"
        });
        let faults = incomplete.observe_chunk_event(
            "source-a",
            "probe.bank.chunk",
            &chunk_data("snap-host-incomplete", 0, 1, 2, vec![main, first]),
        );
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "HOST_ID_FRAGMENT_STREAM_INCOMPLETE")
        );
    }

    #[test]
    fn checkpoint_context_marks_orphans_and_enforces_declared_window() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        let orphan = tracker.checkpoint_context(base);
        assert!(orphan.orphan);
        assert_eq!(tracker.orphan_messages, 1);

        let marker = tracker
            .begin_checkpoint("take-1".into(), Duration::from_secs(5), base)
            .unwrap();
        assert_eq!(marker["phase"], "begin");
        let queued_before_begin = tracker.checkpoint_context(base - Duration::from_millis(1));
        assert!(queued_before_begin.orphan);
        let context = tracker.checkpoint_context(base + Duration::from_secs(2));
        assert_eq!(context.checkpoint_id.as_deref(), Some("take-1"));
        assert_eq!(context.checkpoint_elapsed_ms, Some(2_000));
        assert_eq!(context.checkpoint_window_expired, Some(false));
        let (marker, fault) = tracker
            .end_checkpoint("take-1", base + Duration::from_secs(5))
            .unwrap();
        assert_eq!(marker["window_satisfied"], true);
        assert_eq!(marker["quiet_period_satisfied"], true);
        assert!(fault.is_none());

        tracker
            .begin_checkpoint("take-2".into(), Duration::from_secs(5), base)
            .unwrap();
        let (_, fault) = tracker
            .end_checkpoint("take-2", base + Duration::from_secs(4))
            .unwrap();
        assert_eq!(fault.unwrap().code, "CHECKPOINT_WINDOW_TOO_SHORT");

        let quiet_base = base + Duration::from_secs(10);
        tracker
            .begin_checkpoint("take-3".into(), Duration::from_secs(5), quiet_base)
            .unwrap();
        tracker.checkpoint_context(quiet_base + Duration::from_millis(4_500));
        let (marker, fault) = tracker
            .end_checkpoint("take-3", quiet_base + Duration::from_secs(5))
            .unwrap();
        assert_eq!(marker["quiet_period_satisfied"], false);
        assert_eq!(fault.unwrap().code, "CHECKPOINT_QUIET_PERIOD_NOT_SATISFIED");
    }

    #[test]
    fn checkpoint_end_barrier_includes_queued_callback_in_quiet_period() {
        let now = Instant::now();
        let base = now - Duration::from_secs(5);
        let callback_received_at = now - Duration::from_millis(500);
        let runtime = Arc::new(RuntimeTracker::new());
        runtime
            .state
            .lock()
            .unwrap()
            .begin_checkpoint("quiet-barrier".into(), Duration::from_secs(5), base)
            .unwrap();

        let progress = Arc::new(IngressProgress::default());
        let failed = Arc::new(AtomicBool::new(false));
        let activity = progress.begin_callback(&failed).unwrap();
        assert!(progress.reserve_received(&failed));
        let (started_sender, started_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let worker_failed = Arc::clone(&failed);
        let worker_runtime = Arc::clone(&runtime);
        let worker = thread::spawn(move || {
            started_sender.send(()).unwrap();
            let barrier = worker_progress.synchronize_held(&worker_failed).unwrap();
            let result = worker_runtime
                .state
                .lock()
                .unwrap()
                .end_checkpoint("quiet-barrier", barrier.boundary_time())
                .unwrap();
            drop(barrier);
            done_sender.send(result).unwrap();
        });
        started_receiver.recv().unwrap();
        assert!(done_receiver.try_recv().is_err());

        drop(activity);
        runtime
            .state
            .lock()
            .unwrap()
            .checkpoint_context(callback_received_at);
        progress.mark_processed(&failed);
        let (marker, fault) = done_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(marker["quiet_period_satisfied"], false);
        assert_eq!(fault.unwrap().code, "CHECKPOINT_QUIET_PERIOD_NOT_SATISFIED");
        worker.join().unwrap();
        assert!(!failed.load(Ordering::Acquire));
    }

    #[test]
    fn graceful_drain_times_out_unanswered_targeted_request() {
        let runtime = RuntimeTracker::new();
        let request = request(
            "request-1",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        {
            let base = Instant::now();
            let mut tracker = runtime.state.lock().unwrap();
            make_source_ready(&mut tracker, "source-a");
            select_source(&mut tracker, "source-a", base);
            tracker
                .register_request(&request, base, Duration::from_secs(1))
                .unwrap();
        }
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = JsonlSink {
            writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
            failed: AtomicBool::new(false),
            run_id: "run-1".into(),
            started_at: Instant::now(),
            record_prepare_hook: None,
        };
        let failed = AtomicBool::new(false);
        let report = graceful_drain(&runtime, Duration::from_millis(1), &sink, &failed);
        assert!(report.timed_out);
        assert!(!report.completed);
        assert!(failed.load(Ordering::Acquire));
        let records = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(records.contains("GRACEFUL_DRAIN_TIMEOUT"));
    }

    #[test]
    fn lifecycle_requires_loaded_mapping_and_ready_in_order() {
        let mut tracker = ProtocolTracker::default();
        let invalid = lifecycle_event("probe.capabilities", json!({}));
        assert_eq!(
            tracker.observe_source_message("bad", &invalid, true)[0].code,
            "SOURCE_FIRST_MESSAGE_INVALID"
        );

        let mut tracker = ProtocolTracker::default();
        assert!(
            tracker
                .observe_source_message("source-a", &loaded_message("source-a"), true)
                .is_empty()
        );
        assert_eq!(
            tracker.observe_source_message("source-a", &invalid, false)[0].code,
            "SOURCE_MAPPING_NOT_ACTIVE"
        );
        assert!(
            tracker
                .observe_source_message("source-a", &mapping_active_message("source-a"), false,)
                .is_empty()
        );
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", true), false)
                .is_empty()
        );
        assert_eq!(
            tracker.observe_source_message("source-a", &loaded_message("source-a"), false)[0].code,
            "PROBE_LOADED_REAPPEARED"
        );
    }

    #[test]
    fn lifecycle_deactivation_reactivation_clears_selection_and_rejects_old_delay() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", false), false)
                .is_empty()
        );
        assert!(tracker.selected_source_instance_id.is_none());
        assert_eq!(
            tracker.observe_source_message(
                "source-a",
                &lifecycle_event("probe.capabilities", json!({})),
                false,
            )[0]
            .code,
            "INACTIVE_SOURCE_MESSAGE"
        );
        assert!(
            tracker
                .observe_source_message("source-a", &mapping_active_message("source-a"), false,)
                .is_empty()
        );
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", true), false)
                .is_empty()
        );
        let targeted = request(
            "after-reactivation",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        assert_eq!(
            tracker
                .register_request(&targeted, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "DISCOVERY_REQUIRED"
        );
    }

    #[test]
    fn delayed_reactivation_cannot_preserve_another_selected_source() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", false), false)
                .is_empty()
        );
        make_source_ready(&mut tracker, "source-b");
        select_source(&mut tracker, "source-b", base);
        let faults =
            tracker.observe_source_message("source-a", &mapping_active_message("source-a"), false);
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "MULTIPLE_ACTIVE_PROBE_SOURCES")
        );
        assert!(tracker.selected_source_instance_id.is_none());
        let targeted = request(
            "unsafe-b",
            Some("source-b"),
            "probe.capabilities.get",
            json!({}),
        );
        assert_eq!(
            tracker
                .register_request(&targeted, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "DISCOVERY_REQUIRED"
        );
    }

    #[test]
    fn reactivation_rejects_another_source_awaiting_mapping() {
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        assert!(
            tracker
                .observe_source_message("source-a", &ready_message("source-a", false), false)
                .is_empty()
        );
        assert!(
            tracker
                .observe_source_message("source-b", &loaded_message("source-b"), true)
                .is_empty()
        );
        let faults =
            tracker.observe_source_message("source-a", &mapping_active_message("source-a"), false);
        assert!(
            faults
                .iter()
                .any(|fault| fault.code == "MULTIPLE_ACTIVE_PROBE_SOURCES")
        );
        assert!(tracker.selected_source_instance_id.is_none());
    }

    #[test]
    fn discovery_requires_active_checkpoint_ready_source_and_valid_semantics() {
        let base = Instant::now();
        let discover = request("discover", None, "probe.discover", json!({}));
        let mut no_checkpoint = ProtocolTracker::default();
        assert_eq!(
            no_checkpoint
                .register_request(&discover, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "PROBE_COMMAND_REQUIRES_CHECKPOINT"
        );

        let mut initializing = ProtocolTracker::default();
        initializing
            .begin_checkpoint("init".into(), Duration::from_secs(5), base)
            .unwrap();
        assert!(
            initializing
                .observe_source_message("source-a", &loaded_message("source-a"), true)
                .is_empty()
        );
        assert!(
            initializing
                .observe_source_message("source-a", &mapping_active_message("source-a"), false,)
                .is_empty()
        );
        initializing
            .register_request(&discover, base, Duration::from_secs(1))
            .unwrap();
        initializing.mark_request_sent("discover", base, Duration::from_secs(1));
        let invalid_response = json!({
            "result": {"instance_id": "source-a", "ready": false, "read_only": true}
        });
        assert_eq!(
            initializing.observe_reply_message(
                "source-a",
                "discover",
                MessageKind::Response,
                &invalid_response,
                Some("init"),
            )[0]
            .code,
            "DISCOVERY_RESPONSE_INVALID"
        );
        let expiry = initializing.expire_discoveries(base + Duration::from_secs(1));
        assert!(initializing.selected_source_instance_id.is_none());
        assert_eq!(expiry.records[0]["outcome"], "invalid_response");
    }

    #[test]
    fn targeted_commands_are_discovery_gated_and_followups_are_sequential() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        begin_test_checkpoint(&mut tracker, base);
        let snapshot = request(
            "snapshot",
            Some("source-a"),
            "probe.bank.snapshot",
            json!({"config_id": "MB_CORE_ALL"}),
        );
        assert_eq!(
            tracker
                .register_request(&snapshot, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "DISCOVERY_REQUIRED"
        );
        select_source(&mut tracker, "source-a", base);
        tracker
            .register_request(&snapshot, base, Duration::from_secs(1))
            .unwrap();
        let second = request(
            "second",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        assert_eq!(
            tracker
                .register_request(&second, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "PROBE_COMMAND_NOT_SEQUENTIAL"
        );
        assert!(
            tracker
                .observe_reply("source-a", "snapshot", MessageKind::Response)
                .is_empty()
        );
        assert_eq!(
            tracker
                .register_request(&second, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "PROBE_COMMAND_NOT_SEQUENTIAL"
        );
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("followup", 0, 1, 1, vec![json!({"track": "A"})]),
                )
                .is_empty()
        );
        tracker
            .register_request(&second, base, Duration::from_secs(1))
            .unwrap();
    }

    #[test]
    fn targeted_commands_wait_for_unsolicited_open_snapshots() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        assert!(
            tracker
                .observe_chunk_event(
                    "source-a",
                    "probe.bank.chunk",
                    &chunk_data("still-open", 0, 2, 2, vec![json!({"track": "A"})]),
                )
                .is_empty()
        );
        let targeted = request(
            "unsafe-during-snapshot",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        assert_eq!(
            tracker
                .register_request(&targeted, base, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "PROBE_COMMAND_NOT_SEQUENTIAL"
        );
    }

    #[test]
    fn checkpoint_end_waits_for_protocol_and_receive_time_history_closes_race() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        let request = request(
            "pending",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        tracker
            .register_request(&request, base, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            tracker
                .end_checkpoint("test-checkpoint", base + Duration::from_secs(5))
                .unwrap_err()
                .code,
            "CHECKPOINT_PROTOCOL_NOT_QUIESCENT"
        );
        assert!(tracker.active_checkpoint.is_some());
        assert!(
            tracker
                .observe_reply("source-a", "pending", MessageKind::Response)
                .is_empty()
        );
        tracker
            .end_checkpoint("test-checkpoint", base + Duration::from_secs(5))
            .unwrap();
        let queued = tracker.checkpoint_context(base + Duration::from_secs(4));
        assert_eq!(queued.checkpoint_id.as_deref(), Some("test-checkpoint"));
        assert!(!queued.orphan);
        assert!(queued.processed_after_checkpoint_end);
        assert!(!queued.checkpoint_quiet_period_violated);
        let quiet_violation =
            tracker.checkpoint_context(base + Duration::from_secs(4) + Duration::from_millis(1));
        assert!(quiet_violation.checkpoint_quiet_period_violated);
        assert!(
            tracker
                .checkpoint_context(base + Duration::from_secs(6))
                .orphan
        );
    }

    #[test]
    fn reply_and_followup_must_match_request_checkpoint() {
        let base = Instant::now();
        let mut tracker = ProtocolTracker::default();
        make_source_ready(&mut tracker, "source-a");
        select_source(&mut tracker, "source-a", base);
        let cross_request = request(
            "cross-response",
            Some("source-a"),
            "probe.capabilities.get",
            json!({}),
        );
        tracker
            .register_request(&cross_request, base, Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            tracker.observe_reply_message(
                "source-a",
                "cross-response",
                MessageKind::Response,
                &json!({"result": {}}),
                Some("other"),
            )[0]
            .code,
            "RESPONSE_CHECKPOINT_MISMATCH"
        );
        tracker.cancel_request("cross-response");

        let snapshot = request(
            "cross-followup",
            Some("source-a"),
            "probe.bank.snapshot",
            json!({"config_id": "MB_CORE_ALL"}),
        );
        tracker
            .register_request(&snapshot, base, Duration::from_secs(1))
            .unwrap();
        assert!(
            tracker
                .observe_reply("source-a", "cross-followup", MessageKind::Response)
                .is_empty()
        );
        let faults = tracker.observe_chunk_event_at(
            "source-a",
            "probe.bank.chunk",
            &chunk_data("cross", 0, 1, 1, vec![json!({"track": "A"})]),
            Some("other"),
        );
        assert_eq!(faults[0].code, "FOLLOWUP_CHECKPOINT_MISMATCH");
        assert_eq!(tracker.expected_followups.len(), 1);
    }

    #[test]
    fn chunk_metadata_config_and_host_reference_bounds_are_fail_closed() {
        let mut tracker = ProtocolTracker::default();
        let mut first = chunk_data("drift", 0, 2, 2, vec![json!({"track": "A"})]);
        first["bank_generation"] = json!(1);
        assert!(
            tracker
                .observe_chunk_event("source-a", "probe.bank.chunk", &first)
                .is_empty()
        );
        let mut second = chunk_data("drift", 1, 2, 2, vec![json!({"track": "B"})]);
        second["bank_generation"] = json!(2);
        assert_eq!(
            tracker.observe_chunk_event("source-a", "probe.bank.chunk", &second)[0].code,
            "SNAPSHOT_METADATA_CHANGED"
        );

        let mut mismatch = chunk_data("config", 0, 1, 1, vec![json!({"track": "A"})]);
        mismatch["items"][0]["config_id"] = json!("OTHER");
        assert_eq!(
            ProtocolTracker::default().observe_chunk_event(
                "source-a",
                "probe.bank.chunk",
                &mismatch,
            )[0]
            .code,
            "BANK_ITEM_CONFIG_ID_MISMATCH"
        );

        let long_reference = "x".repeat(MAX_REQUEST_ID_BYTES + 1);
        let data = chunk_data(
            "host-ref",
            0,
            1,
            1,
            vec![json!({
                "host_id_raw": null,
                "host_id_byte_length": 1,
                "host_id_ref": long_reference,
                "host_id_fragment_count": 1
            })],
        );
        let mut bounded = ProtocolTracker::default();
        assert_eq!(
            bounded.observe_chunk_event("source-a", "probe.bank.chunk", &data)[0].code,
            "HOST_ID_REF_INVALID"
        );
        assert!(bounded.open_snapshots.is_empty());
    }

    #[test]
    fn orphan_latches_before_probe_record_and_drain_waits_full_window() {
        let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
        let frame = encode_sysex(&incoming("source-a", 1, loaded_message("source-a"))).unwrap();
        sender
            .send(Ingress::Frame {
                received_at_unix_ms: unix_timestamp_ms(),
                received_at_monotonic: Instant::now(),
                midi_timestamp: 0,
                bytes: frame,
            })
            .unwrap();
        drop(sender);
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::new(JsonlSink {
            writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
            failed: AtomicBool::new(false),
            run_id: "run".into(),
            started_at: Instant::now(),
            record_prepare_hook: None,
        });
        let failed = Arc::new(AtomicBool::new(false));
        let ingress_progress = Arc::new(IngressProgress::default());
        assert!(ingress_progress.reserve_received(&failed));
        let report = collect_incoming(
            receiver,
            Arc::new(AtomicU64::new(0)),
            Arc::clone(&failed),
            Arc::clone(&sink),
            Arc::new(RuntimeTracker::new()),
            Arc::clone(&ingress_progress),
        );
        assert!(failed.load(Ordering::Acquire));
        assert!(report.diagnostics > 0);
        let records = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(records.contains("ORPHAN_PROBE_MESSAGE"));
        assert!(records.contains("\"integrity_ok_at_emit\":false"));

        let runtime = RuntimeTracker::new();
        let failed = AtomicBool::new(false);
        let started = Instant::now();
        let report = graceful_drain(&runtime, Duration::from_millis(20), &sink, &failed);
        assert!(report.completed);
        assert!(!report.timed_out);
        assert!(started.elapsed() >= Duration::from_millis(15));
    }

    #[test]
    fn successful_cut_response_atomically_emits_and_marks_the_action_boundary() {
        let base = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(MIDI_QUEUE_CAPACITY);
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::new(JsonlSink {
            writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
            failed: AtomicBool::new(false),
            run_id: "run".into(),
            started_at: base,
            record_prepare_hook: None,
        });
        let failed = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let ingress_progress = Arc::new(IngressProgress::default());
        let runtime = Arc::new(RuntimeTracker::new());
        runtime
            .state
            .lock()
            .unwrap()
            .begin_checkpoint("test-checkpoint".into(), Duration::from_secs(5), base)
            .unwrap();

        let collector_sink = Arc::clone(&sink);
        let collector_failed = Arc::clone(&failed);
        let collector_dropped = Arc::clone(&dropped);
        let collector_progress = Arc::clone(&ingress_progress);
        let collector_runtime = Arc::clone(&runtime);
        let collector = thread::spawn(move || {
            collect_incoming(
                receiver,
                collector_dropped,
                collector_failed,
                collector_sink,
                collector_runtime,
                collector_progress,
            )
        });

        for (sequence, message) in [
            (1, loaded_message("source-a")),
            (2, mapping_active_message("source-a")),
            (3, ready_message("source-a", true)),
        ] {
            enqueue_test_incoming(
                &sender,
                &dropped,
                &failed,
                &ingress_progress,
                incoming("source-a", sequence, message),
                Instant::now(),
            );
        }
        ingress_progress
            .synchronize_until(&failed, Instant::now() + Duration::from_secs(1))
            .unwrap();

        {
            let mut tracker = runtime.state.lock().unwrap();
            select_source(&mut tracker, "source-a", base);
            let cut = request(
                "cut-auto",
                Some("source-a"),
                "probe.observation.cut",
                json!({}),
            );
            tracker
                .register_request(&cut, Instant::now(), Duration::from_secs(1))
                .unwrap();
            tracker.mark_request_sent("cut-auto", Instant::now(), Duration::from_secs(1));
        }

        enqueue_test_incoming(
            &sender,
            &dropped,
            &failed,
            &ingress_progress,
            incoming(
                "source-a",
                4,
                json!({
                    "version": 1,
                    "id": "cut-auto",
                    "type": "response",
                    "result": {"observation_epoch": 1}
                }),
            ),
            Instant::now(),
        );
        drop(sender);

        let report = collector.join().unwrap();
        assert_eq!(report.diagnostics, 0);
        assert!(!failed.load(Ordering::Acquire));
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let records: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let response_index = records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_response" && record["message"]["id"] == "cut-auto"
            })
            .unwrap();
        let action = &records[response_index + 1];
        assert_eq!(action["record_type"], "collector_action");
        assert_eq!(action["checkpoint_id"], "test-checkpoint");
        assert_eq!(action["boundary_source"], "probe.observation.cut_response");
        assert_eq!(action["request_id"], "cut-auto");
        assert_eq!(action["observation_epoch"], 1);

        let mut tracker = runtime.state.lock().unwrap();
        assert!(
            tracker
                .active_checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.action_marked)
        );
        assert_eq!(
            tracker.mark_action("test-checkpoint").unwrap_err().code,
            "ACTION_ALREADY_MARKED"
        );
    }

    #[test]
    fn jsonl_sink_adds_run_and_single_clock_metadata_to_every_record() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = JsonlSink {
            writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
            failed: AtomicBool::new(false),
            run_id: "evidence-run".into(),
            started_at: Instant::now(),
            record_prepare_hook: None,
        };
        sink.emit(&json!({"record_type": "test_one"})).unwrap();
        sink.emit_pair(
            &json!({"record_type": "test_two"}),
            &json!({"record_type": "test_three"}),
        )
        .unwrap();
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let records: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["record_type"], "test_one");
        assert_eq!(records[1]["record_type"], "test_two");
        assert_eq!(records[2]["record_type"], "test_three");
        for record in &records {
            assert_eq!(record["record_format_version"], RECORD_FORMAT_VERSION);
            assert_eq!(record["run_id"], "evidence-run");
            assert!(record["monotonic_timestamp_ms"].is_u64());
            assert!(record["timestamp_unix_ms"].is_u64());
        }
        assert!(
            records[1]["monotonic_timestamp_ms"].as_u64()
                >= records[0]["monotonic_timestamp_ms"].as_u64()
        );
    }

    #[test]
    fn concurrent_jsonl_emission_assigns_timestamps_under_the_writer_lock() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let sink: Arc<JsonlSink> = Arc::new_cyclic(|weak: &Weak<JsonlSink>| {
            let weak = weak.clone();
            let prepare_calls = Arc::clone(&prepare_calls);
            JsonlSink {
                writer: Mutex::new(Box::new(SharedBuffer(Arc::clone(&output)))),
                failed: AtomicBool::new(false),
                run_id: "concurrent-evidence".into(),
                started_at: Instant::now(),
                record_prepare_hook: Some(Arc::new(move || {
                    prepare_calls.fetch_add(1, Ordering::SeqCst);
                    let sink = weak.upgrade().expect("test sink remains alive");
                    assert!(matches!(
                        sink.writer.try_lock(),
                        Err(TryLockError::WouldBlock)
                    ));
                })),
            }
        });
        let start = Arc::new(Barrier::new(3));
        let workers: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|record_type| {
                let sink = Arc::clone(&sink);
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    sink.emit(&json!({"record_type": record_type}))
                })
            })
            .collect();
        start.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        let records: Vec<Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert!(
            records[1]["monotonic_timestamp_ms"].as_u64()
                >= records[0]["monotonic_timestamp_ms"].as_u64()
        );
    }

    #[test]
    fn help_documents_ports_jsonl_and_eof_summary() {
        let help = Config::help();
        assert!(help.contains(TO_CUBASE_PORT));
        assert!(help.contains(FROM_CUBASE_PORT));
        assert!(help.contains("JSON Lines"));
        assert!(help.contains("Ctrl-D"));
        assert!(help.contains("--run-id"));
        assert!(help.contains("collector.checkpoint.begin"));
        assert!(help.contains("collector.action"));
        assert!(help.contains(SELECTED_TARGET_ALIAS));
        assert!(help.contains("after the atomic send attempt"));
        assert!(help.contains("`sent` remains authoritative"));
        assert!(help.contains("F0 7E 7F 06 01 F7"));
        assert!(help.contains("every other foreign SysEx remains a fatal integrity error"));
    }

    #[test]
    fn bounded_line_reader_discards_the_rest_of_an_oversize_command() {
        let oversized = format!("{}\n{{\"method\":\"probe.discover\"}}\n", "x".repeat(20));
        let mut reader = Cursor::new(oversized.into_bytes());
        let first = read_bounded_line(&mut reader, 8).unwrap().unwrap();
        assert_eq!(first.len(), 9);
        let second = read_bounded_line(&mut reader, 64).unwrap().unwrap();
        assert_eq!(second, br#"{"method":"probe.discover"}"#);
    }
}
