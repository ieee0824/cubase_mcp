#![recursion_limit = "256"]

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

const AUDIT_MANIFEST_VERSION: u32 = 1;
const AUDIT_REPORT_VERSION: u32 = 2;
const RECORD_FORMAT_VERSION: u64 = 1;
const FIXTURE_REVISION: u32 = 2;
const CALLBACK_WINDOW_MS: u64 = 5_000;
const RECONNECT_DEADLINE_MS: u64 = 30_000;
const RECONNECT_FINALIZATION_MS: u64 = 10_000;
const ACTION_COMMAND_DEADLINE_MS: u64 = 1_000;
const MIN_QUIET_PERIOD_MS: u64 = 1_000;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_LOG_BYTES: usize = 64 * 1024 * 1024;
const MAX_LOG_LINE_BYTES: usize = 1024 * 1024;
const MAX_LOG_RECORDS: usize = 200_000;
const MAX_SAFE_TEXT_BYTES: usize = 96;
const MAX_SEMANTIC_ITEMS: usize = 100_000;
const MAX_HOST_ID_BYTES: usize = 4_096;
const MAX_HOST_ID_FRAGMENTS: usize = 16;
const MAX_DIRECT_ACCESS_OBJECT_ID: u64 = 9_007_199_254_740_991;
const P05_TITLE_NFC: &str = "CMCP_05_日本語_é_🎹";
const P05_TITLE_NFD: &str = "CMCP_05_日本語_e\u{301}_🎹";
const P09_TITLE_PREFIX: &str = "CMCP_09_LONG_";
const P09_TITLE_FULL: &str =
    "CMCP_09_LONG_ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNO";

const REQUIRED_CHECKPOINTS: [&str; 44] = [
    "INIT",
    "E0",
    "E1",
    "E8",
    "E8-MB_CORE_ALL-B0-reset",
    "E8-MB_CORE_ALL-B1-next",
    "E8-MB_CORE_ALL-B2-prev",
    "E8-MB_CORE_ALL-B3-reset",
    "E8-MB_CORE_VISIBLE-B0-reset",
    "E8-MB_CORE_VISIBLE-B1-next",
    "E8-MB_CORE_VISIBLE-B2-prev",
    "E8-MB_CORE_VISIBLE-B3-reset",
    "C1",
    "C1-MB_CORE_ALL-B0-reset",
    "C1-MB_CORE_ALL-B1-next",
    "C1-MB_CORE_ALL-B2-next",
    "C1-MB_CORE_ALL-B3-extra-next",
    "C1-MB_CORE_ALL-B4-prev",
    "C1-MB_CORE_ALL-B5-reset",
    "C1-MB_CORE_VISIBLE-B0-reset",
    "C1-MB_CORE_VISIBLE-B1-next",
    "C1-MB_CORE_VISIBLE-B2-next",
    "C1-MB_CORE_VISIBLE-B3-extra-next",
    "C1-MB_CORE_VISIBLE-B4-prev",
    "C1-MB_CORE_VISIBLE-B5-reset",
    "S0",
    "S1-select",
    "S1-rename",
    "S2-select-anchor",
    "S2-add",
    "S3-select-delete",
    "S3-delete",
    "S4-show",
    "S5-select-anchor",
    "S5-select-change",
    "S6-mute",
    "S7-solo",
    "S8-project-only-hide",
    "S8-restore",
    "S9-empty",
    "S9-mutation",
    "S9-baseline",
    "R1",
    "R2",
];

fn main() -> ExitCode {
    match run_cli(env::args().skip(1)) {
        Ok(CliOutcome::Printed) => ExitCode::SUCCESS,
        Ok(CliOutcome::Report(report)) => match serde_json::to_string(&report) {
            Ok(encoded) => {
                println!("{encoded}");
                ExitCode::SUCCESS
            }
            Err(_) => {
                print_failure("REPORT_ENCODING_FAILED");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            print_failure(error.code);
            ExitCode::FAILURE
        }
    }
}

fn print_failure(code: &'static str) {
    let report = json!({
        "audit_report_version": AUDIT_REPORT_VERSION,
        "status": "failed",
        "error_code": code
    });
    eprintln!("{report}");
}

enum CliOutcome {
    Printed,
    Report(Box<AuditReport>),
}

fn run_cli(arguments: impl Iterator<Item = String>) -> Result<CliOutcome, AuditError> {
    let mut manifest_path = None;
    let mut log_path = None;
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--manifest" => {
                manifest_path = Some(
                    arguments
                        .next()
                        .ok_or_else(|| AuditError::new("CLI_ARGUMENT_MISSING"))?,
                );
            }
            "--jsonl" => {
                log_path = Some(
                    arguments
                        .next()
                        .ok_or_else(|| AuditError::new("CLI_ARGUMENT_MISSING"))?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cubase_track_probe_audit --manifest <FILE> --jsonl <FILE>\n\
                     \nThe report never includes raw source IDs, MIDI ports, paths, or request values."
                );
                return Ok(CliOutcome::Printed);
            }
            "--version" | "-V" => {
                println!("cubase_track_probe_audit {}", env!("CARGO_PKG_VERSION"));
                return Ok(CliOutcome::Printed);
            }
            _ => return Err(AuditError::new("CLI_ARGUMENT_INVALID")),
        }
    }
    let manifest_path = manifest_path.ok_or_else(|| AuditError::new("CLI_ARGUMENT_MISSING"))?;
    let log_path = log_path.ok_or_else(|| AuditError::new("CLI_ARGUMENT_MISSING"))?;
    let manifest_bytes = read_bounded_file(Path::new(&manifest_path), MAX_MANIFEST_BYTES)
        .map_err(|_| AuditError::new("MANIFEST_READ_FAILED"))?;
    let log_bytes = read_bounded_file(Path::new(&log_path), MAX_LOG_BYTES)
        .map_err(|_| AuditError::new("LOG_READ_FAILED"))?;
    Ok(CliOutcome::Report(Box::new(audit_bytes(
        &manifest_bytes,
        &log_bytes,
    )?)))
}

fn read_bounded_file(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take((maximum as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bounded input exceeded",
        ));
    }
    Ok(bytes)
}

#[derive(Debug)]
struct AuditError {
    code: &'static str,
}

impl AuditError {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

type AuditResult<T> = Result<T, AuditError>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Profile {
    C13MixerBank,
    C15Combined,
}

impl Profile {
    const fn expected_host(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::C13MixerBank => ("Cubase Pro", "13.0.30.226", "1.1"),
            Self::C15Combined => ("Cubase Pro", "15.0.30.287", "1.3"),
        }
    }

    const fn requires_direct_access(self) -> bool {
        matches!(self, Self::C15Combined)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditManifest {
    audit_manifest_version: u32,
    fixture_revision: u32,
    profile: Profile,
    run_id: String,
    run_started_at: String,
    environment: Environment,
    mixconsole: MixConsoleEvidence,
    filters: FilterEvidence,
    fixture_acceptance: FixtureAcceptance,
    callback_window_ms: u64,
    reconnect_deadline_ms: u64,
    optional_o1: OptionalO1,
    annotations: Vec<UiAnnotation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    host: HostEnvironment,
    os: OsEnvironment,
    repository_commit: String,
    probe_source_sha256: String,
    installer_embedded_sha256: String,
    collector_binary_sha256: String,
    deployed_probe_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostEnvironment {
    product: String,
    version: String,
    api_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OsEnvironment {
    name: String,
    version: String,
    build: String,
    architecture: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MixConsoleSurface {
    Separate,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum VisibilitySync {
    On,
    Off,
    NotOpen,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MixConsoleEvidence {
    surface: MixConsoleSurface,
    visibility_sync_initial: VisibilitySync,
    visibility_sync_during_baseline: VisibilitySync,
    visibility_sync_restored: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ZoneFilterEvidence {
    Excluded,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MainZoneFilterEvidence {
    Implicit,
    Explicit,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum IncludedChannelType {
    Audio,
    Instrument,
    Midi,
    Group,
    Fx,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ExcludedChannelType {
    Sampler,
    Vca,
    Input,
    Output,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FilterEvidence {
    bank_width: u32,
    core_all_follow_visibility: bool,
    core_visible_follow_visibility: bool,
    included_channel_types: Vec<IncludedChannelType>,
    excluded_channel_types: Vec<ExcludedChannelType>,
    left_zone: ZoneFilterEvidence,
    right_zone: ZoneFilterEvidence,
    main_filter: MainZoneFilterEvidence,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum AlternatePluginEntry {
    None,
    Used { accepted_name: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlternatePlugins {
    instrument: AlternatePluginEntry,
    effect: AlternatePluginEntry,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum P05TitlePolicy {
    NfcOrNfdExact,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum P09TitlePolicy {
    FixedNamePrefix,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedFixtureTitle<P> {
    policy: P,
    accepted_title: String,
    unicode_scalar_count: usize,
    utf8_byte_length: usize,
    setup_variance: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureAcceptance {
    alternate_plugins: AlternatePlugins,
    p05_title: AcceptedFixtureTitle<P05TitlePolicy>,
    p09_title: AcceptedFixtureTitle<P09TitlePolicy>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OptionalO1Status {
    Skipped,
    Observed,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OptionalO1Reason {
    NotSeparatelyAuthorized,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionalO1 {
    status: OptionalO1Status,
    reason: OptionalO1Reason,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AnnotationResult {
    Observed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiAnnotation {
    checkpoint_id: String,
    result: AnnotationResult,
    ui_ground_truth_confirmed: bool,
    action_confirmed: bool,
}

#[derive(Debug, Serialize)]
struct AuditReport {
    audit_report_version: u32,
    status: &'static str,
    semantic_assessment: &'static str,
    profile: Profile,
    fixture_revision: u32,
    run_alias: String,
    run_started_at: String,
    environment: Environment,
    mixconsole: MixConsoleEvidence,
    filters: FilterEvidence,
    fixture_acceptance: FixtureAcceptanceReport,
    capabilities: CapabilityReport,
    evidence_sha256: EvidenceDigests,
    artifact_digests_match: bool,
    record_count: usize,
    checkpoint_count: usize,
    probe_command_count: usize,
    completed_snapshot_count: usize,
    probe_source_count: usize,
    reconnects: Vec<ReconnectReport>,
    projections: Vec<SemanticProjection>,
}

#[derive(Debug, Serialize)]
struct FixtureAcceptanceReport {
    alternate_instrument_plugin_used: bool,
    alternate_effect_plugin_used: bool,
    p05_accepted_title: String,
    p05_setup_variance: bool,
    p09_accepted_title: String,
    p09_setup_variance: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct CapabilityReport {
    read_only: bool,
    integrity_failed: bool,
    data_minimization: DataMinimizationCapabilityReport,
    observation_epoch: ObservationEpochCapabilityReport,
    mixer_bank: MixerBankCapabilityReport,
    direct_access: DirectAccessCapabilityReport,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ObservationEpochCapabilityReport {
    supported: bool,
    version: u64,
    maximum: u64,
    rollover_policy: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct DataMinimizationCapabilityReport {
    source_redaction: bool,
    fixture_revision: u64,
    unknown_titles: &'static str,
    unknown_host_ids: &'static str,
    unique_name_policy: &'static str,
    exception_text: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct MixerBankCapabilityReport {
    supported: bool,
    slot_count: u64,
    core_all: bool,
    core_visible: bool,
    title: bool,
    selected: bool,
    mute: bool,
    solo: bool,
    unique_id: bool,
    explicit_main_filter: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct DirectAccessCapabilityReport {
    supported: bool,
    active: bool,
    unique_name: bool,
    unique_name_policy: &'static str,
    unique_id: bool,
    title: bool,
    type_name: bool,
    mixer_visibility: bool,
    mixer_index: bool,
    mixer_zone: bool,
}

#[derive(Debug, Serialize)]
struct ReconnectReport {
    phase: &'static str,
    ready_elapsed_ms: u64,
    discovery_elapsed_ms: u64,
    final_snapshot_elapsed_ms: u64,
}

#[derive(Debug, Serialize)]
struct EvidenceDigests {
    manifest: String,
    raw_jsonl: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "projection", rename_all = "snake_case")]
enum SemanticProjection {
    MixerBank {
        checkpoint_id: &'static str,
        config_id: String,
        total_wire_items: usize,
        missing_slot_count: usize,
        duplicate_slot_count: usize,
        unknown_item_count: usize,
        missing_title_count: usize,
        unknown_title_count: usize,
        redacted_title_count: usize,
        redacted_host_id_count: usize,
        source_redacted_string_count: usize,
        stale_or_unobserved_field_count: usize,
        duplicate_host_id_count: usize,
        p05_title: FixtureTitleComparison,
        p09_title: FixtureTitleComparison,
        slots: Vec<BankSlotProjection>,
    },
    DirectAccess {
        checkpoint_id: &'static str,
        total_wire_items: usize,
        observation_count: usize,
        reference_count: usize,
        cycle_reference_count: usize,
        shared_reference_count: usize,
        missing_count: usize,
        duplicate_count: usize,
        unknown_count: usize,
        redacted_unique_name_count: usize,
        redacted_title_count: usize,
        redacted_type_name_count: usize,
        redacted_host_id_count: usize,
        source_redacted_string_count: usize,
        duplicate_host_id_count: usize,
        p05_title: FixtureTitleComparison,
        p09_title: FixtureTitleComparison,
        nodes: Vec<DirectAccessProjection>,
        references: Vec<DirectAccessReferenceProjection>,
    },
}

#[derive(Debug, Serialize)]
struct FixtureTitleComparison {
    accepted_ui_title: String,
    exact_match_count: usize,
    safe_known_variant_count: usize,
    missing_or_redacted_count: usize,
    target_not_observed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SafeTitleCategory {
    Fixture,
    Empty,
    Unavailable,
    Redacted,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AliasStatus {
    Aliased,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FieldFreshness {
    Fresh,
    StaleOrUnobserved,
    Redacted,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SafeTypeCategory {
    Allowlisted,
    Unavailable,
    Redacted,
}

#[derive(Debug, Serialize)]
struct BankSlotProjection {
    slot_index: u64,
    title: Option<String>,
    title_category: SafeTitleCategory,
    title_freshness: FieldFreshness,
    selected: Option<bool>,
    selected_freshness: FieldFreshness,
    mute: Option<bool>,
    mute_freshness: FieldFreshness,
    solo: Option<bool>,
    solo_freshness: FieldFreshness,
    title_redacted: bool,
    host_id_redacted: bool,
    redacted_string_count: usize,
    host_id_alias: Option<String>,
    host_id_status: AliasStatus,
    host_id_byte_length: Option<usize>,
}

#[derive(Debug, Serialize)]
struct DirectAccessProjection {
    traversal_index: usize,
    depth: Option<u64>,
    child_index: Option<u64>,
    object_alias: Option<String>,
    parent_alias: Option<String>,
    title: Option<String>,
    title_category: SafeTitleCategory,
    unique_name_redacted: bool,
    title_redacted: bool,
    type_name: Option<String>,
    type_name_category: SafeTypeCategory,
    type_name_redacted: bool,
    host_id_redacted: bool,
    host_id_alias: Option<String>,
    host_id_status: AliasStatus,
    host_id_byte_length: Option<usize>,
    mixer_visible: Option<bool>,
    mixer_index: Option<f64>,
    mixer_zone: Option<f64>,
    child_count: Option<u64>,
    metadata_error_count: Option<u64>,
    redacted_string_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DirectAccessReferenceKind {
    AncestorCycle,
    SharedReference,
}

#[derive(Debug, Serialize)]
struct DirectAccessReferenceProjection {
    reference_index: usize,
    depth: u64,
    child_index: u64,
    target_observation_index: u64,
    parent_alias: String,
    target_alias: String,
    reference_kind: DirectAccessReferenceKind,
}

#[derive(Default)]
struct AliasState {
    host_ids: HashMap<String, String>,
    object_ids: HashMap<(String, u64), String>,
    next_host_id: usize,
    next_object_id: usize,
}

impl AliasState {
    fn host_alias(&mut self, raw: String) -> String {
        if let Some(alias) = self.host_ids.get(&raw) {
            return alias.clone();
        }
        self.next_host_id += 1;
        let alias = format!("H{:06}", self.next_host_id);
        self.host_ids.insert(raw, alias.clone());
        alias
    }

    fn object_alias(&mut self, source: &str, object_id: u64) -> String {
        let key = (source.to_owned(), object_id);
        if let Some(alias) = self.object_ids.get(&key) {
            return alias.clone();
        }
        self.next_object_id += 1;
        let alias = format!("O{:06}", self.next_object_id);
        self.object_ids.insert(key, alias.clone());
        alias
    }
}

fn audit_bytes(manifest_bytes: &[u8], log_bytes: &[u8]) -> AuditResult<AuditReport> {
    if manifest_bytes.is_empty() || manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(AuditError::new("MANIFEST_SIZE_INVALID"));
    }
    if log_bytes.is_empty() || log_bytes.len() > MAX_LOG_BYTES {
        return Err(AuditError::new("LOG_SIZE_INVALID"));
    }
    let manifest: AuditManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|_| AuditError::new("MANIFEST_JSON_INVALID"))?;
    validate_manifest(&manifest)?;
    let records = parse_jsonl(log_bytes)?;
    let evidence = collect_evidence(&manifest, &records)?;
    validate_evidence(
        &manifest,
        &records,
        &evidence,
        EvidenceDigests {
            manifest: sha256_hex(manifest_bytes),
            raw_jsonl: sha256_hex(log_bytes),
        },
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn validate_manifest(manifest: &AuditManifest) -> AuditResult<()> {
    if manifest.audit_manifest_version != AUDIT_MANIFEST_VERSION {
        return Err(AuditError::new("MANIFEST_VERSION_UNSUPPORTED"));
    }
    if manifest.fixture_revision != FIXTURE_REVISION {
        return Err(AuditError::new("FIXTURE_REVISION_INVALID"));
    }
    if !safe_run_id(&manifest.run_id) {
        return Err(AuditError::new("RUN_ID_INVALID"));
    }
    if !valid_rfc3339_with_timezone(&manifest.run_started_at) {
        return Err(AuditError::new("RUN_STARTED_AT_INVALID"));
    }
    if manifest.callback_window_ms != CALLBACK_WINDOW_MS {
        return Err(AuditError::new("CALLBACK_WINDOW_INVALID"));
    }
    if manifest.reconnect_deadline_ms != RECONNECT_DEADLINE_MS {
        return Err(AuditError::new("RECONNECT_DEADLINE_INVALID"));
    }

    let expected_host = manifest.profile.expected_host();
    if (
        manifest.environment.host.product.as_str(),
        manifest.environment.host.version.as_str(),
        manifest.environment.host.api_version.as_str(),
    ) != expected_host
    {
        return Err(AuditError::new("HOST_PROFILE_MISMATCH"));
    }
    for value in [
        &manifest.environment.os.name,
        &manifest.environment.os.version,
        &manifest.environment.os.build,
        &manifest.environment.os.architecture,
    ] {
        if !safe_metadata_text(value) {
            return Err(AuditError::new("OS_METADATA_INVALID"));
        }
    }
    let build_valid = match manifest.environment.os.name.as_str() {
        "macOS" => valid_macos_build(&manifest.environment.os.build),
        "Windows" => numeric_dot_version(&manifest.environment.os.build),
        _ => false,
    };
    if !matches!(manifest.environment.os.name.as_str(), "macOS" | "Windows")
        || !numeric_dot_version(&manifest.environment.os.version)
        || !build_valid
        || !matches!(
            manifest.environment.os.architecture.as_str(),
            "arm64" | "x86_64"
        )
    {
        return Err(AuditError::new("OS_METADATA_INVALID"));
    }
    let expected_main_zone = if manifest.profile.requires_direct_access() {
        MainZoneFilterEvidence::Explicit
    } else {
        MainZoneFilterEvidence::Implicit
    };
    if manifest.mixconsole.surface != MixConsoleSurface::Separate
        || manifest.mixconsole.visibility_sync_during_baseline != VisibilitySync::On
        || !manifest.mixconsole.visibility_sync_restored
        || manifest.filters.bank_width != 8
        || manifest.filters.left_zone != ZoneFilterEvidence::Excluded
        || manifest.filters.right_zone != ZoneFilterEvidence::Excluded
        || manifest.filters.core_all_follow_visibility
        || !manifest.filters.core_visible_follow_visibility
        || manifest.filters.main_filter != expected_main_zone
        || manifest.filters.included_channel_types
            != [
                IncludedChannelType::Audio,
                IncludedChannelType::Instrument,
                IncludedChannelType::Midi,
                IncludedChannelType::Group,
                IncludedChannelType::Fx,
            ]
        || manifest.filters.excluded_channel_types
            != [
                ExcludedChannelType::Sampler,
                ExcludedChannelType::Vca,
                ExcludedChannelType::Input,
                ExcludedChannelType::Output,
            ]
    {
        return Err(AuditError::new("SURFACE_OR_FILTER_EVIDENCE_INVALID"));
    }
    if !lower_hex(&manifest.environment.repository_commit, 40) {
        return Err(AuditError::new("COMMIT_HASH_INVALID"));
    }
    for hash in [
        &manifest.environment.probe_source_sha256,
        &manifest.environment.installer_embedded_sha256,
        &manifest.environment.collector_binary_sha256,
        &manifest.environment.deployed_probe_sha256,
    ] {
        if !lower_hex(hash, 64) {
            return Err(AuditError::new("ARTIFACT_HASH_INVALID"));
        }
    }
    if manifest.environment.probe_source_sha256 != manifest.environment.installer_embedded_sha256
        || manifest.environment.probe_source_sha256 != manifest.environment.deployed_probe_sha256
    {
        return Err(AuditError::new("PROBE_HASH_MISMATCH"));
    }
    validate_fixture_acceptance(&manifest.fixture_acceptance)?;

    match manifest.optional_o1.status {
        OptionalO1Status::Skipped => {
            if manifest.optional_o1.reason != OptionalO1Reason::NotSeparatelyAuthorized {
                return Err(AuditError::new("OPTIONAL_O1_REASON_INVALID"));
            }
        }
        OptionalO1Status::Observed => {
            // Revision 1 of the auditor deliberately has no inferred O1 coverage.
            return Err(AuditError::new("OPTIONAL_O1_NOT_AUDITED"));
        }
    }

    let required: HashSet<_> = REQUIRED_CHECKPOINTS.into_iter().collect();
    let mut observed = HashSet::new();
    for annotation in &manifest.annotations {
        let Some(required_id) = canonical_checkpoint_id(&annotation.checkpoint_id) else {
            return Err(AuditError::new("ANNOTATION_CHECKPOINT_UNEXPECTED"));
        };
        if !observed.insert(required_id) {
            return Err(AuditError::new("ANNOTATION_CHECKPOINT_DUPLICATE"));
        }
        if annotation.result != AnnotationResult::Observed
            || !annotation.ui_ground_truth_confirmed
            || !annotation.action_confirmed
        {
            return Err(AuditError::new("UI_GROUND_TRUTH_UNCONFIRMED"));
        }
    }
    if observed != required {
        return Err(AuditError::new("ANNOTATION_CHECKPOINT_MISSING"));
    }

    Ok(())
}

fn validate_fixture_acceptance(acceptance: &FixtureAcceptance) -> AuditResult<()> {
    for plugin in [
        &acceptance.alternate_plugins.instrument,
        &acceptance.alternate_plugins.effect,
    ] {
        if let AlternatePluginEntry::Used { accepted_name } = plugin
            && !safe_plugin_name(accepted_name)
        {
            return Err(AuditError::new("ALTERNATE_PLUGIN_METADATA_INVALID"));
        }
    }
    let p05 = &acceptance.p05_title;
    if p05.policy != P05TitlePolicy::NfcOrNfdExact
        || !matches!(p05.accepted_title.as_str(), P05_TITLE_NFC | P05_TITLE_NFD)
        || p05.unicode_scalar_count != p05.accepted_title.chars().count()
        || p05.utf8_byte_length != p05.accepted_title.len()
        || p05.setup_variance != (p05.accepted_title != P05_TITLE_NFC)
    {
        return Err(AuditError::new("P05_ACCEPTANCE_INVALID"));
    }
    let p09 = &acceptance.p09_title;
    if p09.policy != P09TitlePolicy::FixedNamePrefix
        || p09.accepted_title.len() < P09_TITLE_PREFIX.len()
        || !P09_TITLE_FULL.starts_with(&p09.accepted_title)
        || p09.unicode_scalar_count != p09.accepted_title.chars().count()
        || p09.utf8_byte_length != p09.accepted_title.len()
        || p09.setup_variance != (p09.accepted_title != P09_TITLE_FULL)
    {
        return Err(AuditError::new("P09_ACCEPTANCE_INVALID"));
    }
    Ok(())
}

fn safe_plugin_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SAFE_TEXT_BYTES
        && value.chars().all(|character| {
            !character.is_control() && !matches!(character, '/' | '\\' | ':' | '"' | '\'' | '`')
        })
}

fn safe_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_metadata_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SAFE_TEXT_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'.' | b'_' | b'-' | b'(' | b')')
        })
}

fn numeric_dot_version(value: &str) -> bool {
    let components: Vec<_> = value.split('.').collect();
    (1..=4).contains(&components.len())
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_macos_build(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && index < 3 && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == 0 || !bytes.get(index).is_some_and(u8::is_ascii_uppercase) {
        return false;
    }
    index += 1;
    let digit_start = index;
    while index < bytes.len() && index - digit_start < 5 && bytes[index].is_ascii_digit() {
        index += 1;
    }
    index > digit_start
        && (index == bytes.len() || (index + 1 == bytes.len() && bytes[index].is_ascii_lowercase()))
}

fn valid_rfc3339_with_timezone(value: &str) -> bool {
    parse_rfc3339_unix_ms(value).is_some()
}

fn parse_rfc3339_unix_ms(value: &str) -> Option<u64> {
    if value.len() != 29 || !value.is_ascii() {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes.get(19) != Some(&b'.')
        || !matches!(bytes.get(23), Some(b'+' | b'-'))
        || bytes.get(26) != Some(&b':')
    {
        return None;
    }
    let number = |start: usize, end: usize| {
        value
            .get(start..end)
            .filter(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|part| part.parse::<u32>().ok())
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0, 4),
        number(5, 7),
        number(8, 10),
        number(11, 13),
        number(14, 16),
        number(17, 19),
    ) else {
        return None;
    };
    if year < 2000
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut timezone_start = 19;
    let mut fractional_ms = 0u32;
    if bytes.get(timezone_start) == Some(&b'.') {
        timezone_start += 1;
        let fraction_start = timezone_start;
        while bytes.get(timezone_start).is_some_and(u8::is_ascii_digit) {
            timezone_start += 1;
        }
        if timezone_start - fraction_start != 3 {
            return None;
        }
        let fraction = &value[fraction_start..timezone_start];
        if fraction.bytes().skip(3).any(|digit| digit != b'0') {
            return None;
        }
        for (index, digit) in fraction.bytes().take(3).enumerate() {
            fractional_ms += u32::from(digit - b'0') * 10u32.pow(2 - index as u32);
        }
    }
    let offset_seconds = {
        if !matches!(bytes.get(timezone_start), Some(b'+' | b'-'))
            || timezone_start + 6 != bytes.len()
            || bytes.get(timezone_start + 3) != Some(&b':')
        {
            return None;
        }
        let offset_hour = value
            .get(timezone_start + 1..timezone_start + 3)?
            .parse::<u32>()
            .ok()?;
        let offset_minute = value
            .get(timezone_start + 4..timezone_start + 6)?
            .parse::<u32>()
            .ok()?;
        if offset_hour > 23 || offset_minute > 59 {
            return None;
        }
        let magnitude = i64::from(offset_hour * 3_600 + offset_minute * 60);
        if bytes[timezone_start] == b'+' {
            magnitude
        } else {
            -magnitude
        }
    };
    let mut days = 0i64;
    for candidate_year in 1970..year {
        days += if days_in_month(candidate_year, 2) == 29 {
            366
        } else {
            365
        };
    }
    for candidate_month in 1..month {
        days += i64::from(days_in_month(year, candidate_month));
    }
    days += i64::from(day - 1);
    let local_seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour * 3_600 + minute * 60 + second))?;
    let utc_seconds = local_seconds.checked_sub(offset_seconds)?;
    let millis = utc_seconds
        .checked_mul(1_000)?
        .checked_add(i64::from(fractional_ms))?;
    u64::try_from(millis).ok()
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

fn lower_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_checkpoint_id(value: &str) -> Option<&'static str> {
    REQUIRED_CHECKPOINTS
        .into_iter()
        .find(|candidate| *candidate == value)
}

#[derive(Debug)]
struct RawRecord {
    index: usize,
    record_type: String,
    timestamp_unix_ms: u64,
    monotonic_timestamp_ms: u64,
    value: Value,
}

struct NoDuplicateJson(Value);

impl<'de> Deserialize<'de> for NoDuplicateJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateJsonVisitor)
    }
}

struct NoDuplicateJsonVisitor;

impl<'de> Visitor<'de> for NoDuplicateJsonVisitor {
    type Value = NoDuplicateJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(NoDuplicateJson)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        NoDuplicateJson::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(NoDuplicateJson(value)) = sequence.next_element()? {
            values.push(value);
        }
        Ok(NoDuplicateJson(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            let NoDuplicateJson(value) = object.next_value()?;
            values.insert(key, value);
        }
        Ok(NoDuplicateJson(Value::Object(values)))
    }
}

fn parse_jsonl(bytes: &[u8]) -> AuditResult<Vec<RawRecord>> {
    if !bytes.ends_with(b"\n") {
        return Err(AuditError::new("LOG_FINAL_NEWLINE_MISSING"));
    }
    let mut records = Vec::new();
    let mut previous_monotonic = None;
    for (index, line_with_newline) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let line = &line_with_newline[..line_with_newline.len().saturating_sub(1)];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() > MAX_LOG_LINE_BYTES {
            return Err(AuditError::new("LOG_LINE_TOO_LARGE"));
        }
        if line.is_empty() {
            return Err(AuditError::new("LOG_BLANK_LINE"));
        }
        if records.len() >= MAX_LOG_RECORDS {
            return Err(AuditError::new("LOG_RECORD_LIMIT_EXCEEDED"));
        }
        let NoDuplicateJson(value) =
            serde_json::from_slice(line).map_err(|_| AuditError::new("LOG_RECORD_JSON_INVALID"))?;
        let object = value
            .as_object()
            .ok_or_else(|| AuditError::new("LOG_RECORD_NOT_OBJECT"))?;
        let record_type = required_string(object, "record_type", 64)?.to_owned();
        let timestamp_unix_ms = required_u64(object, "timestamp_unix_ms")?;
        let monotonic_timestamp_ms = required_u64(object, "monotonic_timestamp_ms")?;
        if previous_monotonic.is_some_and(|previous| monotonic_timestamp_ms < previous) {
            return Err(AuditError::new("LOG_MONOTONIC_ORDER_INVALID"));
        }
        previous_monotonic = Some(monotonic_timestamp_ms);
        records.push(RawRecord {
            index,
            record_type,
            timestamp_unix_ms,
            monotonic_timestamp_ms,
            value,
        });
    }
    if records.is_empty() {
        return Err(AuditError::new("LOG_EMPTY"));
    }
    Ok(records)
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    max: usize,
) -> AuditResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= max)
        .ok_or_else(|| AuditError::new("LOG_FIELD_INVALID"))
}

fn required_u64(object: &Map<String, Value>, key: &str) -> AuditResult<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| AuditError::new("LOG_FIELD_INVALID"))
}

fn required_bool(object: &Map<String, Value>, key: &str) -> AuditResult<bool> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| AuditError::new("LOG_FIELD_INVALID"))
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> AuditResult<&'a Map<String, Value>> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| AuditError::new("LOG_FIELD_INVALID"))
}

fn has_exact_keys(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

fn has_exact_record_keys(object: &Map<String, Value>, fields: &[&str]) -> bool {
    const COMMON: &[&str] = &[
        "record_format_version",
        "run_id",
        "timestamp_unix_ms",
        "monotonic_timestamp_ms",
        "record_type",
    ];
    object.len() == COMMON.len() + fields.len()
        && COMMON
            .iter()
            .chain(fields)
            .all(|key| object.contains_key(*key))
}

#[derive(Default)]
struct Evidence {
    start_count: usize,
    summary_count: usize,
    session_id: Option<String>,
    summary: Option<Value>,
    checkpoints: HashMap<&'static str, CheckpointEvidence>,
    actions: HashMap<&'static str, CheckpointMarker>,
    auto_actions: HashMap<&'static str, AutoActionBinding>,
    commands: HashMap<String, CommandEvidence>,
    commands_by_checkpoint: HashMap<&'static str, Vec<String>>,
    send_results: HashMap<String, SendEvidence>,
    responses: HashMap<String, ResponseEvidence>,
    discoveries: HashMap<String, DiscoveryEvidence>,
    snapshots: Vec<SnapshotEvidence>,
    completed_chunk_streams: usize,
    raw_snapshots: HashMap<(String, String), RawSnapshotAssembly>,
    loaded: Vec<LifecycleEvidence>,
    mapping_active: Vec<LifecycleEvidence>,
    capabilities: Vec<CapabilityEventEvidence>,
    ready: Vec<LifecycleEvidence>,
    not_ready: Vec<LifecycleEvidence>,
    probe_source_ids: HashSet<String>,
    last_source_seq: HashMap<String, u64>,
    direct_access_event_observed: bool,
    last_probe_receive_by_checkpoint: HashMap<&'static str, ProbePosition>,
    probe_positions_by_checkpoint: HashMap<&'static str, Vec<ProbePosition>>,
    probe_records: usize,
    drain_started: Option<DrainStartedEvidence>,
    drain_completed: Option<DrainCompletedEvidence>,
}

#[derive(Default)]
struct CheckpointEvidence {
    begin: Option<CheckpointMarker>,
    end: Option<CheckpointMarker>,
}

#[derive(Clone, Copy)]
struct DrainStartedEvidence {
    marker: CheckpointMarker,
    timeout_ms: u64,
    deadline_monotonic_timestamp_ms: u64,
}

#[derive(Clone, Copy)]
struct DrainCompletedEvidence {
    marker: CheckpointMarker,
    completed: bool,
    timed_out: bool,
    duration_ms: u64,
}

#[derive(Clone, Copy)]
struct CheckpointMarker {
    timestamp_unix_ms: u64,
    monotonic_timestamp_ms: u64,
    record_index: usize,
}

struct AutoActionBinding {
    request_id: String,
    observation_epoch: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ProbePosition {
    monotonic_timestamp_ms: u64,
    record_index: usize,
}

struct CommandEvidence {
    checkpoint_id: &'static str,
    request_id: String,
    target_instance_id: Option<String>,
    method: String,
    config_id: Option<String>,
    index: usize,
    emitted_monotonic_timestamp_ms: u64,
}

struct SendEvidence {
    index: usize,
    sent: bool,
    completed_monotonic_timestamp_ms: u64,
    emitted_monotonic_timestamp_ms: u64,
}

struct ResponseEvidence {
    checkpoint_id: &'static str,
    source_instance_id: String,
    received_monotonic_timestamp_ms: u64,
    record_index: usize,
    result: Value,
}

struct DiscoveryEvidence {
    checkpoint_id: &'static str,
    selected_source_instance_id: String,
    monotonic_timestamp_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotKind {
    Bank,
    DirectAccess,
}

struct SnapshotEvidence {
    checkpoint_id: &'static str,
    source_instance_id: String,
    kind: SnapshotKind,
    config_id: Option<String>,
    reason: String,
    snapshot_id: String,
    received_monotonic_timestamp_ms: u64,
    record_index: usize,
}

struct RawSnapshotAssembly {
    checkpoint_id: &'static str,
    source_instance_id: String,
    kind: SnapshotKind,
    is_snapshot: bool,
    config_id: Option<String>,
    reason: String,
    observation_epoch: Option<u64>,
    observation_epoch_status: Option<String>,
    bank_generation: Option<u64>,
    observation_items: usize,
    first_observation_seq: Option<u64>,
    last_observation_seq: Option<u64>,
    remaining_feedback_items: Option<u64>,
    direct_access_base_object_id: Option<u64>,
    direct_access_reference_items: Option<usize>,
    direct_access_cycle_count: Option<u64>,
    direct_access_shared_reference_count: Option<u64>,
    direct_access_error_count: Option<u64>,
    chunk_count: usize,
    next_chunk: usize,
    total_items: usize,
    items: Vec<Value>,
    item_positions: Vec<ProbePosition>,
    chunk_positions: Vec<ProbePosition>,
    completed: bool,
}

struct LifecycleEvidence {
    checkpoint_id: &'static str,
    source_instance_id: String,
    source_seq: u64,
    monotonic_timestamp_ms: u64,
    index: usize,
}

struct CapabilityEventEvidence {
    checkpoint_id: &'static str,
    source_instance_id: String,
    monotonic_timestamp_ms: u64,
    index: usize,
    data: Value,
}

fn collect_evidence(manifest: &AuditManifest, records: &[RawRecord]) -> AuditResult<Evidence> {
    let mut evidence = Evidence::default();
    for record in records {
        let object = record
            .value
            .as_object()
            .ok_or_else(|| AuditError::new("LOG_RECORD_NOT_OBJECT"))?;
        if object.get("record_format_version").and_then(Value::as_u64)
            != Some(RECORD_FORMAT_VERSION)
        {
            return Err(AuditError::new("RECORD_FORMAT_VERSION_INVALID"));
        }
        if object.get("run_id").and_then(Value::as_str) != Some(manifest.run_id.as_str()) {
            return Err(AuditError::new("RUN_ID_MISMATCH"));
        }
        match record.record_type.as_str() {
            "collector_started" => collect_started(record, object, manifest, &mut evidence)?,
            "collector_checkpoint" => collect_checkpoint(record, object, &mut evidence)?,
            "collector_action" => collect_action(record, object, &mut evidence)?,
            "probe_command" => collect_command(record, object, &mut evidence)?,
            "probe_command_send_result" => collect_send_result(record, object, &mut evidence)?,
            "probe_response" => collect_probe_response(record, object, &mut evidence)?,
            "probe_error" => return Err(AuditError::new("PROBE_ERROR_OBSERVED")),
            "probe_event" => collect_probe_event(record, object, &mut evidence)?,
            "collector_discovery_completed" => collect_discovery(record, object, &mut evidence)?,
            "collector_diagnostic" => return Err(AuditError::new("COLLECTOR_DIAGNOSTIC_OBSERVED")),
            "collector_drain_started" => collect_drain_started(record, object, &mut evidence)?,
            "collector_drain_completed" => collect_drain_completed(record, object, &mut evidence)?,
            "collector_summary" => collect_summary(record, object, &mut evidence)?,
            _ => return Err(AuditError::new("LOG_RECORD_TYPE_UNKNOWN")),
        }
    }
    Ok(evidence)
}

fn collect_drain_started(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    if !has_exact_record_keys(object, &["timeout_ms", "deadline_monotonic_timestamp_ms"]) {
        return Err(AuditError::new("DRAIN_STARTED_SCHEMA_INVALID"));
    }
    let started = DrainStartedEvidence {
        marker: CheckpointMarker {
            timestamp_unix_ms: record.timestamp_unix_ms,
            monotonic_timestamp_ms: record.monotonic_timestamp_ms,
            record_index: record.index,
        },
        timeout_ms: required_u64(object, "timeout_ms")?,
        deadline_monotonic_timestamp_ms: required_u64(object, "deadline_monotonic_timestamp_ms")?,
    };
    if evidence.drain_started.replace(started).is_some() {
        return Err(AuditError::new("DRAIN_STARTED_DUPLICATE"));
    }
    Ok(())
}

fn collect_drain_completed(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    if !has_exact_record_keys(object, &["completed", "timed_out", "duration_ms"]) {
        return Err(AuditError::new("DRAIN_COMPLETED_SCHEMA_INVALID"));
    }
    let completed = DrainCompletedEvidence {
        marker: CheckpointMarker {
            timestamp_unix_ms: record.timestamp_unix_ms,
            monotonic_timestamp_ms: record.monotonic_timestamp_ms,
            record_index: record.index,
        },
        completed: required_bool(object, "completed")?,
        timed_out: required_bool(object, "timed_out")?,
        duration_ms: required_u64(object, "duration_ms")?,
    };
    if evidence.drain_completed.replace(completed).is_some() {
        return Err(AuditError::new("DRAIN_COMPLETED_DUPLICATE"));
    }
    Ok(())
}

fn collect_started(
    record: &RawRecord,
    object: &Map<String, Value>,
    manifest: &AuditManifest,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    evidence.start_count += 1;
    if !has_exact_record_keys(
        object,
        &[
            "session_id",
            "collector_version",
            "collector_binary_sha256",
            "probe_transport_version",
            "midi_mode",
            "virtual_to_cubase_port",
            "virtual_from_cubase_port",
            "configured_midi_input_port",
            "configured_midi_output_port",
            "resolved_midi_input_port",
            "resolved_midi_output_port",
            "max_json_bytes",
            "max_sysex_bytes",
            "max_outbound_json_bytes",
            "queue_capacity",
            "ingress_barrier_timeout_ms",
            "checkpoint_quiet_period_ms",
            "graceful_drain_timeout_ms",
            "discovery_window_ms",
        ],
    ) {
        return Err(AuditError::new("COLLECTOR_START_SCHEMA_INVALID"));
    }
    let session_id = required_string(object, "session_id", 128)?;
    if evidence.session_id.replace(session_id.to_owned()).is_some() {
        return Err(AuditError::new("COLLECTOR_START_DUPLICATE"));
    }
    if object
        .get("probe_transport_version")
        .and_then(Value::as_u64)
        != Some(1)
        || required_u64(object, "max_json_bytes")? != 65_536
        || required_u64(object, "max_sysex_bytes")? != 131_080
        || required_u64(object, "max_outbound_json_bytes")? != 4_096
        || required_u64(object, "queue_capacity")? != 1_024
        || required_u64(object, "ingress_barrier_timeout_ms")? != 5_000
        || required_u64(object, "discovery_window_ms")? != 1_000
        || required_u64(object, "graceful_drain_timeout_ms")? != 5_000
        || required_u64(object, "checkpoint_quiet_period_ms")? != MIN_QUIET_PERIOD_MS
        || record.index != 0
    {
        return Err(AuditError::new("COLLECTOR_START_CONFIG_INVALID"));
    }
    validate_collector_midi_mode(object)?;
    if parse_rfc3339_unix_ms(&manifest.run_started_at) != Some(record.timestamp_unix_ms) {
        return Err(AuditError::new("RUN_START_TIME_MISMATCH"));
    }
    if !safe_metadata_text(required_string(object, "collector_version", 96)?)
        || required_string(object, "collector_binary_sha256", 64)?
            != manifest.environment.collector_binary_sha256
    {
        return Err(AuditError::new("COLLECTOR_BINARY_IDENTITY_MISMATCH"));
    }
    Ok(())
}

fn validate_collector_midi_mode(object: &Map<String, Value>) -> AuditResult<()> {
    let nullable_string = |key: &str| {
        object.get(key).is_some_and(|value| {
            value.is_null() || value.as_str().is_some_and(|text| !text.is_empty())
        })
    };
    let mode = required_string(object, "midi_mode", 16)?;
    let valid = match mode {
        "existing" => {
            object
                .get("virtual_to_cubase_port")
                .is_some_and(Value::is_null)
                && object
                    .get("virtual_from_cubase_port")
                    .is_some_and(Value::is_null)
                && nullable_string("configured_midi_input_port")
                && nullable_string("configured_midi_output_port")
                && required_string(object, "resolved_midi_input_port", 512).is_ok()
                && required_string(object, "resolved_midi_output_port", 512).is_ok()
        }
        "virtual" => {
            object
                .get("configured_midi_input_port")
                .is_some_and(Value::is_null)
                && object
                    .get("configured_midi_output_port")
                    .is_some_and(Value::is_null)
                && object.get("virtual_to_cubase_port").and_then(Value::as_str)
                    == Some("Cubase MCP Track Probe To Cubase")
                && object
                    .get("virtual_from_cubase_port")
                    .and_then(Value::as_str)
                    == Some("Cubase MCP Track Probe From Cubase")
                && required_string(object, "resolved_midi_input_port", 512).is_ok()
                && required_string(object, "resolved_midi_output_port", 512).is_ok()
        }
        _ => false,
    };
    if !valid {
        return Err(AuditError::new("COLLECTOR_MIDI_MODE_INVALID"));
    }
    Ok(())
}

fn collect_checkpoint(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    let id = required_string(object, "checkpoint_id", 128)?;
    let id = canonical_checkpoint_id(id).ok_or_else(|| AuditError::new("CHECKPOINT_UNEXPECTED"))?;
    let phase = required_string(object, "phase", 32)?;
    let schema_valid = match phase {
        "begin" => has_exact_record_keys(object, &["phase", "checkpoint_id", "window_ms"]),
        "end" => has_exact_record_keys(
            object,
            &[
                "phase",
                "checkpoint_id",
                "window_ms",
                "observed_duration_ms",
                "window_satisfied",
                "quiet_period_required_ms",
                "quiet_period_observed_ms",
                "quiet_period_satisfied",
                "messages_processed_before_end_marker",
                "late_received_frames_may_be_classified_by_receive_timestamp",
            ],
        ),
        _ => false,
    };
    if !schema_valid {
        return Err(AuditError::new("CHECKPOINT_RECORD_SCHEMA_INVALID"));
    }
    let checkpoint = evidence.checkpoints.entry(id).or_default();
    let marker = CheckpointMarker {
        timestamp_unix_ms: record.timestamp_unix_ms,
        monotonic_timestamp_ms: record.monotonic_timestamp_ms,
        record_index: record.index,
    };
    match phase {
        "begin" => {
            if checkpoint.begin.replace(marker).is_some()
                || required_u64(object, "window_ms")? != CALLBACK_WINDOW_MS
            {
                return Err(AuditError::new("CHECKPOINT_BEGIN_INVALID"));
            }
        }
        "end" => {
            if checkpoint.end.replace(marker).is_some()
                || required_u64(object, "window_ms")? != CALLBACK_WINDOW_MS
                || required_u64(object, "observed_duration_ms")? < CALLBACK_WINDOW_MS
                || !required_bool(object, "window_satisfied")?
                || required_u64(object, "quiet_period_required_ms")? != MIN_QUIET_PERIOD_MS
                || required_u64(object, "quiet_period_observed_ms")? < MIN_QUIET_PERIOD_MS
                || !required_bool(object, "quiet_period_satisfied")?
                || required_u64(object, "messages_processed_before_end_marker")?
                    != evidence
                        .probe_positions_by_checkpoint
                        .get(id)
                        .map_or(0, Vec::len) as u64
                || !required_bool(
                    object,
                    "late_received_frames_may_be_classified_by_receive_timestamp",
                )?
            {
                return Err(AuditError::new("CHECKPOINT_END_INVALID"));
            }
        }
        "end_deferred" | "aborted_eof" => {
            return Err(AuditError::new("CHECKPOINT_INCOMPLETE"));
        }
        _ => return Err(AuditError::new("CHECKPOINT_PHASE_INVALID")),
    }
    Ok(())
}

fn collect_action(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    let checkpoint_id = canonical_checkpoint_id(required_string(object, "checkpoint_id", 128)?)
        .ok_or_else(|| AuditError::new("ACTION_CHECKPOINT_INVALID"))?;
    let auto = requires_observation_cut(checkpoint_id);
    if !has_exact_record_keys(
        object,
        if auto {
            &[
                "phase",
                "checkpoint_id",
                "boundary_source",
                "request_id",
                "observation_epoch",
            ]
        } else {
            &["phase", "checkpoint_id"]
        },
    ) {
        return Err(AuditError::new("ACTION_RECORD_SCHEMA_INVALID"));
    }
    if required_string(object, "phase", 32)? != "marked" {
        return Err(AuditError::new("ACTION_PHASE_INVALID"));
    }
    if auto
        && (object.get("boundary_source").and_then(Value::as_str)
            != Some("probe.observation.cut_response")
            || evidence
                .auto_actions
                .insert(
                    checkpoint_id,
                    AutoActionBinding {
                        request_id: required_string(object, "request_id", 128)?.to_owned(),
                        observation_epoch: required_u64(object, "observation_epoch")?,
                    },
                )
                .is_some())
    {
        return Err(AuditError::new("ACTION_BOUNDARY_INVALID"));
    }
    if evidence
        .actions
        .insert(
            checkpoint_id,
            CheckpointMarker {
                timestamp_unix_ms: record.timestamp_unix_ms,
                monotonic_timestamp_ms: record.monotonic_timestamp_ms,
                record_index: record.index,
            },
        )
        .is_some()
    {
        return Err(AuditError::new("ACTION_MARKER_DUPLICATE"));
    }
    Ok(())
}

fn collect_command(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    if required_string(object, "phase", 16)? != "started" {
        return Err(AuditError::new("COMMAND_PHASE_INVALID"));
    }
    let checkpoint_id = canonical_checkpoint_id(required_string(object, "checkpoint_id", 128)?)
        .ok_or_else(|| AuditError::new("COMMAND_CHECKPOINT_INVALID"))?;
    let request_id = required_string(object, "request_id", 128)?.to_owned();
    let request = required_object(object, "request")?;
    let message = required_object(request, "message")?;
    if !has_exact_keys(
        request,
        &["probe_transport_version", "target_instance_id", "message"],
    ) || !has_exact_keys(message, &["version", "id", "type", "method", "params"])
        || request
            .get("probe_transport_version")
            .and_then(Value::as_u64)
            != Some(1)
        || required_string(message, "id", 128)? != request_id
        || required_string(message, "type", 16)? != "request"
        || message.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Err(AuditError::new("COMMAND_ENVELOPE_INVALID"));
    }
    let method = required_string(message, "method", 128)?.to_owned();
    let selected = method != "probe.discover";
    if selected && !object.contains_key("evidence_emission") {
        return Err(AuditError::new("SELECTED_COMMAND_EVIDENCE_MISSING"));
    }
    if !has_exact_record_keys(
        object,
        if selected {
            &[
                "phase",
                "request_id",
                "checkpoint_id",
                "request",
                "evidence_emission",
            ]
        } else {
            &["phase", "request_id", "checkpoint_id", "request"]
        },
    ) {
        return Err(AuditError::new("COMMAND_RECORD_SCHEMA_INVALID"));
    }
    if selected
        && object.get("evidence_emission").and_then(Value::as_str)
            != Some("after_midi_send_attempt")
    {
        return Err(AuditError::new("SELECTED_COMMAND_EVIDENCE_MISSING"));
    }
    let params = required_object(message, "params")?;
    let config_id = params
        .get("config_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if (config_id.is_some() && params.len() != 1) || (config_id.is_none() && !params.is_empty()) {
        return Err(AuditError::new("COMMAND_PARAMS_INVALID"));
    }
    let target_instance_id = request
        .get("target_instance_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if method == "probe.discover" {
        if request
            .get("target_instance_id")
            .is_some_and(|value| !value.is_null())
        {
            return Err(AuditError::new("DISCOVERY_TARGET_INVALID"));
        }
    } else if request
        .get("target_instance_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(AuditError::new("COMMAND_TARGET_MISSING"));
    }
    let command = CommandEvidence {
        checkpoint_id,
        request_id: request_id.clone(),
        target_instance_id,
        method,
        config_id,
        index: record.index,
        emitted_monotonic_timestamp_ms: record.monotonic_timestamp_ms,
    };
    if evidence
        .commands
        .insert(request_id.clone(), command)
        .is_some()
    {
        return Err(AuditError::new("COMMAND_REQUEST_DUPLICATE"));
    }
    evidence
        .commands_by_checkpoint
        .entry(checkpoint_id)
        .or_default()
        .push(request_id);
    Ok(())
}

fn collect_send_result(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    let request_id = required_string(object, "request_id", 128)?.to_owned();
    let command = evidence
        .commands
        .get(&request_id)
        .ok_or_else(|| AuditError::new("COMMAND_SEND_RESULT_ORPHAN"))?;
    if !has_exact_record_keys(
        object,
        if command.method == "probe.discover" {
            &[
                "request_id",
                "checkpoint_id",
                "sent",
                "sysex_bytes",
                "send_completed_monotonic_timestamp_ms",
            ]
        } else {
            &[
                "request_id",
                "checkpoint_id",
                "sent",
                "sysex_bytes",
                "evidence_emission",
                "send_completed_monotonic_timestamp_ms",
            ]
        },
    ) || required_u64(object, "sysex_bytes")? == 0
    {
        return Err(AuditError::new("COMMAND_SEND_RECORD_SCHEMA_INVALID"));
    }
    if command.method != "probe.discover"
        && object.get("evidence_emission").and_then(Value::as_str)
            != Some("after_midi_send_attempt")
    {
        return Err(AuditError::new("SELECTED_SEND_EVIDENCE_MISSING"));
    }
    let send = SendEvidence {
        index: record.index,
        sent: required_bool(object, "sent")?,
        completed_monotonic_timestamp_ms: required_u64(
            object,
            "send_completed_monotonic_timestamp_ms",
        )?,
        emitted_monotonic_timestamp_ms: record.monotonic_timestamp_ms,
    };
    if evidence.send_results.insert(request_id, send).is_some() {
        return Err(AuditError::new("COMMAND_SEND_RESULT_DUPLICATE"));
    }
    Ok(())
}

fn validate_probe_record_context(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &Evidence,
) -> AuditResult<(&'static str, String, u64)> {
    const OUTER_KEYS: &[&str] = &[
        "record_format_version",
        "run_id",
        "timestamp_unix_ms",
        "monotonic_timestamp_ms",
        "record_type",
        "received_at_unix_ms",
        "received_at_monotonic_timestamp_ms",
        "midi_timestamp",
        "integrity_ok_at_emit",
        "probe_transport_version",
        "source_instance_id",
        "source_seq",
        "checkpoint_id",
        "orphan",
        "checkpoint_elapsed_ms",
        "checkpoint_window_ms",
        "checkpoint_window_expired",
        "processed_after_checkpoint_end",
        "checkpoint_quiet_period_violated",
        "message",
    ];
    if !has_exact_keys(object, OUTER_KEYS)
        || object
            .get("probe_transport_version")
            .and_then(Value::as_u64)
            != Some(1)
        || object
            .get("midi_timestamp")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Err(AuditError::new("PROBE_RECORD_SCHEMA_INVALID"));
    }
    let checkpoint = canonical_checkpoint_id(required_string(object, "checkpoint_id", 128)?)
        .ok_or_else(|| AuditError::new("PROBE_CHECKPOINT_INVALID"))?;
    let checkpoint_begin = evidence
        .checkpoints
        .get(checkpoint)
        .and_then(|checkpoint| checkpoint.begin)
        .ok_or_else(|| AuditError::new("PROBE_CHECKPOINT_CONTEXT_INVALID"))?;
    let checkpoint_elapsed = required_u64(object, "checkpoint_elapsed_ms")?;
    let checkpoint_window = required_u64(object, "checkpoint_window_ms")?;
    let checkpoint_expired = required_bool(object, "checkpoint_window_expired")?;
    if checkpoint_window != CALLBACK_WINDOW_MS
        || checkpoint_expired != (checkpoint_elapsed >= checkpoint_window)
        || record.index <= checkpoint_begin.record_index
        || record.monotonic_timestamp_ms < checkpoint_begin.monotonic_timestamp_ms
    {
        return Err(AuditError::new("PROBE_CHECKPOINT_CONTEXT_INVALID"));
    }
    if required_bool(object, "orphan")?
        || required_bool(object, "processed_after_checkpoint_end")?
        || required_bool(object, "checkpoint_quiet_period_violated")?
        || !required_bool(object, "integrity_ok_at_emit")?
    {
        return Err(AuditError::new("PROBE_RECORD_INTEGRITY_INVALID"));
    }
    let source = required_string(object, "source_instance_id", 256)?.to_owned();
    let source_seq = required_u64(object, "source_seq")?;
    Ok((checkpoint, source, source_seq))
}

fn collect_probe_response(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    let (checkpoint_id, source, source_seq) =
        validate_probe_record_context(record, object, evidence)?;
    let received_at_unix_ms = required_u64(object, "received_at_unix_ms")?;
    let received_monotonic_timestamp_ms =
        required_u64(object, "received_at_monotonic_timestamp_ms")?;
    if received_at_unix_ms > record.timestamp_unix_ms
        || received_monotonic_timestamp_ms > record.monotonic_timestamp_ms
    {
        return Err(AuditError::new("PROBE_RECEIVE_TIMESTAMP_INVALID"));
    }
    let position = ProbePosition {
        monotonic_timestamp_ms: received_monotonic_timestamp_ms,
        record_index: record.index,
    };
    evidence
        .last_probe_receive_by_checkpoint
        .entry(checkpoint_id)
        .and_modify(|last| {
            *last = (*last).max(position);
        })
        .or_insert(position);
    evidence
        .probe_positions_by_checkpoint
        .entry(checkpoint_id)
        .or_default()
        .push(position);
    let message = required_object(object, "message")?;
    if !has_exact_keys(message, &["version", "id", "type", "result"])
        || message.get("version").and_then(Value::as_u64) != Some(1)
        || required_string(message, "type", 16)? != "response"
        || !message.get("result").is_some_and(Value::is_object)
    {
        return Err(AuditError::new("PROBE_RESPONSE_INVALID"));
    }
    observe_source_sequence(evidence, &source, source_seq, false)?;
    evidence.probe_records += 1;
    let request_id = required_string(message, "id", 128)?.to_owned();
    let result = message
        .get("result")
        .cloned()
        .ok_or_else(|| AuditError::new("PROBE_RESPONSE_INVALID"))?;
    if evidence
        .responses
        .insert(
            request_id,
            ResponseEvidence {
                checkpoint_id,
                source_instance_id: source,
                received_monotonic_timestamp_ms,
                record_index: record.index,
                result,
            },
        )
        .is_some()
    {
        return Err(AuditError::new("PROBE_RESPONSE_DUPLICATE"));
    }
    Ok(())
}

fn collect_probe_event(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    let (checkpoint_id, source, source_seq) =
        validate_probe_record_context(record, object, evidence)?;
    let received_at_unix_ms = required_u64(object, "received_at_unix_ms")?;
    let received_monotonic_timestamp_ms =
        required_u64(object, "received_at_monotonic_timestamp_ms")?;
    if received_at_unix_ms > record.timestamp_unix_ms
        || received_monotonic_timestamp_ms > record.monotonic_timestamp_ms
    {
        return Err(AuditError::new("PROBE_RECEIVE_TIMESTAMP_INVALID"));
    }
    let position = ProbePosition {
        monotonic_timestamp_ms: received_monotonic_timestamp_ms,
        record_index: record.index,
    };
    evidence
        .last_probe_receive_by_checkpoint
        .entry(checkpoint_id)
        .and_modify(|last| {
            *last = (*last).max(position);
        })
        .or_insert(position);
    evidence
        .probe_positions_by_checkpoint
        .entry(checkpoint_id)
        .or_default()
        .push(position);
    let message = required_object(object, "message")?;
    if !has_exact_keys(message, &["version", "type", "event", "data"])
        || message.get("version").and_then(Value::as_u64) != Some(1)
        || required_string(message, "type", 16)? != "event"
    {
        return Err(AuditError::new("PROBE_EVENT_INVALID"));
    }
    let event = required_string(message, "event", 128)?;
    let data = required_object(message, "data")?;
    if event.starts_with("probe.direct_access.") {
        evidence.direct_access_event_observed = true;
    }
    observe_source_sequence(evidence, &source, source_seq, event == "probe.loaded")?;
    evidence.probe_records += 1;
    match event {
        "probe.loaded" => {
            validate_lifecycle_metadata(data, &source, "loaded")?;
            evidence.loaded.push(LifecycleEvidence {
                checkpoint_id,
                source_instance_id: source,
                source_seq,
                monotonic_timestamp_ms: received_monotonic_timestamp_ms,
                index: record.index,
            });
        }
        "probe.mapping_active" => {
            validate_lifecycle_metadata(data, &source, "mapping_active")?;
            evidence.mapping_active.push(LifecycleEvidence {
                checkpoint_id,
                source_instance_id: source,
                source_seq,
                monotonic_timestamp_ms: received_monotonic_timestamp_ms,
                index: record.index,
            });
        }
        "probe.capabilities" => {
            evidence.capabilities.push(CapabilityEventEvidence {
                checkpoint_id,
                source_instance_id: source,
                monotonic_timestamp_ms: received_monotonic_timestamp_ms,
                index: record.index,
                data: Value::Object(data.clone()),
            });
        }
        "probe.ready"
            if data.get("ready").and_then(Value::as_bool) == Some(true)
                && data
                    .get("initial_snapshots_complete")
                    .and_then(Value::as_bool)
                    == Some(true) =>
        {
            validate_lifecycle_metadata(data, &source, "ready")?;
            evidence.ready.push(LifecycleEvidence {
                checkpoint_id,
                source_instance_id: source,
                source_seq,
                monotonic_timestamp_ms: received_monotonic_timestamp_ms,
                index: record.index,
            });
        }
        "probe.ready" if data.get("ready").and_then(Value::as_bool) == Some(false) => {
            validate_lifecycle_metadata(data, &source, "not_ready")?;
            evidence.not_ready.push(LifecycleEvidence {
                checkpoint_id,
                source_instance_id: source,
                source_seq,
                monotonic_timestamp_ms: received_monotonic_timestamp_ms,
                index: record.index,
            });
        }
        "probe.bank.chunk" | "probe.direct_access.chunk" => collect_snapshot_chunk(
            evidence,
            checkpoint_id,
            &source,
            event,
            data,
            received_monotonic_timestamp_ms,
            record.index,
        )?,
        "probe.overflow" => return Err(AuditError::new("PROBE_OVERFLOW_OBSERVED")),
        "probe.direct_access.error" => {
            return Err(AuditError::new("DIRECT_ACCESS_ERROR_OBSERVED"));
        }
        _ => return Err(AuditError::new("PROBE_EVENT_UNEXPECTED")),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_snapshot_chunk(
    evidence: &mut Evidence,
    checkpoint_id: &'static str,
    source_instance_id: &str,
    event: &str,
    data: &Map<String, Value>,
    received_monotonic_timestamp_ms: u64,
    record_index: usize,
) -> AuditResult<()> {
    let stream = required_string(data, "stream", 64)?;
    let (kind, is_snapshot) = match (event, stream) {
        ("probe.bank.chunk", "mixer_bank_feedback") => (SnapshotKind::Bank, false),
        ("probe.direct_access.chunk", "direct_access_feedback") => {
            (SnapshotKind::DirectAccess, false)
        }
        ("probe.bank.chunk", "mixer_bank_snapshot") => (SnapshotKind::Bank, true),
        ("probe.direct_access.chunk", "direct_access_snapshot") => {
            (SnapshotKind::DirectAccess, true)
        }
        _ => return Err(AuditError::new("SNAPSHOT_STREAM_INVALID")),
    };
    validate_chunk_data_schema(data, kind, is_snapshot)?;
    if data.get("truncated").and_then(Value::as_bool) != Some(false)
        || data.get("overflow_safe").and_then(Value::as_bool) != Some(true)
    {
        return Err(AuditError::new("SNAPSHOT_CHUNK_INVALID"));
    }
    let snapshot_id = required_string(data, "snapshot_id", 256)?.to_owned();
    let reason = required_string(data, "reason", 64)?.to_owned();
    let config_id = data
        .get("config_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let observation_epoch = data.get("observation_epoch").and_then(Value::as_u64);
    let observation_epoch_status = data
        .get("observation_epoch_status")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let bank_generation = data.get("bank_generation").and_then(Value::as_u64);
    let observation_items = usize::try_from(required_u64(data, "observation_items")?)
        .map_err(|_| AuditError::new("SNAPSHOT_CHUNK_INVALID"))?;
    let first_observation_seq = data.get("first_observation_seq").and_then(Value::as_u64);
    let last_observation_seq = data.get("last_observation_seq").and_then(Value::as_u64);
    let remaining_feedback_items = data.get("remaining_items").and_then(Value::as_u64);
    let direct_access_base_object_id = data.get("base_object_id").and_then(Value::as_u64);
    let direct_access_reference_items = data
        .get("reference_items")
        .and_then(Value::as_u64)
        .map(usize::try_from)
        .transpose()
        .map_err(|_| AuditError::new("DIRECT_ACCESS_CHUNK_METADATA_INVALID"))?;
    let direct_access_cycle_count = data.get("cycle_count").and_then(Value::as_u64);
    let direct_access_shared_reference_count =
        data.get("shared_reference_count").and_then(Value::as_u64);
    let direct_access_error_count = data.get("error_count").and_then(Value::as_u64);
    if (is_snapshot && kind == SnapshotKind::Bank && config_id.is_none())
        || (is_snapshot && kind == SnapshotKind::DirectAccess && config_id.is_some())
        || (!is_snapshot && config_id.is_some())
        || (is_snapshot
            && kind == SnapshotKind::DirectAccess
            && (observation_epoch.is_none()
                || observation_epoch_status.as_deref() != Some("snapshot_observed")))
        || (is_snapshot
            && kind == SnapshotKind::Bank
            && (bank_generation.is_none()
                || data
                    .get("requested_bank_generation")
                    .and_then(Value::as_u64)
                    != bank_generation
                || data.get("superseded").and_then(Value::as_bool) != Some(false)
                || data.get("follow_visibility").and_then(Value::as_bool)
                    != Some(config_id.as_deref() == Some("MB_CORE_VISIBLE"))))
    {
        return Err(AuditError::new("SNAPSHOT_CONFIG_INVALID"));
    }
    let chunk_index = usize::try_from(required_u64(data, "chunk_index")?)
        .map_err(|_| AuditError::new("SNAPSHOT_CHUNK_INVALID"))?;
    let chunk_count = usize::try_from(required_u64(data, "chunk_count")?)
        .map_err(|_| AuditError::new("SNAPSHOT_CHUNK_INVALID"))?;
    let total_items = usize::try_from(required_u64(data, "total_items")?)
        .map_err(|_| AuditError::new("SNAPSHOT_CHUNK_INVALID"))?;
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("SNAPSHOT_ITEMS_INVALID"))?;
    let snapshot_complete = required_bool(data, "snapshot_complete")?;
    let reason_valid = if is_snapshot {
        match kind {
            SnapshotKind::Bank => matches!(
                reason.as_str(),
                "page_activate"
                    | "command_reset"
                    | "command_next"
                    | "command_prev"
                    | "command_snapshot"
            ),
            SnapshotKind::DirectAccess => matches!(
                reason.as_str(),
                "page_activate"
                    | "object_change"
                    | "object_will_be_removed"
                    | "parameter_change"
                    | "command_snapshot"
            ),
        }
    } else {
        reason == "feedback"
    };
    let observation_limit = match (kind, is_snapshot) {
        (SnapshotKind::Bank, true) => 8,
        (SnapshotKind::DirectAccess, true) => 256,
        (_, false) => 512,
    };
    if !reason_valid
        || chunk_count == 0
        || chunk_count > 1_024
        || chunk_index >= chunk_count
        || total_items > 1_024
        || items.len() > 2
        || (total_items == 0 && (chunk_count != 1 || !items.is_empty()))
        || (total_items > 0
            && (items.is_empty()
                || chunk_count > total_items
                || chunk_count < total_items.div_ceil(2)))
        || observation_items > total_items
        || direct_access_reference_items
            .is_some_and(|references| observation_items.saturating_add(references) > total_items)
        || observation_items > observation_limit
        || snapshot_complete != (chunk_index + 1 == chunk_count)
    {
        return Err(AuditError::new("SNAPSHOT_CHUNK_INVALID"));
    }
    let key = (source_instance_id.to_owned(), snapshot_id.clone());
    let assembly = evidence
        .raw_snapshots
        .entry(key)
        .or_insert_with(|| RawSnapshotAssembly {
            checkpoint_id,
            source_instance_id: source_instance_id.to_owned(),
            kind,
            is_snapshot,
            config_id: config_id.clone(),
            reason: reason.clone(),
            observation_epoch,
            observation_epoch_status: observation_epoch_status.clone(),
            bank_generation,
            observation_items,
            first_observation_seq,
            last_observation_seq,
            remaining_feedback_items,
            direct_access_base_object_id,
            direct_access_reference_items,
            direct_access_cycle_count,
            direct_access_shared_reference_count,
            direct_access_error_count,
            chunk_count,
            next_chunk: 0,
            total_items,
            items: Vec::with_capacity(total_items),
            item_positions: Vec::with_capacity(total_items),
            chunk_positions: Vec::with_capacity(chunk_count),
            completed: false,
        });
    if assembly.completed
        || assembly.checkpoint_id != checkpoint_id
        || assembly.source_instance_id != source_instance_id
        || assembly.kind != kind
        || assembly.is_snapshot != is_snapshot
        || assembly.config_id != config_id
        || assembly.reason != reason
        || assembly.observation_epoch != observation_epoch
        || assembly.observation_epoch_status != observation_epoch_status
        || assembly.bank_generation != bank_generation
        || assembly.observation_items != observation_items
        || assembly.first_observation_seq != first_observation_seq
        || assembly.last_observation_seq != last_observation_seq
        || assembly.remaining_feedback_items != remaining_feedback_items
        || assembly.direct_access_base_object_id != direct_access_base_object_id
        || assembly.direct_access_reference_items != direct_access_reference_items
        || assembly.direct_access_cycle_count != direct_access_cycle_count
        || assembly.direct_access_shared_reference_count != direct_access_shared_reference_count
        || assembly.direct_access_error_count != direct_access_error_count
        || assembly.chunk_count != chunk_count
        || assembly.total_items != total_items
        || assembly.next_chunk != chunk_index
    {
        return Err(AuditError::new("SNAPSHOT_CHUNK_SEQUENCE_INVALID"));
    }
    let position = ProbePosition {
        monotonic_timestamp_ms: received_monotonic_timestamp_ms,
        record_index,
    };
    assembly.items.extend(items.iter().cloned());
    assembly
        .item_positions
        .extend(std::iter::repeat_n(position, items.len()));
    assembly.chunk_positions.push(position);
    assembly.next_chunk += 1;
    if snapshot_complete {
        if assembly.next_chunk != assembly.chunk_count || assembly.items.len() != total_items {
            return Err(AuditError::new("SNAPSHOT_ASSEMBLY_INCOMPLETE"));
        }
        assembly.completed = true;
        validate_snapshot_privacy(assembly)?;
        validate_chunk_aggregate_metadata(assembly)?;
        if assembly.kind == SnapshotKind::DirectAccess && assembly.is_snapshot {
            validate_direct_access_graph(assembly)?;
        }
        evidence.completed_chunk_streams += 1;
        if is_snapshot {
            evidence.snapshots.push(SnapshotEvidence {
                checkpoint_id,
                source_instance_id: source_instance_id.to_owned(),
                kind,
                config_id,
                reason,
                snapshot_id,
                received_monotonic_timestamp_ms,
                record_index,
            });
        }
    }
    Ok(())
}

fn validate_chunk_data_schema(
    data: &Map<String, Value>,
    kind: SnapshotKind,
    is_snapshot: bool,
) -> AuditResult<()> {
    const COMMON: &[&str] = &[
        "snapshot_id",
        "stream",
        "reason",
        "chunk_index",
        "chunk_count",
        "total_items",
        "items",
        "snapshot_complete",
        "truncated",
        "overflow_safe",
        "observation_items",
    ];
    const BANK_SNAPSHOT_EXTRA: &[&str] = &[
        "config_id",
        "follow_visibility",
        "requested_bank_generation",
        "bank_generation",
        "superseded",
    ];
    const DIRECT_SNAPSHOT_EXTRA: &[&str] = &[
        "base_object_id",
        "observation_epoch",
        "observation_epoch_status",
        "reference_items",
        "cycle_count",
        "shared_reference_count",
        "error_count",
        "truncation_reasons",
    ];
    const FEEDBACK_EXTRA: &[&str] = &[
        "first_observation_seq",
        "last_observation_seq",
        "remaining_items",
    ];
    let extra = match (kind, is_snapshot) {
        (SnapshotKind::Bank, true) => BANK_SNAPSHOT_EXTRA,
        (SnapshotKind::DirectAccess, true) => DIRECT_SNAPSHOT_EXTRA,
        (_, false) => FEEDBACK_EXTRA,
    };
    if data.len() != COMMON.len() + extra.len()
        || COMMON
            .iter()
            .chain(extra)
            .any(|key| !data.contains_key(*key))
    {
        return Err(AuditError::new("SNAPSHOT_CHUNK_SCHEMA_INVALID"));
    }
    let observation_items = required_u64(data, "observation_items")?;
    if observation_items > MAX_SEMANTIC_ITEMS as u64 {
        return Err(AuditError::new("SNAPSHOT_CHUNK_SCHEMA_INVALID"));
    }
    if is_snapshot && kind == SnapshotKind::DirectAccess {
        let reasons = data
            .get("truncation_reasons")
            .and_then(Value::as_array)
            .ok_or_else(|| AuditError::new("DIRECT_ACCESS_CHUNK_METADATA_INVALID"))?;
        let reference_items = required_u64(data, "reference_items")?;
        let cycle_count = required_u64(data, "cycle_count")?;
        let shared_reference_count = required_u64(data, "shared_reference_count")?;
        if data.get("base_object_id").and_then(Value::as_u64).is_none()
            || reference_items > MAX_SEMANTIC_ITEMS as u64
            || cycle_count.checked_add(shared_reference_count) != Some(reference_items)
            || reasons.iter().any(|reason| reason.as_str().is_none())
            || (!required_bool(data, "truncated")? && !reasons.is_empty())
        {
            return Err(AuditError::new("DIRECT_ACCESS_CHUNK_METADATA_INVALID"));
        }
    }
    if !is_snapshot {
        let first = required_u64(data, "first_observation_seq")?;
        let last = required_u64(data, "last_observation_seq")?;
        if first == 0
            || last < first
            || last.saturating_sub(first).saturating_add(1) != observation_items
            || required_u64(data, "remaining_items")? > 512
        {
            return Err(AuditError::new("FEEDBACK_CHUNK_METADATA_INVALID"));
        }
    }
    Ok(())
}

fn validate_chunk_aggregate_metadata(assembly: &RawSnapshotAssembly) -> AuditResult<()> {
    let semantic_items: Vec<_> = assembly
        .items
        .iter()
        .filter_map(|item| item.as_object())
        .filter(|object| {
            object.get("record_kind").and_then(Value::as_str) != Some("host_id_fragment")
        })
        .collect();
    if assembly.kind == SnapshotKind::DirectAccess && assembly.is_snapshot {
        let observations: Vec<_> = semantic_items
            .iter()
            .copied()
            .filter(|object| {
                object.get("record_kind").and_then(Value::as_str) == Some("observation")
            })
            .collect();
        let references = semantic_items
            .iter()
            .filter(|object| {
                object.get("record_kind").and_then(Value::as_str) == Some("object_reference")
            })
            .count();
        if observations.len() != assembly.observation_items
            || assembly.direct_access_reference_items != Some(references)
            || observations.len().saturating_add(references) != semantic_items.len()
        {
            return Err(AuditError::new("SNAPSHOT_OBSERVATION_COUNT_INVALID"));
        }
        let metadata_errors = observations.iter().try_fold(0_u64, |sum, item| {
            let count = required_u64(item, "metadata_error_count")?;
            sum.checked_add(count)
                .ok_or_else(|| AuditError::new("DIRECT_ACCESS_CHUNK_METADATA_INVALID"))
        })?;
        if assembly.direct_access_error_count != Some(metadata_errors) {
            return Err(AuditError::new("DIRECT_ACCESS_CHUNK_METADATA_INVALID"));
        }
    } else {
        if semantic_items.len() != assembly.observation_items {
            return Err(AuditError::new("SNAPSHOT_OBSERVATION_COUNT_INVALID"));
        }
    }
    if !assembly.is_snapshot {
        let first = semantic_items
            .first()
            .and_then(|item| item.get("observation_seq"))
            .and_then(Value::as_u64);
        let last = semantic_items
            .last()
            .and_then(|item| item.get("observation_seq"))
            .and_then(Value::as_u64);
        if first != assembly.first_observation_seq || last != assembly.last_observation_seq {
            return Err(AuditError::new("FEEDBACK_CHUNK_METADATA_INVALID"));
        }
    }
    Ok(())
}

fn direct_access_reference_kind(value: &str) -> Option<DirectAccessReferenceKind> {
    match value {
        "ancestor_cycle" => Some(DirectAccessReferenceKind::AncestorCycle),
        "shared_reference" => Some(DirectAccessReferenceKind::SharedReference),
        _ => None,
    }
}

struct DirectAccessDfsFrame {
    object_id: u64,
    depth: u64,
    child_count: u64,
    next_child_index: u64,
}

fn validate_direct_access_graph(assembly: &RawSnapshotAssembly) -> AuditResult<()> {
    let base_object_id = assembly
        .direct_access_base_object_id
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
    let semantic_objects: Vec<_> = assembly
        .items
        .iter()
        .filter_map(Value::as_object)
        .filter(|object| {
            object.get("record_kind").and_then(Value::as_str) != Some("host_id_fragment")
        })
        .collect();
    if base_object_id > MAX_DIRECT_ACCESS_OBJECT_ID
        || semantic_objects.is_empty()
        || semantic_objects.len() > 256
    {
        return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
    }

    let root = semantic_objects[0];
    let root_child_count = root
        .get("child_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
    if root.get("record_kind").and_then(Value::as_str) != Some("observation")
        || root.get("object_id").and_then(Value::as_u64) != Some(base_object_id)
        || !root.get("parent_id").is_some_and(Value::is_null)
        || root.get("depth").and_then(Value::as_u64) != Some(0)
        || !root.get("child_index").is_some_and(Value::is_null)
        || root_child_count > 128
        || !root
            .get("child_enumeration_error")
            .is_some_and(Value::is_null)
    {
        return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
    }

    let mut seen_observations = vec![base_object_id];
    let mut seen_indices = HashMap::from([(base_object_id, 0usize)]);
    let mut active = vec![DirectAccessDfsFrame {
        object_id: base_object_id,
        depth: 0,
        child_count: root_child_count,
        next_child_index: 0,
    }];
    let mut reference_count = 0_u64;
    let mut cycle_count = 0_u64;
    let mut shared_reference_count = 0_u64;
    for object in semantic_objects.into_iter().skip(1) {
        while active
            .last()
            .is_some_and(|frame| frame.next_child_index == frame.child_count)
        {
            active.pop();
        }
        let parent = active
            .last_mut()
            .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
        let parent_id = required_u64(object, "parent_id")?;
        let depth = required_u64(object, "depth")?;
        let child_index = required_u64(object, "child_index")?;
        if parent_id > MAX_DIRECT_ACCESS_OBJECT_ID
            || parent_id != parent.object_id
            || child_index != parent.next_child_index
            || parent.depth.checked_add(1) != Some(depth)
            || depth > 32
        {
            return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
        }
        parent.next_child_index = parent
            .next_child_index
            .checked_add(1)
            .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;

        let object_id = required_u64(object, "object_id")?;
        if object_id > MAX_DIRECT_ACCESS_OBJECT_ID {
            return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
        }
        match object.get("record_kind").and_then(Value::as_str) {
            Some("observation") => {
                let child_count = object
                    .get("child_count")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                if child_count > 128
                    || !object
                        .get("child_enumeration_error")
                        .is_some_and(Value::is_null)
                    || seen_indices.contains_key(&object_id)
                {
                    return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
                }
                let observation_index = seen_observations.len();
                seen_observations.push(object_id);
                seen_indices.insert(object_id, observation_index);
                active.push(DirectAccessDfsFrame {
                    object_id,
                    depth,
                    child_count,
                    next_child_index: 0,
                });
            }
            Some("object_reference") => {
                reference_count = reference_count
                    .checked_add(1)
                    .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                let target_observation_index =
                    usize::try_from(required_u64(object, "target_observation_index")?)
                        .map_err(|_| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                if seen_observations.get(target_observation_index).copied() != Some(object_id)
                    || seen_indices.get(&object_id).copied() != Some(target_observation_index)
                {
                    return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
                }
                let target_is_ancestor = active.iter().any(|frame| frame.object_id == object_id);
                let reference_kind = object
                    .get("reference_kind")
                    .and_then(Value::as_str)
                    .and_then(direct_access_reference_kind)
                    .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                match (reference_kind, target_is_ancestor) {
                    (DirectAccessReferenceKind::AncestorCycle, true) => {
                        cycle_count = cycle_count
                            .checked_add(1)
                            .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                    }
                    (DirectAccessReferenceKind::SharedReference, false) => {
                        shared_reference_count = shared_reference_count
                            .checked_add(1)
                            .ok_or_else(|| AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"))?;
                    }
                    _ => return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID")),
                }
            }
            _ => {
                return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
            }
        }
    }

    while active
        .last()
        .is_some_and(|frame| frame.next_child_index == frame.child_count)
    {
        active.pop();
    }
    if assembly.direct_access_reference_items != usize::try_from(reference_count).ok()
        || assembly.direct_access_cycle_count != Some(cycle_count)
        || assembly.direct_access_shared_reference_count != Some(shared_reference_count)
        || seen_observations.len() != assembly.observation_items
        || !active.is_empty()
    {
        return Err(AuditError::new("DIRECT_ACCESS_GRAPH_INVALID"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct AuthorizedHostIdRef {
    fragment_count: usize,
    byte_length: usize,
}

fn validate_snapshot_privacy(assembly: &RawSnapshotAssembly) -> AuditResult<()> {
    if assembly.items.len() != assembly.item_positions.len() {
        return Err(AuditError::new("SNAPSHOT_ITEM_POSITION_INVALID"));
    }
    let mut authorized_refs = HashMap::new();
    for item in &assembly.items {
        let object = item
            .as_object()
            .ok_or_else(|| AuditError::new("SNAPSHOT_PRIVACY_INVALID"))?;
        if object.get("record_kind").and_then(Value::as_str) == Some("host_id_fragment") {
            continue;
        }
        match (assembly.kind, assembly.is_snapshot) {
            (SnapshotKind::Bank, _) => {
                validate_bank_observation_privacy(object, assembly, &mut authorized_refs)?
            }
            (SnapshotKind::DirectAccess, true) => {
                match object.get("record_kind").and_then(Value::as_str) {
                    Some("observation") => validate_direct_access_observation_privacy(
                        object,
                        assembly,
                        &mut authorized_refs,
                    )?,
                    Some("object_reference") => {
                        validate_direct_access_reference_privacy(object, assembly)?
                    }
                    _ => return Err(AuditError::new("DIRECT_ACCESS_RECORD_KIND_INVALID")),
                }
            }
            (SnapshotKind::DirectAccess, false) => validate_direct_access_feedback_privacy(object)?,
        }
    }
    validate_host_id_fragments(&assembly.items, &authorized_refs)
}

fn validate_bank_observation_privacy(
    object: &Map<String, Value>,
    assembly: &RawSnapshotAssembly,
    authorized_refs: &mut HashMap<String, AuthorizedHostIdRef>,
) -> AuditResult<()> {
    const BASE_KEYS: &[&str] = &[
        "record_kind",
        "config_id",
        "bank_generation",
        "slot_index",
        "title",
        "title_redacted",
        "selected",
        "mute",
        "solo",
        "host_id_raw",
        "host_id_redacted",
        "host_id_observed_with_title_callback",
        "host_id_observation_status",
        "redacted_string_count",
        "field_observation_generation",
        "field_last_observation_seq",
        "field_last_observation_epoch",
        "host_id_byte_length",
        "host_id_ref",
        "host_id_fragment_count",
    ];
    const FEEDBACK_KEYS: &[&str] = &[
        "observation_seq",
        "observation_epoch",
        "observation_epoch_status",
        "changed_field",
        "changed_value",
        "changed_value_redacted",
        "callback_source",
    ];
    if object.get("record_kind").and_then(Value::as_str) != Some("observation")
        || !object.keys().all(|key| {
            BASE_KEYS.contains(&key.as_str())
                || (!assembly.is_snapshot && FEEDBACK_KEYS.contains(&key.as_str()))
        })
        || BASE_KEYS.iter().any(|key| !object.contains_key(*key))
        || required_u64(object, "slot_index")? >= 8
    {
        return Err(AuditError::new("BANK_OBSERVATION_PRIVACY_INVALID"));
    }
    let config_id = required_string(object, "config_id", 32)?;
    if !matches!(config_id, "MB_CORE_ALL" | "MB_CORE_VISIBLE")
        || (assembly.is_snapshot && assembly.config_id.as_deref() != Some(config_id))
    {
        return Err(AuditError::new("BANK_OBSERVATION_PRIVACY_INVALID"));
    }
    let generation = required_u64(object, "bank_generation")?;
    if assembly.is_snapshot && assembly.bank_generation != Some(generation) {
        return Err(AuditError::new("BANK_GENERATION_MISMATCH"));
    }
    for key in ["selected", "mute", "solo"] {
        if !is_nullable_bool(object.get(key)) {
            return Err(AuditError::new("BANK_OBSERVATION_PRIVACY_INVALID"));
        }
    }
    let title_redacted = required_bool(object, "title_redacted")?;
    validate_redacted_title(object.get("title"), title_redacted)?;
    let host_id_redacted = required_bool(object, "host_id_redacted")?;
    let redacted_string_count = required_u64(object, "redacted_string_count")?;
    if redacted_string_count != u64::from(title_redacted) + u64::from(host_id_redacted)
        || redacted_string_count > 2
        || (host_id_redacted && !title_redacted)
    {
        return Err(AuditError::new("BANK_REDACTION_COUNT_INVALID"));
    }
    let authorized_title = object
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|title| !title.is_empty() && fixture_title_allowed(title));
    validate_host_id_wire(
        object,
        authorized_title,
        title_redacted,
        host_id_redacted,
        authorized_refs,
    )?;

    let observation_generations = required_object(object, "field_observation_generation")?;
    let last_sequences = required_object(object, "field_last_observation_seq")?;
    let last_epochs = required_object(object, "field_last_observation_epoch")?;
    const FIELDS: [&str; 5] = ["title", "selected", "mute", "solo", "host_id_raw"];
    if observation_generations.len() != FIELDS.len()
        || last_sequences.len() != FIELDS.len()
        || last_epochs.len() != FIELDS.len()
        || FIELDS.iter().any(|field| {
            !observation_generations.contains_key(*field)
                || !last_sequences.contains_key(*field)
                || !last_epochs.contains_key(*field)
        })
    {
        return Err(AuditError::new("BANK_FIELD_METADATA_INVALID"));
    }
    for field in FIELDS {
        let observed_generation = observation_generations
            .get(field)
            .and_then(Value::as_i64)
            .ok_or_else(|| AuditError::new("BANK_FIELD_METADATA_INVALID"))?;
        if observed_generation < -1
            || observed_generation > i64::try_from(generation).unwrap_or(i64::MAX)
            || !is_nullable_u64(last_sequences.get(field))
            || !is_nullable_u64(last_epochs.get(field))
        {
            return Err(AuditError::new("BANK_FIELD_METADATA_INVALID"));
        }
        if object.get(field).is_some_and(|value| !value.is_null())
            && observed_generation != i64::try_from(generation).unwrap_or(i64::MAX)
        {
            return Err(AuditError::new("BANK_FIELD_GENERATION_STALE"));
        }
    }

    let observed_with_title_callback =
        required_bool(object, "host_id_observed_with_title_callback")?;
    let observation_status = required_string(object, "host_id_observation_status", 40)?;
    if !matches!(
        observation_status,
        "not_observed"
            | "title_not_authorized"
            | "mapping_unavailable"
            | "getter_unavailable"
            | "invalid_type"
            | "getter_failed"
            | "observed_with_title_callback"
    ) || observed_with_title_callback != (observation_status == "observed_with_title_callback")
        || (observed_with_title_callback && !authorized_title)
        || (observed_with_title_callback
            && object.get("host_id_raw").is_none_or(Value::is_null)
            && object.get("host_id_ref").is_none_or(Value::is_null))
        || (!observed_with_title_callback
            && (!object.get("host_id_raw").is_none_or(Value::is_null)
                || !object.get("host_id_ref").is_none_or(Value::is_null)))
    {
        return Err(AuditError::new("BANK_HOST_ID_OBSERVATION_INVALID"));
    }
    let title_sequence = last_sequences.get("title").and_then(Value::as_u64);
    let host_sequence = last_sequences.get("host_id_raw").and_then(Value::as_u64);
    if (observed_with_title_callback
        && (host_sequence.is_none() || host_sequence != title_sequence))
        || (!observed_with_title_callback && host_sequence.is_some())
    {
        return Err(AuditError::new("BANK_HOST_ID_OBSERVATION_INVALID"));
    }

    if !assembly.is_snapshot {
        if FEEDBACK_KEYS.iter().any(|key| !object.contains_key(*key))
            || required_u64(object, "observation_seq")? == 0
            || object
                .get("observation_epoch_status")
                .and_then(Value::as_str)
                != Some("callback_observed")
        {
            return Err(AuditError::new("BANK_FEEDBACK_PRIVACY_INVALID"));
        }
        let changed_field = required_string(object, "changed_field", 16)?;
        let observation_seq = required_u64(object, "observation_seq")?;
        let callback_epoch = required_u64(object, "observation_epoch")?;
        if last_sequences.get(changed_field).and_then(Value::as_u64) != Some(observation_seq)
            || last_epochs.get(changed_field).and_then(Value::as_u64) != Some(callback_epoch)
            || (changed_field == "title"
                && observed_with_title_callback
                && (last_sequences.get("host_id_raw").and_then(Value::as_u64)
                    != Some(observation_seq)
                    || last_epochs.get("host_id_raw").and_then(Value::as_u64)
                        != Some(callback_epoch)))
        {
            return Err(AuditError::new("BANK_FEEDBACK_EPOCH_INVALID"));
        }
        let callback_source = required_string(object, "callback_source", 32)?;
        let callback_source_valid = match changed_field {
            "title" => matches!(callback_source, "mixer_bank_channel" | "selected_binding"),
            "selected" => callback_source == "selected_binding",
            "mute" => callback_source == "mute_binding",
            "solo" => callback_source == "solo_binding",
            _ => false,
        };
        if !matches!(changed_field, "title" | "selected" | "mute" | "solo")
            || required_bool(object, "changed_value_redacted")?
                != (changed_field == "title" && title_redacted)
            || !callback_source_valid
        {
            return Err(AuditError::new("BANK_FEEDBACK_PRIVACY_INVALID"));
        }
        let changed_value = object
            .get("changed_value")
            .ok_or_else(|| AuditError::new("BANK_FEEDBACK_PRIVACY_INVALID"))?;
        let changed_value_valid = match changed_field {
            "title" => {
                changed_value.is_null() || changed_value.as_str().is_some_and(fixture_title_allowed)
            }
            _ => changed_value.is_null() || changed_value.is_boolean(),
        };
        if !changed_value_valid {
            return Err(AuditError::new("BANK_FEEDBACK_PRIVACY_INVALID"));
        }
    }
    Ok(())
}

fn validate_direct_access_observation_privacy(
    object: &Map<String, Value>,
    assembly: &RawSnapshotAssembly,
    authorized_refs: &mut HashMap<String, AuthorizedHostIdRef>,
) -> AuditResult<()> {
    const KEYS: &[&str] = &[
        "record_kind",
        "observation_epoch",
        "observation_epoch_status",
        "object_id",
        "parent_id",
        "depth",
        "child_index",
        "unique_name",
        "unique_name_redacted",
        "unique_name_status",
        "host_id_raw",
        "host_id_redacted",
        "host_id_byte_length",
        "host_id_ref",
        "host_id_fragment_count",
        "title",
        "title_redacted",
        "type_name",
        "type_name_redacted",
        "mixer_visible",
        "mixer_index",
        "mixer_zone",
        "child_count",
        "child_enumeration_error",
        "metadata_error_count",
        "metadata_errors",
        "redacted_string_count",
    ];
    if object.get("record_kind").and_then(Value::as_str) != Some("observation")
        || object.keys().any(|key| !KEYS.contains(&key.as_str()))
        || KEYS.iter().any(|key| !object.contains_key(*key))
        || object
            .get("unique_name")
            .is_none_or(|value| !value.is_null())
        || object.get("object_id").and_then(Value::as_u64).is_none()
        || object
            .get("observation_epoch_status")
            .and_then(Value::as_str)
            != Some("snapshot_observed")
        || object.get("observation_epoch").and_then(Value::as_u64) != assembly.observation_epoch
        || !is_nullable_u64(object.get("parent_id"))
        || !is_nullable_u64(object.get("depth"))
        || !is_nullable_u64(object.get("child_index"))
        || !is_nullable_bool(object.get("mixer_visible"))
        || !is_nullable_number(object.get("mixer_index"))
        || !is_nullable_number(object.get("mixer_zone"))
        || !is_nullable_u64(object.get("child_count"))
    {
        return Err(AuditError::new("DIRECT_ACCESS_OBSERVATION_PRIVACY_INVALID"));
    }
    let unique_name_redacted = required_bool(object, "unique_name_redacted")?;
    if object.get("unique_name_status").and_then(Value::as_str)
        != Some(if unique_name_redacted {
            "not_invoked_by_policy"
        } else {
            "not_available"
        })
    {
        return Err(AuditError::new("DIRECT_ACCESS_UNIQUE_NAME_POLICY_INVALID"));
    }
    let title_redacted = required_bool(object, "title_redacted")?;
    validate_redacted_title(object.get("title"), title_redacted)?;
    let type_name_redacted = required_bool(object, "type_name_redacted")?;
    match object.get("type_name") {
        Some(Value::Null) => {}
        Some(Value::String(value))
            if !type_name_redacted && safe_direct_access_type_name(value) => {}
        _ => return Err(AuditError::new("DIRECT_ACCESS_TYPE_PRIVACY_INVALID")),
    }
    let host_id_redacted = required_bool(object, "host_id_redacted")?;
    if host_id_redacted && !title_redacted {
        return Err(AuditError::new("DIRECT_ACCESS_HOST_ID_PRIVACY_INVALID"));
    }
    let authorized_title = object
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|title| !title.is_empty() && fixture_title_allowed(title));
    validate_host_id_wire(
        object,
        authorized_title,
        title_redacted,
        host_id_redacted,
        authorized_refs,
    )?;
    let redacted_count = required_u64(object, "redacted_string_count")?;
    let expected_count = u64::from(unique_name_redacted)
        + u64::from(host_id_redacted)
        + u64::from(title_redacted)
        + u64::from(type_name_redacted);
    if redacted_count != expected_count || redacted_count > 4 {
        return Err(AuditError::new("DIRECT_ACCESS_REDACTION_COUNT_INVALID"));
    }
    let errors = object
        .get("metadata_errors")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_ERROR_CODE_INVALID"))?;
    if required_u64(object, "metadata_error_count")? != errors.len() as u64
        || errors.iter().any(|error| {
            error
                .as_str()
                .is_none_or(|code| !safe_direct_access_metadata_error(code))
        })
    {
        return Err(AuditError::new("DIRECT_ACCESS_ERROR_CODE_INVALID"));
    }
    if object.get("child_enumeration_error").is_none_or(|value| {
        !(value.is_null()
            || matches!(
                value.as_str(),
                Some("get_number_of_child_objects_failed" | "invalid_child_count")
            ))
    }) {
        return Err(AuditError::new("DIRECT_ACCESS_ERROR_CODE_INVALID"));
    }
    Ok(())
}

fn validate_direct_access_reference_privacy(
    object: &Map<String, Value>,
    assembly: &RawSnapshotAssembly,
) -> AuditResult<()> {
    const KEYS: [&str; 9] = [
        "record_kind",
        "observation_epoch",
        "observation_epoch_status",
        "object_id",
        "parent_id",
        "depth",
        "child_index",
        "target_observation_index",
        "reference_kind",
    ];
    if object.len() != KEYS.len()
        || KEYS.iter().any(|key| !object.contains_key(*key))
        || object.get("record_kind").and_then(Value::as_str) != Some("object_reference")
        || object.get("observation_epoch").and_then(Value::as_u64) != assembly.observation_epoch
        || object
            .get("observation_epoch_status")
            .and_then(Value::as_str)
            != Some("snapshot_observed")
        || object.get("object_id").and_then(Value::as_u64).is_none()
        || object.get("parent_id").and_then(Value::as_u64).is_none()
        || object.get("depth").and_then(Value::as_u64).is_none()
        || object.get("child_index").and_then(Value::as_u64).is_none()
        || object
            .get("target_observation_index")
            .and_then(Value::as_u64)
            .is_none()
        || !matches!(
            object.get("reference_kind").and_then(Value::as_str),
            Some("ancestor_cycle" | "shared_reference")
        )
    {
        return Err(AuditError::new("DIRECT_ACCESS_REFERENCE_PRIVACY_INVALID"));
    }
    Ok(())
}

fn validate_direct_access_feedback_privacy(object: &Map<String, Value>) -> AuditResult<()> {
    const KEYS: [&str; 6] = [
        "observation_seq",
        "observation_epoch",
        "observation_epoch_status",
        "change",
        "object_id",
        "parameter_tag",
    ];
    if object.len() != KEYS.len()
        || KEYS.iter().any(|key| !object.contains_key(*key))
        || required_u64(object, "observation_seq")? == 0
        || object
            .get("observation_epoch")
            .and_then(Value::as_u64)
            .is_none()
        || object
            .get("observation_epoch_status")
            .and_then(Value::as_str)
            != Some("callback_observed")
        || !matches!(
            required_string(object, "change", 32)?,
            "object_change" | "object_will_be_removed" | "parameter_change"
        )
        || !is_nullable_number(object.get("object_id"))
        || !is_nullable_number(object.get("parameter_tag"))
    {
        return Err(AuditError::new("DIRECT_ACCESS_FEEDBACK_PRIVACY_INVALID"));
    }
    Ok(())
}

fn validate_redacted_title(value: Option<&Value>, redacted: bool) -> AuditResult<()> {
    match value {
        Some(Value::Null) => Ok(()),
        Some(Value::String(title)) if title.is_empty() && !redacted => Ok(()),
        Some(Value::String(title)) if fixture_title_allowed(title) && !redacted => Ok(()),
        _ => Err(AuditError::new("TITLE_PRIVACY_INVALID")),
    }
}

fn validate_host_id_wire(
    object: &Map<String, Value>,
    authorized_title: bool,
    title_redacted: bool,
    host_id_redacted: bool,
    authorized_refs: &mut HashMap<String, AuthorizedHostIdRef>,
) -> AuditResult<()> {
    let raw = object
        .get("host_id_raw")
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    let reference = object
        .get("host_id_ref")
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    let byte_length = object
        .get("host_id_byte_length")
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    let fragment_count = required_u64(object, "host_id_fragment_count")?;
    if host_id_redacted {
        if !title_redacted
            || !raw.is_null()
            || !reference.is_null()
            || !byte_length.is_null()
            || fragment_count != 0
        {
            return Err(AuditError::new("HOST_ID_PRIVACY_INVALID"));
        }
        return Ok(());
    }
    if let Some(raw) = raw.as_str() {
        if !authorized_title
            || !reference.is_null()
            || fragment_count != 0
            || byte_length.as_u64() != Some(raw.len() as u64)
            || raw.len() > MAX_HOST_ID_BYTES
        {
            return Err(AuditError::new("HOST_ID_PRIVACY_INVALID"));
        }
        return Ok(());
    }
    if !raw.is_null() {
        return Err(AuditError::new("HOST_ID_PRIVACY_INVALID"));
    }
    if reference.is_null() {
        if !byte_length.is_null() || fragment_count != 0 {
            return Err(AuditError::new("HOST_ID_PRIVACY_INVALID"));
        }
        return Ok(());
    }
    let reference = reference
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    let fragment_count = usize::try_from(fragment_count)
        .ok()
        .filter(|count| (1..=MAX_HOST_ID_FRAGMENTS).contains(count))
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    let byte_length = byte_length
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|length| *length <= MAX_HOST_ID_BYTES)
        .ok_or_else(|| AuditError::new("HOST_ID_PRIVACY_INVALID"))?;
    if !authorized_title
        || authorized_refs
            .insert(
                reference.to_owned(),
                AuthorizedHostIdRef {
                    fragment_count,
                    byte_length,
                },
            )
            .is_some()
    {
        return Err(AuditError::new("HOST_ID_PRIVACY_INVALID"));
    }
    Ok(())
}

fn validate_host_id_fragments(
    items: &[Value],
    authorized_refs: &HashMap<String, AuthorizedHostIdRef>,
) -> AuditResult<()> {
    let mut fragments: HashMap<&str, Vec<Option<&str>>> = authorized_refs
        .iter()
        .map(|(reference, metadata)| (reference.as_str(), vec![None; metadata.fragment_count]))
        .collect();
    for item in items {
        let Some(object) = item.as_object() else {
            return Err(AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"));
        };
        if object.get("record_kind").and_then(Value::as_str) != Some("host_id_fragment") {
            continue;
        }
        const KEYS: [&str; 6] = [
            "record_kind",
            "host_id_ref",
            "host_id_byte_length",
            "fragment_index",
            "fragment_count",
            "fragment",
        ];
        if object.len() != KEYS.len() || KEYS.iter().any(|key| !object.contains_key(*key)) {
            return Err(AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"));
        }
        let reference = required_string(object, "host_id_ref", 256)?;
        let metadata = authorized_refs
            .get(reference)
            .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?;
        let index = required_u64(object, "fragment_index")? as usize;
        let slots = fragments
            .get_mut(reference)
            .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?;
        let fragment = object
            .get("fragment")
            .and_then(Value::as_str)
            .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?;
        if index >= slots.len()
            || slots[index].is_some()
            || required_u64(object, "fragment_count")? != metadata.fragment_count as u64
            || required_u64(object, "host_id_byte_length")? != metadata.byte_length as u64
        {
            return Err(AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"));
        }
        slots[index] = Some(fragment);
    }
    for (reference, slots) in fragments {
        let metadata = authorized_refs
            .get(reference)
            .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?;
        let mut length = 0usize;
        for fragment in slots {
            length = length
                .checked_add(
                    fragment
                        .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?
                        .len(),
                )
                .ok_or_else(|| AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"))?;
        }
        if length != metadata.byte_length {
            return Err(AuditError::new("HOST_ID_FRAGMENT_PRIVACY_INVALID"));
        }
    }
    Ok(())
}

fn is_nullable_bool(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || value.is_boolean())
}

fn is_nullable_u64(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || value.as_u64().is_some())
}

fn is_nullable_number(value: Option<&Value>) -> bool {
    value.is_some_and(|value| value.is_null() || value.as_f64().is_some())
}

fn safe_direct_access_type_name(value: &str) -> bool {
    matches!(
        value,
        "MixConsole"
            | "AudioChannel"
            | "MIDIChannel"
            | "InstrumentChannel"
            | "GroupChannel"
            | "FXChannel"
            | "FolderTrack"
    )
}

fn safe_direct_access_metadata_error(value: &str) -> bool {
    matches!(
        value,
        "getObjectTitle_invalid_string"
            | "getObjectTitle_failed"
            | "getObjectUniqueIDString_invalid_string"
            | "getObjectUniqueIDString_failed"
            | "getObjectTypeName_invalid_string"
            | "getObjectTypeName_failed"
            | "isMixerChannelVisible_invalid_boolean"
            | "isMixerChannelVisible_failed"
            | "getMixerChannelIndex_invalid_number"
            | "getMixerChannelIndex_failed"
            | "getMixerChannelZone_invalid_number"
            | "getMixerChannelZone_failed"
    )
}

fn validate_lifecycle_metadata(
    data: &Map<String, Value>,
    source: &str,
    lifecycle: &str,
) -> AuditResult<()> {
    let exact_keys = match lifecycle {
        "loaded" | "mapping_active" => has_exact_keys(
            data,
            &[
                "probe_session_id",
                "mapping_active",
                "read_only",
                "protocol_version",
            ],
        ),
        "ready" | "not_ready" => has_exact_keys(
            data,
            &[
                "probe_session_id",
                "ready",
                "initial_snapshots_complete",
                "read_only",
                "protocol_version",
            ],
        ),
        _ => false,
    };
    if !exact_keys
        || data.get("probe_session_id").and_then(Value::as_str) != Some(source)
        || data.get("read_only").and_then(Value::as_bool) != Some(true)
        || data.get("protocol_version").and_then(Value::as_u64) != Some(1)
        || (matches!(lifecycle, "loaded" | "mapping_active")
            && data.get("mapping_active").and_then(Value::as_bool) != Some(true))
        || (lifecycle == "ready" && data.get("ready").and_then(Value::as_bool) != Some(true))
        || (lifecycle == "not_ready" && data.get("ready").and_then(Value::as_bool) != Some(false))
    {
        return Err(AuditError::new("LIFECYCLE_METADATA_INVALID"));
    }
    Ok(())
}

fn observe_source_sequence(
    evidence: &mut Evidence,
    source: &str,
    source_seq: u64,
    is_loaded: bool,
) -> AuditResult<()> {
    match evidence.last_source_seq.get(source).copied() {
        None if source_seq == 1 && is_loaded => {}
        None => return Err(AuditError::new("SOURCE_SESSION_START_INVALID")),
        Some(previous) if previous.checked_add(1) == Some(source_seq) && !is_loaded => {}
        Some(_) => return Err(AuditError::new("SOURCE_SEQUENCE_INVALID")),
    }
    evidence.probe_source_ids.insert(source.to_owned());
    evidence
        .last_source_seq
        .insert(source.to_owned(), source_seq);
    Ok(())
}

fn collect_discovery(
    record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    if !has_exact_record_keys(
        object,
        &[
            "request_id",
            "checkpoint_id",
            "responder_count",
            "source_instance_ids",
            "observed_source_instance_ids",
            "selected_source_instance_id",
            "outcome",
            "window_closed",
        ],
    ) {
        return Err(AuditError::new("DISCOVERY_RECORD_SCHEMA_INVALID"));
    }
    let checkpoint_id = canonical_checkpoint_id(required_string(object, "checkpoint_id", 128)?)
        .ok_or_else(|| AuditError::new("DISCOVERY_CHECKPOINT_INVALID"))?;
    let request_id = required_string(object, "request_id", 128)?.to_owned();
    if required_u64(object, "responder_count")? != 1
        || required_string(object, "outcome", 32)? != "selected"
        || !required_bool(object, "window_closed")?
    {
        return Err(AuditError::new("DISCOVERY_RESULT_INVALID"));
    }
    let selected = required_string(object, "selected_source_instance_id", 256)?.to_owned();
    let source_ids = object
        .get("source_instance_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("DISCOVERY_RESULT_INVALID"))?;
    let observed_ids = object
        .get("observed_source_instance_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("DISCOVERY_RESULT_INVALID"))?;
    if source_ids.len() != 1
        || source_ids[0].as_str() != Some(&selected)
        || observed_ids.len() != 1
        || observed_ids[0].as_str() != Some(&selected)
    {
        return Err(AuditError::new("DISCOVERY_RESULT_INVALID"));
    }
    if evidence
        .discoveries
        .insert(
            request_id,
            DiscoveryEvidence {
                checkpoint_id,
                selected_source_instance_id: selected,
                monotonic_timestamp_ms: record.monotonic_timestamp_ms,
            },
        )
        .is_some()
    {
        return Err(AuditError::new("DISCOVERY_RESULT_DUPLICATE"));
    }
    Ok(())
}

fn collect_summary(
    _record: &RawRecord,
    object: &Map<String, Value>,
    evidence: &mut Evidence,
) -> AuditResult<()> {
    evidence.summary_count += 1;
    if !has_exact_record_keys(
        object,
        &[
            "session_id",
            "integrity_ok",
            "exit_ok",
            "exit_reason",
            "commands",
            "graceful_drain",
            "orphan_messages",
            "protocol_tracking",
            "incoming",
        ],
    ) {
        return Err(AuditError::new("COLLECTOR_SUMMARY_SCHEMA_INVALID"));
    }
    if evidence
        .summary
        .replace(Value::Object(object.clone()))
        .is_some()
    {
        return Err(AuditError::new("COLLECTOR_SUMMARY_DUPLICATE"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ExpectedCommand {
    method: &'static str,
    config_id: Option<&'static str>,
}

fn expected_commands_with_activation(
    checkpoint_id: &'static str,
    direct_access_required: bool,
    same_script_reactivated: bool,
) -> Vec<ExpectedCommand> {
    let mut commands = Vec::new();
    if checkpoint_id == "INIT" {
        commands.push(ExpectedCommand {
            method: "probe.discover",
            config_id: None,
        });
        commands.push(ExpectedCommand {
            method: "probe.capabilities.get",
            config_id: None,
        });
    } else if matches!(checkpoint_id, "R1" | "R2") {
        commands.push(ExpectedCommand {
            method: "probe.discover",
            config_id: None,
        });
    }
    if requires_observation_cut(checkpoint_id) {
        commands.push(ExpectedCommand {
            method: "probe.observation.cut",
            config_id: None,
        });
    }
    let navigation = bank_operation(checkpoint_id);
    if same_script_reactivated && navigation.is_none() {
        commands.push(ExpectedCommand {
            method: "probe.discover",
            config_id: None,
        });
    }

    if let Some((config_id, operation)) = navigation {
        commands.push(ExpectedCommand {
            method: operation,
            config_id: Some(config_id),
        });
        if same_script_reactivated {
            commands.push(ExpectedCommand {
                method: "probe.discover",
                config_id: None,
            });
        }
        if direct_access_required {
            commands.push(ExpectedCommand {
                method: "probe.direct_access.snapshot",
                config_id: None,
            });
        }
        commands.push(ExpectedCommand {
            method: "probe.bank.snapshot",
            config_id: Some(config_id),
        });
    } else {
        if direct_access_required {
            commands.push(ExpectedCommand {
                method: "probe.direct_access.snapshot",
                config_id: None,
            });
        }
        commands.push(ExpectedCommand {
            method: "probe.bank.snapshot",
            config_id: Some("MB_CORE_ALL"),
        });
        commands.push(ExpectedCommand {
            method: "probe.bank.snapshot",
            config_id: Some("MB_CORE_VISIBLE"),
        });
    }
    commands
}

fn requires_observation_cut(checkpoint_id: &str) -> bool {
    matches!(checkpoint_id, "E0" | "E1" | "E8" | "C1") || checkpoint_id.starts_with('S')
}

fn bank_operation(checkpoint_id: &str) -> Option<(&'static str, &'static str)> {
    let config = if checkpoint_id.contains("-MB_CORE_ALL-") {
        "MB_CORE_ALL"
    } else if checkpoint_id.contains("-MB_CORE_VISIBLE-") {
        "MB_CORE_VISIBLE"
    } else {
        return None;
    };
    let method = if checkpoint_id.ends_with("reset") {
        "probe.bank.reset"
    } else if checkpoint_id.ends_with("next") {
        "probe.bank.next"
    } else if checkpoint_id.ends_with("prev") {
        "probe.bank.prev"
    } else {
        return None;
    };
    Some((config, method))
}

fn validate_evidence(
    manifest: &AuditManifest,
    records: &[RawRecord],
    evidence: &Evidence,
    evidence_sha256: EvidenceDigests,
) -> AuditResult<AuditReport> {
    if evidence.start_count != 1 || evidence.summary_count != 1 {
        return Err(AuditError::new("COLLECTOR_BOUNDARY_RECORD_INVALID"));
    }
    if records.first().map(|record| record.record_type.as_str()) != Some("collector_started")
        || records.last().map(|record| record.record_type.as_str()) != Some("collector_summary")
    {
        return Err(AuditError::new("COLLECTOR_BOUNDARY_ORDER_INVALID"));
    }
    if evidence.commands.is_empty() {
        return Err(AuditError::new("NO_PROBE_COMMANDS"));
    }
    if evidence
        .raw_snapshots
        .values()
        .any(|snapshot| !snapshot.completed)
    {
        return Err(AuditError::new("SNAPSHOT_ASSEMBLY_INCOMPLETE"));
    }
    validate_probe_lifecycle_replay(records)?;
    validate_initial_capability_epoch_binding(evidence)?;
    let direct_access_required = observed_direct_access(manifest.profile, evidence)?;
    let capabilities = capability_report(initial_capability_result(evidence)?)?;
    validate_observation_capability_consistency(evidence, &capabilities)?;
    validate_checkpoints(evidence)?;
    validate_commands(manifest.profile, direct_access_required, evidence)?;
    validate_feedback_epoch_sequences(evidence)?;
    validate_drain_boundary(records, evidence)?;
    validate_summary(evidence)?;
    if !direct_access_required && evidence.direct_access_event_observed {
        return Err(AuditError::new("DIRECT_ACCESS_UNEXPECTED_FOR_C13"));
    }
    let reconnects = validate_reconnects(manifest.profile, direct_access_required, evidence)?;
    let projections = build_semantic_projections(
        direct_access_required,
        &manifest.fixture_acceptance,
        evidence,
    )?;
    Ok(AuditReport {
        audit_report_version: AUDIT_REPORT_VERSION,
        status: "evidence_valid",
        semantic_assessment: "observed_not_evaluated",
        profile: manifest.profile,
        fixture_revision: manifest.fixture_revision,
        run_alias: safe_run_alias(&manifest.run_id),
        run_started_at: manifest.run_started_at.clone(),
        environment: manifest.environment.clone(),
        mixconsole: manifest.mixconsole.clone(),
        filters: manifest.filters.clone(),
        fixture_acceptance: fixture_acceptance_report(&manifest.fixture_acceptance),
        capabilities,
        evidence_sha256,
        artifact_digests_match: true,
        record_count: records.len(),
        checkpoint_count: evidence.checkpoints.len(),
        probe_command_count: evidence.commands.len(),
        completed_snapshot_count: evidence.snapshots.len(),
        probe_source_count: evidence.probe_source_ids.len(),
        reconnects,
        projections,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplayedSourceState {
    AwaitingMapping,
    Initializing,
    Ready,
    Inactive,
}

#[derive(Default)]
struct ReplayedActivationProgress {
    capabilities_seen: bool,
    direct_access_required: bool,
    core_all_complete: bool,
    core_visible_complete: bool,
    direct_access_complete: bool,
}

impl ReplayedActivationProgress {
    fn initial_snapshots_complete(&self) -> bool {
        self.capabilities_seen
            && self.core_all_complete
            && self.core_visible_complete
            && (!self.direct_access_required || self.direct_access_complete)
    }
}

fn validate_probe_lifecycle_replay(records: &[RawRecord]) -> AuditResult<()> {
    let mut states: HashMap<String, ReplayedSourceState> = HashMap::new();
    let mut activation_progress: HashMap<String, ReplayedActivationProgress> = HashMap::new();
    for record in records.iter().filter(|record| {
        matches!(
            record.record_type.as_str(),
            "probe_event" | "probe_response"
        )
    }) {
        let object = record
            .value
            .as_object()
            .ok_or_else(|| AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"))?;
        let source = required_string(object, "source_instance_id", 256)?;
        let message = required_object(object, "message")?;
        let event = (record.record_type == "probe_event")
            .then(|| message.get("event").and_then(Value::as_str))
            .flatten();
        let current = states.get(source).copied();
        let other_live = states.iter().any(|(other, state)| {
            other != source
                && matches!(
                    state,
                    ReplayedSourceState::AwaitingMapping
                        | ReplayedSourceState::Initializing
                        | ReplayedSourceState::Ready
                )
        });
        match current {
            None => {
                if event != Some("probe.loaded") || other_live {
                    return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                }
                states.insert(source.to_owned(), ReplayedSourceState::AwaitingMapping);
            }
            Some(ReplayedSourceState::AwaitingMapping | ReplayedSourceState::Inactive) => {
                if event != Some("probe.mapping_active") || other_live {
                    return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                }
                states.insert(source.to_owned(), ReplayedSourceState::Initializing);
                activation_progress
                    .insert(source.to_owned(), ReplayedActivationProgress::default());
            }
            Some(ReplayedSourceState::Initializing) => match event {
                Some("probe.capabilities") => {
                    let progress = activation_progress
                        .get_mut(source)
                        .ok_or_else(|| AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"))?;
                    let data = required_object(message, "data")?;
                    let direct = required_object(data, "direct_access")?;
                    if progress.capabilities_seen
                        || direct.get("active").and_then(Value::as_bool).is_none()
                    {
                        return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                    }
                    progress.capabilities_seen = true;
                    progress.direct_access_required = required_bool(direct, "active")?;
                }
                Some("probe.bank.chunk" | "probe.direct_access.chunk") => {
                    let data = required_object(message, "data")?;
                    let progress = activation_progress
                        .get_mut(source)
                        .ok_or_else(|| AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"))?;
                    let stream = required_string(data, "stream", 64)?;
                    let reason = required_string(data, "reason", 64)?;
                    let complete = required_bool(data, "snapshot_complete")?;
                    if reason == "feedback" {
                        if !progress.capabilities_seen
                            || progress.initial_snapshots_complete()
                            || !matches!(stream, "mixer_bank_feedback" | "direct_access_feedback")
                            || (stream == "direct_access_feedback"
                                && !progress.direct_access_required)
                        {
                            return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                        }
                    } else if reason != "page_activate" {
                        return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                    } else if complete {
                        match stream {
                            "mixer_bank_snapshot" => match data
                                .get("config_id")
                                .and_then(Value::as_str)
                            {
                                Some("MB_CORE_ALL") => progress.core_all_complete = true,
                                Some("MB_CORE_VISIBLE") => progress.core_visible_complete = true,
                                _ => {
                                    return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                                }
                            },
                            "direct_access_snapshot" if progress.direct_access_required => {
                                progress.direct_access_complete = true;
                            }
                            _ => {
                                return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                            }
                        }
                    }
                }
                Some("probe.ready")
                    if message
                        .get("data")
                        .and_then(Value::as_object)
                        .and_then(|data| data.get("ready"))
                        .and_then(Value::as_bool)
                        == Some(true) =>
                {
                    if !activation_progress
                        .get(source)
                        .is_some_and(ReplayedActivationProgress::initial_snapshots_complete)
                    {
                        return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                    }
                    states.insert(source.to_owned(), ReplayedSourceState::Ready);
                }
                _ => return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID")),
            },
            Some(ReplayedSourceState::Ready) => {
                if event == Some("probe.ready")
                    && message
                        .get("data")
                        .and_then(Value::as_object)
                        .and_then(|data| data.get("ready"))
                        .and_then(Value::as_bool)
                        == Some(false)
                {
                    states.insert(source.to_owned(), ReplayedSourceState::Inactive);
                } else if matches!(
                    event,
                    Some(
                        "probe.loaded"
                            | "probe.mapping_active"
                            | "probe.capabilities"
                            | "probe.ready"
                    )
                ) || event
                    .and_then(|event| {
                        matches!(event, "probe.bank.chunk" | "probe.direct_access.chunk")
                            .then(|| required_object(message, "data"))
                    })
                    .transpose()?
                    .is_some_and(|data| {
                        data.get("reason").and_then(Value::as_str) == Some("page_activate")
                    })
                {
                    return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
                }
            }
        }
    }
    if states
        .values()
        .filter(|state| **state == ReplayedSourceState::Ready)
        .count()
        != 1
        || states.values().any(|state| {
            matches!(
                state,
                ReplayedSourceState::AwaitingMapping | ReplayedSourceState::Initializing
            )
        })
    {
        return Err(AuditError::new("PROBE_SOURCE_LIFECYCLE_INVALID"));
    }
    Ok(())
}

fn validate_observation_capability_consistency(
    evidence: &Evidence,
    capabilities: &CapabilityReport,
) -> AuditResult<()> {
    let direct = &capabilities.direct_access;
    for assembly in evidence.raw_snapshots.values().filter(|assembly| {
        assembly.kind == SnapshotKind::DirectAccess && assembly.is_snapshot && assembly.completed
    }) {
        validate_direct_access_snapshot_epoch(evidence, assembly)?;
        for item in &assembly.items {
            let Some(object) = item.as_object().filter(|object| {
                object.get("record_kind").and_then(Value::as_str) == Some("observation")
            }) else {
                continue;
            };
            let unique_redacted = required_bool(object, "unique_name_redacted")?;
            let expected_unique_status = if direct.unique_name {
                "not_invoked_by_policy"
            } else {
                "not_available"
            };
            if unique_redacted != direct.unique_name
                || object.get("unique_name_status").and_then(Value::as_str)
                    != Some(expected_unique_status)
                || (!direct.unique_id
                    && (required_bool(object, "host_id_redacted")?
                        || !object.get("host_id_raw").is_none_or(Value::is_null)
                        || !object.get("host_id_ref").is_none_or(Value::is_null)))
                || (!direct.title
                    && (required_bool(object, "title_redacted")?
                        || !object.get("title").is_none_or(Value::is_null)))
                || (!direct.type_name
                    && (required_bool(object, "type_name_redacted")?
                        || !object.get("type_name").is_none_or(Value::is_null)))
                || (!direct.mixer_visibility
                    && !object.get("mixer_visible").is_none_or(Value::is_null))
                || (!direct.mixer_index && !object.get("mixer_index").is_none_or(Value::is_null))
                || (!direct.mixer_zone && !object.get("mixer_zone").is_none_or(Value::is_null))
            {
                return Err(AuditError::new("OBSERVATION_CAPABILITY_MISMATCH"));
            }
        }
    }
    Ok(())
}

fn validate_direct_access_snapshot_epoch(
    evidence: &Evidence,
    assembly: &RawSnapshotAssembly,
) -> AuditResult<()> {
    let observed = assembly
        .observation_epoch
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"))?;
    let first_chunk = assembly
        .chunk_positions
        .first()
        .copied()
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"))?;
    let valid = match assembly.reason.as_str() {
        "page_activate" => {
            let mut matching_capabilities = evidence.capabilities.iter().filter(|event| {
                event.checkpoint_id == assembly.checkpoint_id
                    && event.source_instance_id == assembly.source_instance_id
                    && event.index < first_chunk.record_index
                    && event.monotonic_timestamp_ms <= first_chunk.monotonic_timestamp_ms
            });
            let capability = matching_capabilities
                .next()
                .ok_or_else(|| AuditError::new("DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"))?;
            if matching_capabilities.next().is_some() {
                return Err(AuditError::new("DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"));
            }
            observed == capability_current_epoch(&capability.data)?
        }
        "command_snapshot" => {
            observed
                == known_observation_epoch_at(evidence, &assembly.source_instance_id, first_chunk)?
        }
        "object_change" | "object_will_be_removed" | "parameter_change" => {
            observed
                == known_observation_epoch_at(evidence, &assembly.source_instance_id, first_chunk)?
        }
        _ => false,
    };
    if !valid {
        return Err(AuditError::new("DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"));
    }
    Ok(())
}

fn known_observation_epoch_at(
    evidence: &Evidence,
    source_instance_id: &str,
    position: ProbePosition,
) -> AuditResult<u64> {
    if !evidence.loaded.iter().any(|loaded| {
        loaded.source_instance_id == source_instance_id && loaded.index < position.record_index
    }) {
        return Err(AuditError::new("OBSERVATION_EPOCH_SOURCE_INVALID"));
    }
    Ok(evidence
        .responses
        .iter()
        .filter_map(|(request_id, response)| {
            evidence
                .commands
                .get(request_id)
                .filter(|command| command.method == "probe.observation.cut")
                .filter(|_| response.source_instance_id == source_instance_id)
                .and_then(|_| {
                    let response_position = ProbePosition {
                        monotonic_timestamp_ms: response.received_monotonic_timestamp_ms,
                        record_index: response.record_index,
                    };
                    (response_position.record_index <= position.record_index).then(|| {
                        response
                            .result
                            .get("observation_epoch")
                            .and_then(Value::as_u64)
                            .map(|epoch| (response_position, epoch))
                    })
                })
                .flatten()
        })
        .max_by_key(|(response_position, _)| response_position.record_index)
        .map_or(0, |(_, epoch)| epoch))
}

fn safe_run_alias(run_id: &str) -> String {
    let digest = sha256_hex(run_id.as_bytes());
    format!("run-{}", &digest[..16])
}

fn fixture_acceptance_report(acceptance: &FixtureAcceptance) -> FixtureAcceptanceReport {
    FixtureAcceptanceReport {
        alternate_instrument_plugin_used: matches!(
            acceptance.alternate_plugins.instrument,
            AlternatePluginEntry::Used { .. }
        ),
        alternate_effect_plugin_used: matches!(
            acceptance.alternate_plugins.effect,
            AlternatePluginEntry::Used { .. }
        ),
        p05_accepted_title: acceptance.p05_title.accepted_title.clone(),
        p05_setup_variance: acceptance.p05_title.setup_variance,
        p09_accepted_title: acceptance.p09_title.accepted_title.clone(),
        p09_setup_variance: acceptance.p09_title.setup_variance,
    }
}

fn validate_checkpoints(evidence: &Evidence) -> AuditResult<()> {
    if evidence.checkpoints.len() != REQUIRED_CHECKPOINTS.len()
        || evidence.actions.len() != REQUIRED_CHECKPOINTS.len()
    {
        return Err(AuditError::new("CHECKPOINT_COVERAGE_INVALID"));
    }
    let mut previous_end: Option<CheckpointMarker> = None;
    for id in REQUIRED_CHECKPOINTS {
        let checkpoint = evidence
            .checkpoints
            .get(id)
            .ok_or_else(|| AuditError::new("CHECKPOINT_MISSING"))?;
        let begin = checkpoint
            .begin
            .ok_or_else(|| AuditError::new("CHECKPOINT_BEGIN_MISSING"))?;
        let end = checkpoint
            .end
            .ok_or_else(|| AuditError::new("CHECKPOINT_END_MISSING"))?;
        let action = evidence
            .actions
            .get(id)
            .copied()
            .ok_or_else(|| AuditError::new("ACTION_MARKER_MISSING"))?;
        if previous_end.is_some_and(|previous| {
            begin.record_index <= previous.record_index
                || begin.monotonic_timestamp_ms < previous.monotonic_timestamp_ms
                || begin.timestamp_unix_ms < previous.timestamp_unix_ms
        }) {
            return Err(AuditError::new("CHECKPOINT_ORDER_INVALID"));
        }
        if action.monotonic_timestamp_ms < begin.monotonic_timestamp_ms
            || action.monotonic_timestamp_ms > end.monotonic_timestamp_ms
            || action.timestamp_unix_ms < begin.timestamp_unix_ms
            || action.timestamp_unix_ms > end.timestamp_unix_ms
            || action.record_index <= begin.record_index
            || action.record_index >= end.record_index
        {
            return Err(AuditError::new("ACTION_MARKER_OUTSIDE_CHECKPOINT"));
        }
        if end
            .monotonic_timestamp_ms
            .saturating_sub(begin.monotonic_timestamp_ms)
            < CALLBACK_WINDOW_MS
            || end
                .timestamp_unix_ms
                .saturating_sub(begin.timestamp_unix_ms)
                < CALLBACK_WINDOW_MS
        {
            return Err(AuditError::new("CHECKPOINT_DURATION_INVALID"));
        }
        let last_probe_receive = evidence
            .last_probe_receive_by_checkpoint
            .get(id)
            .copied()
            .ok_or_else(|| AuditError::new("CHECKPOINT_HAS_NO_PROBE_EVIDENCE"))?;
        if end
            .monotonic_timestamp_ms
            .saturating_sub(last_probe_receive.monotonic_timestamp_ms)
            < MIN_QUIET_PERIOD_MS
        {
            return Err(AuditError::new("CHECKPOINT_RAW_QUIET_PERIOD_MISSING"));
        }
        previous_end = Some(end);
    }
    Ok(())
}

fn validate_commands(
    profile: Profile,
    direct_access_required: bool,
    evidence: &Evidence,
) -> AuditResult<()> {
    if evidence.commands.is_empty() {
        return Err(AuditError::new("NO_PROBE_COMMANDS"));
    }
    for checkpoint_id in REQUIRED_CHECKPOINTS {
        let actual_ids = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .ok_or_else(|| AuditError::new("CHECKPOINT_COMMANDS_MISSING"))?;
        let same_script_reactivated =
            checkpoint_has_same_script_reactivation(evidence, checkpoint_id);
        let expected = expected_commands_with_activation(
            checkpoint_id,
            direct_access_required,
            same_script_reactivated,
        );
        if actual_ids.len() != expected.len() {
            return Err(AuditError::new("CHECKPOINT_COMMAND_SEQUENCE_INVALID"));
        }
        let final_snapshot_anchor = final_snapshot_anchor(evidence, checkpoint_id, actual_ids)?;
        for (request_id, expected_command) in actual_ids.iter().zip(expected) {
            let command = evidence
                .commands
                .get(request_id)
                .ok_or_else(|| AuditError::new("COMMAND_EVIDENCE_MISSING"))?;
            if command.checkpoint_id != checkpoint_id
                || command.method != expected_command.method
                || command.config_id.as_deref() != expected_command.config_id
            {
                return Err(AuditError::new("CHECKPOINT_COMMAND_SEQUENCE_INVALID"));
            }
            let checkpoint_begin = evidence
                .checkpoints
                .get(checkpoint_id)
                .and_then(|checkpoint| checkpoint.begin)
                .ok_or_else(|| AuditError::new("CHECKPOINT_BEGIN_MISSING"))?;
            let action = evidence
                .actions
                .get(checkpoint_id)
                .copied()
                .ok_or_else(|| AuditError::new("ACTION_MARKER_MISSING"))?;
            let send = evidence
                .send_results
                .get(request_id)
                .ok_or_else(|| AuditError::new("COMMAND_SEND_RESULT_MISSING"))?;
            let response = evidence
                .responses
                .get(request_id)
                .ok_or_else(|| AuditError::new("COMMAND_RESPONSE_MISSING"))?;
            if !send.sent
                || (command.method == "probe.discover" && send.index <= command.index)
                || (command.method != "probe.discover"
                    && send.index != command.index.saturating_add(1))
                || response.checkpoint_id != checkpoint_id
            {
                return Err(AuditError::new("COMMAND_COMPLETION_INVALID"));
            }
            if send.emitted_monotonic_timestamp_ms < send.completed_monotonic_timestamp_ms
                || (command.method != "probe.discover"
                    && command.emitted_monotonic_timestamp_ms
                        < send.completed_monotonic_timestamp_ms)
            {
                return Err(AuditError::new("COMMAND_SEND_TIMESTAMP_INVALID"));
            }
            let checkpoint_end = evidence
                .checkpoints
                .get(checkpoint_id)
                .and_then(|checkpoint| checkpoint.end)
                .ok_or_else(|| AuditError::new("CHECKPOINT_END_MISSING"))?;
            if command.index <= checkpoint_begin.record_index
                || command.index >= checkpoint_end.record_index
                || send.index <= checkpoint_begin.record_index
                || send.index >= checkpoint_end.record_index
                || response.record_index <= checkpoint_begin.record_index
                || response.record_index >= checkpoint_end.record_index
                || send.completed_monotonic_timestamp_ms < checkpoint_begin.monotonic_timestamp_ms
                || send.completed_monotonic_timestamp_ms > checkpoint_end.monotonic_timestamp_ms
                || command.emitted_monotonic_timestamp_ms < checkpoint_begin.monotonic_timestamp_ms
                || command.emitted_monotonic_timestamp_ms > checkpoint_end.monotonic_timestamp_ms
                || send.emitted_monotonic_timestamp_ms < checkpoint_begin.monotonic_timestamp_ms
                || send.emitted_monotonic_timestamp_ms > checkpoint_end.monotonic_timestamp_ms
                || response.received_monotonic_timestamp_ms
                    < checkpoint_begin.monotonic_timestamp_ms
                || response.received_monotonic_timestamp_ms > checkpoint_end.monotonic_timestamp_ms
            {
                return Err(AuditError::new("COMMAND_OUTSIDE_CHECKPOINT_WINDOW"));
            }
            if command.method == "probe.observation.cut" {
                let binding = evidence
                    .auto_actions
                    .get(checkpoint_id)
                    .ok_or_else(|| AuditError::new("OBSERVATION_CUT_ACTION_ORDER_INVALID"))?;
                let response_epoch = response
                    .result
                    .get("observation_epoch")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| AuditError::new("OBSERVATION_CUT_ACTION_ORDER_INVALID"))?;
                if binding.request_id != *request_id
                    || binding.observation_epoch != response_epoch
                    || response.record_index >= action.record_index
                    || response.record_index.saturating_add(1) != action.record_index
                    || send.completed_monotonic_timestamp_ms > action.monotonic_timestamp_ms
                    || response.received_monotonic_timestamp_ms > action.monotonic_timestamp_ms
                    || action
                        .monotonic_timestamp_ms
                        .saturating_sub(response.received_monotonic_timestamp_ms)
                        > ACTION_COMMAND_DEADLINE_MS
                {
                    return Err(AuditError::new("OBSERVATION_CUT_ACTION_ORDER_INVALID"));
                }
            } else if send.completed_monotonic_timestamp_ms < action.monotonic_timestamp_ms
                || command.index <= action.record_index
            {
                return Err(AuditError::new("COMMAND_BEFORE_ACTION_MARKER"));
            }
            if bank_operation(checkpoint_id)
                .is_some_and(|(_, operation)| operation == command.method)
                && send
                    .completed_monotonic_timestamp_ms
                    .saturating_sub(action.monotonic_timestamp_ms)
                    > ACTION_COMMAND_DEADLINE_MS
            {
                return Err(AuditError::new("BANK_ACTION_COMMAND_TOO_LATE"));
            }
            if response.received_monotonic_timestamp_ms < send.completed_monotonic_timestamp_ms {
                return Err(AuditError::new("COMMAND_RESPONSE_BEFORE_SEND"));
            }
            if matches!(
                command.method.as_str(),
                "probe.bank.snapshot" | "probe.direct_access.snapshot"
            ) && send.completed_monotonic_timestamp_ms
                < final_snapshot_anchor.saturating_add(CALLBACK_WINDOW_MS)
            {
                return Err(AuditError::new("FINAL_SNAPSHOT_STARTED_TOO_EARLY"));
            }
            if command.method != "probe.discover"
                && command.target_instance_id.as_deref()
                    != Some(response.source_instance_id.as_str())
            {
                return Err(AuditError::new("COMMAND_RESPONSE_SOURCE_MISMATCH"));
            }
            validate_command_response_result(command, response)?;
            if command.method == "probe.capabilities.get"
                && (validate_capabilities(profile, &response.result)? != direct_access_required
                    || capability_report(&response.result)?
                        != capability_report(initial_capability_result(evidence)?)?)
            {
                return Err(AuditError::new("CAPABILITY_RESULT_CHANGED"));
            }
            if command.method == "probe.discover" {
                let discovery = evidence
                    .discoveries
                    .get(request_id)
                    .ok_or_else(|| AuditError::new("DISCOVERY_COMPLETION_MISSING"))?;
                if discovery.checkpoint_id != checkpoint_id
                    || discovery.selected_source_instance_id != response.source_instance_id
                {
                    return Err(AuditError::new("DISCOVERY_COMPLETION_INVALID"));
                }
            }
            if let Some((kind, reason)) = command_followup(&command.method) {
                let snapshot = matching_snapshot(evidence, command, kind, reason)
                    .ok_or_else(|| AuditError::new("COMMAND_SNAPSHOT_FOLLOWUP_MISSING"))?;
                if snapshot.received_monotonic_timestamp_ms
                    < response.received_monotonic_timestamp_ms
                    || snapshot.received_monotonic_timestamp_ms
                        > checkpoint_end.monotonic_timestamp_ms
                {
                    return Err(AuditError::new("COMMAND_SNAPSHOT_FOLLOWUP_INVALID"));
                }
            }
        }
        if bank_operation(checkpoint_id).is_some() {
            validate_navigation_generation(evidence, actual_ids)?;
        }
        validate_final_projection_isolation(checkpoint_id, evidence, actual_ids)?;
        let final_request_id = actual_ids
            .last()
            .ok_or_else(|| AuditError::new("CHECKPOINT_COMMANDS_MISSING"))?;
        let final_command = evidence
            .commands
            .get(final_request_id)
            .ok_or_else(|| AuditError::new("COMMAND_EVIDENCE_MISSING"))?;
        if final_command.method != "probe.bank.snapshot" {
            return Err(AuditError::new("FINAL_SNAPSHOT_COMMAND_MISSING"));
        }
        let response = evidence
            .responses
            .get(final_request_id)
            .ok_or_else(|| AuditError::new("COMMAND_RESPONSE_MISSING"))?;
        let (kind, reason) = command_followup(&final_command.method)
            .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_COMMAND_MISSING"))?;
        let snapshot = matching_snapshot(evidence, final_command, kind, reason)
            .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_CHUNK_MISSING"))?;
        let final_position = ProbePosition {
            monotonic_timestamp_ms: snapshot.received_monotonic_timestamp_ms,
            record_index: snapshot.record_index,
        };
        if evidence
            .last_probe_receive_by_checkpoint
            .get(checkpoint_id)
            .copied()
            != Some(final_position)
        {
            return Err(AuditError::new("PROBE_RECORD_AFTER_FINAL_SNAPSHOT"));
        }
        let last_final_evidence = response
            .received_monotonic_timestamp_ms
            .max(snapshot.received_monotonic_timestamp_ms);
        let checkpoint_end = evidence
            .checkpoints
            .get(checkpoint_id)
            .and_then(|checkpoint| checkpoint.end)
            .ok_or_else(|| AuditError::new("CHECKPOINT_END_MISSING"))?;
        if checkpoint_end
            .monotonic_timestamp_ms
            .saturating_sub(last_final_evidence)
            < MIN_QUIET_PERIOD_MS
        {
            return Err(AuditError::new("FINAL_SNAPSHOT_QUIET_PERIOD_MISSING"));
        }
    }
    validate_observation_cuts(evidence)?;
    validate_snapshot_command_reverse_coverage(evidence)?;
    if evidence.send_results.len() != evidence.commands.len()
        || evidence.responses.len() != evidence.commands.len()
        || evidence.discoveries.len()
            != evidence
                .commands
                .values()
                .filter(|command| command.method == "probe.discover")
                .count()
    {
        return Err(AuditError::new("COMMAND_EVIDENCE_COUNT_INVALID"));
    }
    Ok(())
}

fn validate_snapshot_command_reverse_coverage(evidence: &Evidence) -> AuditResult<()> {
    let mut matched_commands = HashSet::new();
    for snapshot in evidence
        .snapshots
        .iter()
        .filter(|snapshot| snapshot.reason.starts_with("command_"))
    {
        let mut candidates = evidence.commands.values().filter(|command| {
            let Some((kind, reason)) = command_followup(&command.method) else {
                return false;
            };
            kind == snapshot.kind
                && reason == snapshot.reason
                && command.checkpoint_id == snapshot.checkpoint_id
                && command.config_id.as_deref() == snapshot.config_id.as_deref()
                && command.target_instance_id.as_deref()
                    == Some(snapshot.source_instance_id.as_str())
                && evidence
                    .send_results
                    .get(&command.request_id)
                    .is_some_and(|send| {
                        send.sent
                            && send.completed_monotonic_timestamp_ms
                                <= snapshot.received_monotonic_timestamp_ms
                    })
        });
        let command = candidates
            .next()
            .ok_or_else(|| AuditError::new("SNAPSHOT_COMMAND_COVERAGE_INVALID"))?;
        if candidates.next().is_some() || !matched_commands.insert(command.request_id.as_str()) {
            return Err(AuditError::new("SNAPSHOT_COMMAND_COVERAGE_INVALID"));
        }
    }
    let expected = evidence
        .commands
        .values()
        .filter(|command| command_followup(&command.method).is_some())
        .count();
    if matched_commands.len() != expected {
        return Err(AuditError::new("SNAPSHOT_COMMAND_COVERAGE_INVALID"));
    }
    Ok(())
}

fn validate_command_response_result(
    command: &CommandEvidence,
    response: &ResponseEvidence,
) -> AuditResult<()> {
    let result = response
        .result
        .as_object()
        .ok_or_else(|| AuditError::new("COMMAND_RESPONSE_RESULT_INVALID"))?;
    let valid = match command.method.as_str() {
        "probe.discover" => {
            has_exact_keys(result, &["instance_id", "ready", "read_only"])
                && result.get("instance_id").and_then(Value::as_str)
                    == Some(response.source_instance_id.as_str())
                && result.get("ready").and_then(Value::as_bool) == Some(true)
                && result.get("read_only").and_then(Value::as_bool) == Some(true)
        }
        "probe.capabilities.get" => true,
        "probe.observation.cut" => {
            has_exact_keys(result, &["observation_epoch"])
                && result
                    .get("observation_epoch")
                    .and_then(Value::as_u64)
                    .is_some_and(|epoch| epoch <= 2_147_483_647)
        }
        "probe.bank.reset" | "probe.bank.next" | "probe.bank.prev" => {
            has_exact_keys(result, &["config_id", "action"])
                && result.get("config_id").and_then(Value::as_str) == command.config_id.as_deref()
                && result.get("action").and_then(Value::as_str)
                    == command.method.strip_prefix("probe.bank.")
        }
        "probe.bank.snapshot" => {
            has_exact_keys(result, &["config_id"])
                && result.get("config_id").and_then(Value::as_str) == command.config_id.as_deref()
        }
        "probe.direct_access.snapshot" => result.is_empty(),
        _ => false,
    };
    if !valid {
        return Err(AuditError::new("COMMAND_RESPONSE_RESULT_INVALID"));
    }
    Ok(())
}

fn final_snapshot_anchor(
    evidence: &Evidence,
    checkpoint_id: &'static str,
    request_ids: &[String],
) -> AuditResult<u64> {
    let action = evidence
        .actions
        .get(checkpoint_id)
        .copied()
        .ok_or_else(|| AuditError::new("ACTION_MARKER_MISSING"))?;
    if requires_observation_cut(checkpoint_id) {
        return Ok(action.monotonic_timestamp_ms);
    }
    let anchor_method = bank_operation(checkpoint_id).map(|(_, method)| method);
    let Some(anchor_method) = anchor_method else {
        return Ok(action.monotonic_timestamp_ms);
    };
    let mut matches = request_ids.iter().filter_map(|request_id| {
        evidence
            .commands
            .get(request_id)
            .filter(|command| command.method == anchor_method)
            .map(|command| (request_id, command))
    });
    let (request_id, command) = matches
        .next()
        .ok_or_else(|| AuditError::new("OPERATION_ANCHOR_MISSING"))?;
    if matches.next().is_some() {
        return Err(AuditError::new("OPERATION_ANCHOR_DUPLICATE"));
    }
    let response = evidence
        .responses
        .get(request_id)
        .ok_or_else(|| AuditError::new("OPERATION_ANCHOR_RESPONSE_MISSING"))?;
    if anchor_method.starts_with("probe.bank.") {
        let result = response
            .result
            .as_object()
            .ok_or_else(|| AuditError::new("BANK_ACTION_RESPONSE_INVALID"))?;
        let expected_action = anchor_method
            .strip_prefix("probe.bank.")
            .ok_or_else(|| AuditError::new("BANK_ACTION_RESPONSE_INVALID"))?;
        if result.len() != 2
            || result.get("config_id").and_then(Value::as_str) != command.config_id.as_deref()
            || result.get("action").and_then(Value::as_str) != Some(expected_action)
        {
            return Err(AuditError::new("BANK_ACTION_RESPONSE_INVALID"));
        }
    }
    Ok(response.received_monotonic_timestamp_ms)
}

fn validate_navigation_generation(evidence: &Evidence, request_ids: &[String]) -> AuditResult<()> {
    let mut generations = Vec::new();
    for request_id in request_ids {
        let command = evidence
            .commands
            .get(request_id)
            .ok_or_else(|| AuditError::new("COMMAND_EVIDENCE_MISSING"))?;
        if command_followup(&command.method).is_none()
            || !matches!(
                command.method.as_str(),
                "probe.bank.reset" | "probe.bank.next" | "probe.bank.prev" | "probe.bank.snapshot"
            )
        {
            continue;
        }
        let (_, reason) = command_followup(&command.method)
            .ok_or_else(|| AuditError::new("NAVIGATION_GENERATION_INVALID"))?;
        let snapshot = matching_snapshot(evidence, command, SnapshotKind::Bank, reason)
            .ok_or_else(|| AuditError::new("NAVIGATION_GENERATION_INVALID"))?;
        let generation = evidence
            .raw_snapshots
            .get(&(
                snapshot.source_instance_id.clone(),
                snapshot.snapshot_id.clone(),
            ))
            .and_then(|assembly| assembly.bank_generation)
            .ok_or_else(|| AuditError::new("NAVIGATION_GENERATION_INVALID"))?;
        generations.push(generation);
    }
    if generations.len() != 2 || generations[0] != generations[1] {
        return Err(AuditError::new("NAVIGATION_GENERATION_INVALID"));
    }
    Ok(())
}

fn validate_observation_cuts(evidence: &Evidence) -> AuditResult<()> {
    let initial = initial_capability_result(evidence)?
        .as_object()
        .and_then(|object| object.get("observation_epoch"))
        .and_then(Value::as_object)
        .and_then(|epoch| epoch.get("current"))
        .and_then(Value::as_u64)
        .ok_or_else(|| AuditError::new("OBSERVATION_EPOCH_CAPABILITY_INVALID"))?;
    let mut previous = initial;
    for checkpoint_id in REQUIRED_CHECKPOINTS
        .into_iter()
        .filter(|checkpoint_id| requires_observation_cut(checkpoint_id))
    {
        let command_ids = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .ok_or_else(|| AuditError::new("OBSERVATION_CUT_MISSING"))?;
        let mut cuts = command_ids.iter().filter_map(|request_id| {
            evidence
                .commands
                .get(request_id)
                .filter(|command| command.method == "probe.observation.cut")
                .map(|_| request_id)
        });
        let request_id = cuts
            .next()
            .ok_or_else(|| AuditError::new("OBSERVATION_CUT_MISSING"))?;
        if cuts.next().is_some() {
            return Err(AuditError::new("OBSERVATION_CUT_DUPLICATE"));
        }
        let result = evidence
            .responses
            .get(request_id)
            .and_then(|response| response.result.as_object())
            .ok_or_else(|| AuditError::new("OBSERVATION_CUT_RESPONSE_INVALID"))?;
        if result.len() != 1 {
            return Err(AuditError::new("OBSERVATION_CUT_RESPONSE_INVALID"));
        }
        let epoch = required_u64(result, "observation_epoch")?;
        if epoch != previous.saturating_add(1) || epoch > 2_147_483_647 {
            return Err(AuditError::new("OBSERVATION_EPOCH_SEQUENCE_INVALID"));
        }
        previous = epoch;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct FeedbackEpochEvidence {
    position: ProbePosition,
    item_index: usize,
    observation_seq: u64,
    observation_epoch: u64,
}

fn validate_feedback_epoch_sequences(evidence: &Evidence) -> AuditResult<()> {
    let mut feedback_by_source: HashMap<&str, Vec<FeedbackEpochEvidence>> = HashMap::new();
    for assembly in evidence
        .raw_snapshots
        .values()
        .filter(|assembly| !assembly.is_snapshot)
    {
        for (item_index, (item, position)) in assembly
            .items
            .iter()
            .zip(&assembly.item_positions)
            .enumerate()
        {
            let object = item
                .as_object()
                .ok_or_else(|| AuditError::new("FEEDBACK_OBSERVATION_INVALID"))?;
            if object.get("record_kind").and_then(Value::as_str) == Some("host_id_fragment") {
                continue;
            }
            let observation_seq = required_u64(object, "observation_seq")?;
            let observation_epoch = required_u64(object, "observation_epoch")?;
            if observation_seq == 0 || observation_epoch > 2_147_483_647 {
                return Err(AuditError::new("FEEDBACK_OBSERVATION_INVALID"));
            }
            feedback_by_source
                .entry(&assembly.source_instance_id)
                .or_default()
                .push(FeedbackEpochEvidence {
                    position: *position,
                    item_index,
                    observation_seq,
                    observation_epoch,
                });
        }
    }

    let mut cuts_by_source: HashMap<&str, Vec<(ProbePosition, u64)>> = HashMap::new();
    for (request_id, response) in &evidence.responses {
        if evidence
            .commands
            .get(request_id)
            .is_some_and(|command| command.method == "probe.observation.cut")
        {
            let epoch = response
                .result
                .get("observation_epoch")
                .and_then(Value::as_u64)
                .ok_or_else(|| AuditError::new("OBSERVATION_CUT_RESPONSE_INVALID"))?;
            cuts_by_source
                .entry(&response.source_instance_id)
                .or_default()
                .push((
                    ProbePosition {
                        monotonic_timestamp_ms: response.received_monotonic_timestamp_ms,
                        record_index: response.record_index,
                    },
                    epoch,
                ));
        }
    }

    for (source, feedback) in &mut feedback_by_source {
        if !evidence
            .loaded
            .iter()
            .any(|loaded| loaded.source_instance_id == *source)
        {
            return Err(AuditError::new("FEEDBACK_SOURCE_SESSION_INVALID"));
        }
        feedback.sort_by_key(|item| (item.position.record_index, item.item_index));
        let mut expected_seq = 1_u64;
        let mut previous_epoch = None;
        let mut cuts = cuts_by_source.remove(source).unwrap_or_default();
        cuts.sort_by_key(|(position, _)| position.record_index);
        for item in feedback {
            if item.observation_seq != expected_seq
                || previous_epoch.is_some_and(|previous| item.observation_epoch < previous)
            {
                return Err(AuditError::new("FEEDBACK_OBSERVATION_SEQUENCE_INVALID"));
            }
            let known_current = cuts
                .iter()
                .filter(|(position, _)| position.record_index <= item.position.record_index)
                .map(|(_, epoch)| *epoch)
                .next_back()
                .unwrap_or(0);
            if item.observation_epoch > known_current {
                return Err(AuditError::new("FEEDBACK_OBSERVATION_EPOCH_FUTURE"));
            }
            expected_seq = expected_seq
                .checked_add(1)
                .ok_or_else(|| AuditError::new("FEEDBACK_OBSERVATION_SEQUENCE_INVALID"))?;
            previous_epoch = Some(item.observation_epoch);
        }
    }
    Ok(())
}

fn validate_final_projection_isolation(
    checkpoint_id: &'static str,
    evidence: &Evidence,
    request_ids: &[String],
) -> AuditResult<()> {
    let mut final_evidence = Vec::new();
    for request_id in request_ids {
        let command = evidence
            .commands
            .get(request_id)
            .ok_or_else(|| AuditError::new("COMMAND_EVIDENCE_MISSING"))?;
        let kind = match command.method.as_str() {
            "probe.bank.snapshot" => SnapshotKind::Bank,
            "probe.direct_access.snapshot" => SnapshotKind::DirectAccess,
            _ => continue,
        };
        let snapshot = matching_snapshot(evidence, command, kind, "command_snapshot")
            .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_CHUNK_MISSING"))?;
        let assembly = evidence
            .raw_snapshots
            .get(&(
                snapshot.source_instance_id.clone(),
                snapshot.snapshot_id.clone(),
            ))
            .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_ASSEMBLY_MISSING"))?;
        let response = evidence
            .responses
            .get(request_id)
            .ok_or_else(|| AuditError::new("COMMAND_RESPONSE_MISSING"))?;
        let send = evidence
            .send_results
            .get(request_id)
            .ok_or_else(|| AuditError::new("COMMAND_SEND_RESULT_MISSING"))?;
        final_evidence.push((command, snapshot, assembly, response, send));
    }
    let Some((_, first_snapshot, _, _, _)) = final_evidence.first() else {
        return Err(AuditError::new("FINAL_SNAPSHOT_COMMAND_MISSING"));
    };
    for pair in final_evidence.windows(2) {
        let (_, previous_snapshot, _, _, _) = pair[0];
        let (next_command, _, _, _, next_send) = pair[1];
        if next_command.index <= previous_snapshot.record_index
            || next_send.completed_monotonic_timestamp_ms
                < previous_snapshot.received_monotonic_timestamp_ms
        {
            return Err(AuditError::new("FINAL_PROJECTION_SEQUENCE_OVERLAP"));
        }
    }
    let mut allowed_positions = HashSet::new();
    for (_, _, assembly, response, _) in &final_evidence {
        allowed_positions.insert(ProbePosition {
            monotonic_timestamp_ms: response.received_monotonic_timestamp_ms,
            record_index: response.record_index,
        });
        allowed_positions.extend(assembly.chunk_positions.iter().copied());
    }
    let first_completion = ProbePosition {
        monotonic_timestamp_ms: first_snapshot.received_monotonic_timestamp_ms,
        record_index: first_snapshot.record_index,
    };
    if evidence
        .probe_positions_by_checkpoint
        .get(checkpoint_id)
        .into_iter()
        .flatten()
        .any(|position| *position > first_completion && !allowed_positions.contains(position))
    {
        return Err(AuditError::new("PROBE_ACTIVITY_BETWEEN_FINAL_PROJECTIONS"));
    }
    Ok(())
}

fn command_followup(method: &str) -> Option<(SnapshotKind, &'static str)> {
    match method {
        "probe.bank.reset" => Some((SnapshotKind::Bank, "command_reset")),
        "probe.bank.next" => Some((SnapshotKind::Bank, "command_next")),
        "probe.bank.prev" => Some((SnapshotKind::Bank, "command_prev")),
        "probe.bank.snapshot" => Some((SnapshotKind::Bank, "command_snapshot")),
        "probe.direct_access.snapshot" => Some((SnapshotKind::DirectAccess, "command_snapshot")),
        _ => None,
    }
}

fn matching_snapshot<'a>(
    evidence: &'a Evidence,
    command: &CommandEvidence,
    kind: SnapshotKind,
    reason: &str,
) -> Option<&'a SnapshotEvidence> {
    let mut matches = evidence.snapshots.iter().filter(|snapshot| {
        let send_completed = evidence
            .send_results
            .get(&command.request_id)
            .map(|send| send.completed_monotonic_timestamp_ms);
        snapshot.checkpoint_id == command.checkpoint_id
            && snapshot.kind == kind
            && snapshot.reason == reason
            && snapshot.config_id.as_deref() == command.config_id.as_deref()
            && command
                .target_instance_id
                .as_deref()
                .is_some_and(|target| target == snapshot.source_instance_id)
            && send_completed
                .is_some_and(|sent_at| snapshot.received_monotonic_timestamp_ms >= sent_at)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn build_semantic_projections(
    direct_access_required: bool,
    fixture_acceptance: &FixtureAcceptance,
    evidence: &Evidence,
) -> AuditResult<Vec<SemanticProjection>> {
    let mut projections = Vec::new();
    let mut aliases = AliasState::default();
    let feedback_index = bank_feedback_index(evidence)?;
    for checkpoint_id in REQUIRED_CHECKPOINTS {
        let request_ids = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .ok_or_else(|| AuditError::new("CHECKPOINT_COMMANDS_MISSING"))?;
        for request_id in request_ids {
            let command = evidence
                .commands
                .get(request_id)
                .ok_or_else(|| AuditError::new("COMMAND_EVIDENCE_MISSING"))?;
            let kind = match command.method.as_str() {
                "probe.bank.snapshot" => SnapshotKind::Bank,
                "probe.direct_access.snapshot" if direct_access_required => {
                    SnapshotKind::DirectAccess
                }
                _ => continue,
            };
            let snapshot = matching_snapshot(evidence, command, kind, "command_snapshot")
                .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_CHUNK_MISSING"))?;
            let assembly = evidence
                .raw_snapshots
                .get(&(
                    snapshot.source_instance_id.clone(),
                    snapshot.snapshot_id.clone(),
                ))
                .filter(|assembly| assembly.completed)
                .ok_or_else(|| AuditError::new("FINAL_SNAPSHOT_ASSEMBLY_MISSING"))?;
            let projection = match kind {
                SnapshotKind::Bank => build_bank_projection(
                    checkpoint_id,
                    assembly,
                    fixture_acceptance,
                    evidence,
                    &feedback_index,
                    &mut aliases,
                ),
                SnapshotKind::DirectAccess => build_direct_access_projection(
                    checkpoint_id,
                    assembly,
                    fixture_acceptance,
                    &mut aliases,
                ),
            };
            projections.push(projection);
        }
    }
    Ok(projections)
}

struct BankFeedbackObservation<'a> {
    checkpoint_id: &'static str,
    config_id: &'a str,
    slot_index: u64,
    changed_field: &'a str,
    observation_epoch: u64,
    changed_value: &'a Value,
    position: ProbePosition,
}

fn bank_feedback_index(
    evidence: &Evidence,
) -> AuditResult<HashMap<(String, u64), BankFeedbackObservation<'_>>> {
    let mut index = HashMap::new();
    for assembly in evidence
        .raw_snapshots
        .values()
        .filter(|assembly| assembly.kind == SnapshotKind::Bank && !assembly.is_snapshot)
    {
        for (item, position) in assembly.items.iter().zip(&assembly.item_positions) {
            let Some(object) = item.as_object().filter(|object| {
                object.get("record_kind").and_then(Value::as_str) == Some("observation")
            }) else {
                continue;
            };
            let sequence = required_u64(object, "observation_seq")?;
            let observation = BankFeedbackObservation {
                checkpoint_id: assembly.checkpoint_id,
                config_id: required_string(object, "config_id", 32)?,
                slot_index: required_u64(object, "slot_index")?,
                changed_field: required_string(object, "changed_field", 16)?,
                observation_epoch: required_u64(object, "observation_epoch")?,
                changed_value: object
                    .get("changed_value")
                    .ok_or_else(|| AuditError::new("BANK_FEEDBACK_PRIVACY_INVALID"))?,
                position: *position,
            };
            if index
                .insert((assembly.source_instance_id.clone(), sequence), observation)
                .is_some()
            {
                return Err(AuditError::new("BANK_OBSERVATION_SEQUENCE_DUPLICATE"));
            }
        }
    }
    Ok(index)
}

fn build_bank_projection(
    checkpoint_id: &'static str,
    assembly: &RawSnapshotAssembly,
    fixture_acceptance: &FixtureAcceptance,
    evidence: &Evidence,
    feedback_index: &HashMap<(String, u64), BankFeedbackObservation<'_>>,
    aliases: &mut AliasState,
) -> SemanticProjection {
    let mut slots_by_index: HashMap<u64, BankSlotProjection> = HashMap::new();
    let mut duplicate_slot_count = 0usize;
    let mut unknown_item_count = 0usize;
    let mut missing_title_count = 0usize;
    let mut unknown_title_count = 0usize;
    let mut duplicate_host_id_count = 0usize;
    let mut redacted_title_count = 0usize;
    let mut redacted_host_id_count = 0usize;
    let mut source_redacted_string_count = 0usize;
    let mut stale_or_unobserved_field_count = 0usize;
    let mut seen_host_aliases = HashSet::new();
    let action = evidence.actions.get(checkpoint_id).copied();
    for item in &assembly.items {
        let Some(object) = item.as_object() else {
            unknown_item_count += 1;
            continue;
        };
        match object.get("record_kind").and_then(Value::as_str) {
            Some("host_id_fragment") => continue,
            Some("observation") => {}
            _ => {
                unknown_item_count += 1;
                continue;
            }
        }
        let Some(slot_index) = object.get("slot_index").and_then(Value::as_u64) else {
            unknown_item_count += 1;
            continue;
        };
        if slot_index >= 8
            || object.get("config_id").and_then(Value::as_str) != assembly.config_id.as_deref()
        {
            unknown_item_count += 1;
            continue;
        }
        if slots_by_index.contains_key(&slot_index) {
            duplicate_slot_count += 1;
            continue;
        }
        let title_redacted = object
            .get("title_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let host_id_redacted = object
            .get("host_id_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let raw_redacted_count = object
            .get("redacted_string_count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        redacted_title_count += usize::from(title_redacted);
        redacted_host_id_count += usize::from(host_id_redacted);
        source_redacted_string_count += raw_redacted_count;
        let title_freshness = bank_field_freshness(
            evidence,
            assembly,
            object,
            "title",
            title_redacted,
            action,
            feedback_index,
        );
        let selected_freshness = bank_field_freshness(
            evidence,
            assembly,
            object,
            "selected",
            false,
            action,
            feedback_index,
        );
        let mute_freshness = bank_field_freshness(
            evidence,
            assembly,
            object,
            "mute",
            false,
            action,
            feedback_index,
        );
        let solo_freshness = bank_field_freshness(
            evidence,
            assembly,
            object,
            "solo",
            false,
            action,
            feedback_index,
        );
        stale_or_unobserved_field_count += [
            title_freshness,
            selected_freshness,
            mute_freshness,
            solo_freshness,
        ]
        .into_iter()
        .filter(|freshness| *freshness == FieldFreshness::StaleOrUnobserved)
        .count();
        let (mut title, mut title_category) = safe_fixture_title(object.get("title"));
        if title_freshness != FieldFreshness::Fresh {
            title = None;
            title_category = if title_redacted {
                SafeTitleCategory::Redacted
            } else {
                SafeTitleCategory::Unavailable
            };
        }
        match title_category {
            SafeTitleCategory::Unavailable | SafeTitleCategory::Empty => {
                missing_title_count += 1;
            }
            SafeTitleCategory::Redacted => unknown_title_count += 1,
            SafeTitleCategory::Fixture => {}
        }
        let host_id_freshness = bank_field_freshness(
            evidence,
            assembly,
            object,
            "host_id_raw",
            host_id_redacted,
            action,
            feedback_index,
        );
        stale_or_unobserved_field_count +=
            usize::from(host_id_freshness == FieldFreshness::StaleOrUnobserved);
        let (host_id_alias, host_id_byte_length) = if host_id_freshness == FieldFreshness::Fresh {
            resolve_host_id(object, &assembly.items, aliases)
        } else {
            (None, None)
        };
        if host_id_alias
            .as_ref()
            .is_some_and(|alias| !seen_host_aliases.insert(alias.clone()))
        {
            duplicate_host_id_count += 1;
        }
        slots_by_index.insert(
            slot_index,
            BankSlotProjection {
                slot_index,
                title,
                title_category,
                title_freshness,
                selected: (selected_freshness == FieldFreshness::Fresh)
                    .then(|| object.get("selected").and_then(Value::as_bool))
                    .flatten(),
                selected_freshness,
                mute: (mute_freshness == FieldFreshness::Fresh)
                    .then(|| object.get("mute").and_then(Value::as_bool))
                    .flatten(),
                mute_freshness,
                solo: (solo_freshness == FieldFreshness::Fresh)
                    .then(|| object.get("solo").and_then(Value::as_bool))
                    .flatten(),
                solo_freshness,
                title_redacted,
                host_id_redacted,
                redacted_string_count: raw_redacted_count,
                host_id_status: if host_id_alias.is_some() {
                    AliasStatus::Aliased
                } else {
                    AliasStatus::Unavailable
                },
                host_id_alias,
                host_id_byte_length,
            },
        );
    }
    let mut slots: Vec<_> = slots_by_index.into_values().collect();
    slots.sort_by_key(|slot| slot.slot_index);
    let title_observations: Vec<_> = slots.iter().map(|slot| slot.title.as_deref()).collect();
    let (p05_title, p09_title) =
        fixture_title_comparisons_from_titles(&title_observations, fixture_acceptance);
    SemanticProjection::MixerBank {
        checkpoint_id,
        config_id: assembly
            .config_id
            .as_deref()
            .filter(|config| matches!(*config, "MB_CORE_ALL" | "MB_CORE_VISIBLE"))
            .unwrap_or("redacted")
            .to_owned(),
        total_wire_items: assembly.items.len(),
        missing_slot_count: 8usize.saturating_sub(slots.len()),
        duplicate_slot_count,
        unknown_item_count,
        missing_title_count,
        unknown_title_count,
        redacted_title_count,
        redacted_host_id_count,
        source_redacted_string_count,
        stale_or_unobserved_field_count,
        duplicate_host_id_count,
        p05_title,
        p09_title,
        slots,
    }
}

fn bank_field_freshness(
    evidence: &Evidence,
    assembly: &RawSnapshotAssembly,
    object: &Map<String, Value>,
    field: &str,
    redacted: bool,
    action: Option<CheckpointMarker>,
    feedback_index: &HashMap<(String, u64), BankFeedbackObservation<'_>>,
) -> FieldFreshness {
    let generation_fresh = object
        .get("field_observation_generation")
        .and_then(Value::as_object)
        .and_then(|fields| fields.get(field))
        .and_then(Value::as_i64)
        == object
            .get("bank_generation")
            .and_then(Value::as_u64)
            .and_then(|generation| i64::try_from(generation).ok());
    if bank_operation(assembly.checkpoint_id).is_some()
        || matches!(assembly.checkpoint_id, "INIT" | "R1" | "R2")
    {
        return if !generation_fresh {
            FieldFreshness::StaleOrUnobserved
        } else if redacted {
            FieldFreshness::Redacted
        } else {
            FieldFreshness::Fresh
        };
    }
    let Some(cut_epoch) = checkpoint_cut_epoch(evidence, assembly.checkpoint_id) else {
        return FieldFreshness::StaleOrUnobserved;
    };
    let Some(action) = action else {
        return FieldFreshness::StaleOrUnobserved;
    };
    let Some(sequence) = object
        .get("field_last_observation_seq")
        .and_then(Value::as_object)
        .and_then(|fields| fields.get(field))
        .and_then(Value::as_u64)
    else {
        return FieldFreshness::StaleOrUnobserved;
    };
    let field_epoch = object
        .get("field_last_observation_epoch")
        .and_then(Value::as_object)
        .and_then(|fields| fields.get(field))
        .and_then(Value::as_u64);
    let Some(observation) = feedback_index.get(&(assembly.source_instance_id.clone(), sequence))
    else {
        return FieldFreshness::StaleOrUnobserved;
    };
    let expected_changed_field = if field == "host_id_raw" {
        "title"
    } else {
        field
    };
    let snapshot_position = assembly
        .chunk_positions
        .last()
        .copied()
        .unwrap_or(ProbePosition {
            monotonic_timestamp_ms: 0,
            record_index: 0,
        });
    let fresh = observation.checkpoint_id == assembly.checkpoint_id
        && generation_fresh
        && field_epoch == Some(cut_epoch)
        && observation.observation_epoch == cut_epoch
        && observation.config_id
            == object
                .get("config_id")
                .and_then(Value::as_str)
                .unwrap_or("")
        && observation.slot_index
            == object
                .get("slot_index")
                .and_then(Value::as_u64)
                .unwrap_or(9)
        && observation.changed_field == expected_changed_field
        && (field == "host_id_raw"
            || object
                .get(field)
                .is_some_and(|value| value == observation.changed_value))
        && observation.position.record_index > action.record_index
        && observation.position.monotonic_timestamp_ms > action.monotonic_timestamp_ms
        && observation.position <= snapshot_position;
    if !fresh {
        FieldFreshness::StaleOrUnobserved
    } else if redacted {
        FieldFreshness::Redacted
    } else {
        FieldFreshness::Fresh
    }
}

fn checkpoint_cut_epoch(evidence: &Evidence, checkpoint_id: &'static str) -> Option<u64> {
    evidence
        .commands_by_checkpoint
        .get(checkpoint_id)?
        .iter()
        .filter_map(|request_id| {
            evidence
                .commands
                .get(request_id)
                .filter(|command| command.method == "probe.observation.cut")
                .and_then(|_| evidence.responses.get(request_id))
                .and_then(|response| response.result.get("observation_epoch"))
                .and_then(Value::as_u64)
        })
        .next()
}

fn build_direct_access_projection(
    checkpoint_id: &'static str,
    assembly: &RawSnapshotAssembly,
    fixture_acceptance: &FixtureAcceptance,
    aliases: &mut AliasState,
) -> SemanticProjection {
    let mut nodes = Vec::new();
    let mut references = Vec::new();
    let mut missing_count = 0usize;
    let mut duplicate_count = 0usize;
    let mut unknown_count = 0usize;
    let mut duplicate_host_id_count = 0usize;
    let mut redacted_unique_name_count = 0usize;
    let mut redacted_title_count = 0usize;
    let mut redacted_type_name_count = 0usize;
    let mut redacted_host_id_count = 0usize;
    let mut source_redacted_string_count = 0usize;
    let mut seen_objects = HashSet::new();
    let mut seen_host_aliases = HashSet::new();
    for item in &assembly.items {
        let Some(object) = item.as_object() else {
            unknown_count += 1;
            continue;
        };
        match object.get("record_kind").and_then(Value::as_str) {
            Some("host_id_fragment") => continue,
            Some("observation") => {}
            Some("object_reference") => continue,
            _ => {
                unknown_count += 1;
                continue;
            }
        }
        let object_id = object.get("object_id").and_then(Value::as_u64);
        if object_id.is_none() {
            missing_count += 1;
        }
        if object_id.is_some_and(|id| !seen_objects.insert(id)) {
            duplicate_count += 1;
        }
        let object_alias =
            object_id.map(|id| aliases.object_alias(&assembly.source_instance_id, id));
        let parent_alias = object
            .get("parent_id")
            .and_then(Value::as_u64)
            .map(|id| aliases.object_alias(&assembly.source_instance_id, id));
        let unique_name_redacted = object
            .get("unique_name_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let title_redacted = object
            .get("title_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let type_name_redacted = object
            .get("type_name_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let host_id_redacted = object
            .get("host_id_redacted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        redacted_unique_name_count += usize::from(unique_name_redacted);
        redacted_title_count += usize::from(title_redacted);
        redacted_type_name_count += usize::from(type_name_redacted);
        redacted_host_id_count += usize::from(host_id_redacted);
        let source_redacted_count = object
            .get("redacted_string_count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        source_redacted_string_count += source_redacted_count;
        let (title, title_category) = if title_redacted {
            (None, SafeTitleCategory::Redacted)
        } else {
            safe_fixture_title(object.get("title"))
        };
        if matches!(
            title_category,
            SafeTitleCategory::Unavailable | SafeTitleCategory::Empty
        ) {
            missing_count += 1;
        } else if title_category == SafeTitleCategory::Redacted {
            unknown_count += 1;
        }
        let (host_id_alias, host_id_byte_length) =
            resolve_host_id(object, &assembly.items, aliases);
        if host_id_alias
            .as_ref()
            .is_some_and(|alias| !seen_host_aliases.insert(alias.clone()))
        {
            duplicate_host_id_count += 1;
        }
        let (type_name, type_name_category) = match object.get("type_name") {
            Some(Value::String(value)) if safe_direct_access_type_name(value) => {
                (Some(value.clone()), SafeTypeCategory::Allowlisted)
            }
            _ if type_name_redacted => (None, SafeTypeCategory::Redacted),
            _ => (None, SafeTypeCategory::Unavailable),
        };
        nodes.push(DirectAccessProjection {
            traversal_index: nodes.len(),
            depth: object.get("depth").and_then(Value::as_u64),
            child_index: object.get("child_index").and_then(Value::as_u64),
            object_alias,
            parent_alias,
            title,
            title_category,
            unique_name_redacted,
            title_redacted,
            type_name,
            type_name_category,
            type_name_redacted,
            host_id_redacted,
            host_id_status: if host_id_alias.is_some() {
                AliasStatus::Aliased
            } else {
                AliasStatus::Unavailable
            },
            host_id_alias,
            host_id_byte_length,
            mixer_visible: object.get("mixer_visible").and_then(Value::as_bool),
            mixer_index: object.get("mixer_index").and_then(Value::as_f64),
            mixer_zone: object.get("mixer_zone").and_then(Value::as_f64),
            child_count: object.get("child_count").and_then(Value::as_u64),
            metadata_error_count: object.get("metadata_error_count").and_then(Value::as_u64),
            redacted_string_count: source_redacted_count,
        });
    }
    for item in &assembly.items {
        let Some(object) = item.as_object().filter(|object| {
            object.get("record_kind").and_then(Value::as_str) == Some("object_reference")
        }) else {
            continue;
        };
        let Some(parent_id) = object.get("parent_id").and_then(Value::as_u64) else {
            unknown_count += 1;
            continue;
        };
        let Some(target_id) = object.get("object_id").and_then(Value::as_u64) else {
            unknown_count += 1;
            continue;
        };
        let Some(depth) = object.get("depth").and_then(Value::as_u64) else {
            unknown_count += 1;
            continue;
        };
        let Some(child_index) = object.get("child_index").and_then(Value::as_u64) else {
            unknown_count += 1;
            continue;
        };
        let Some(target_observation_index) = object
            .get("target_observation_index")
            .and_then(Value::as_u64)
        else {
            unknown_count += 1;
            continue;
        };
        let Some(reference_kind) = object
            .get("reference_kind")
            .and_then(Value::as_str)
            .and_then(direct_access_reference_kind)
        else {
            unknown_count += 1;
            continue;
        };
        references.push(DirectAccessReferenceProjection {
            reference_index: references.len(),
            depth,
            child_index,
            target_observation_index,
            parent_alias: aliases.object_alias(&assembly.source_instance_id, parent_id),
            target_alias: aliases.object_alias(&assembly.source_instance_id, target_id),
            reference_kind,
        });
    }
    let cycle_reference_count = references
        .iter()
        .filter(|reference| reference.reference_kind == DirectAccessReferenceKind::AncestorCycle)
        .count();
    let shared_reference_count = references.len().saturating_sub(cycle_reference_count);
    let (p05_title, p09_title) = fixture_title_comparisons(&assembly.items, fixture_acceptance);
    SemanticProjection::DirectAccess {
        checkpoint_id,
        total_wire_items: assembly.items.len(),
        observation_count: nodes.len(),
        reference_count: references.len(),
        cycle_reference_count,
        shared_reference_count,
        missing_count,
        duplicate_count,
        unknown_count,
        redacted_unique_name_count,
        redacted_title_count,
        redacted_type_name_count,
        redacted_host_id_count,
        source_redacted_string_count,
        duplicate_host_id_count,
        p05_title,
        p09_title,
        nodes,
        references,
    }
}

fn fixture_title_comparisons(
    items: &[Value],
    fixture_acceptance: &FixtureAcceptance,
) -> (FixtureTitleComparison, FixtureTitleComparison) {
    let observations: Vec<Option<&str>> = items
        .iter()
        .filter_map(|item| {
            item.as_object().and_then(|object| {
                (object.get("record_kind").and_then(Value::as_str) == Some("observation"))
                    .then(|| object.get("title").and_then(Value::as_str))
            })
        })
        .collect();
    fixture_title_comparisons_from_titles(&observations, fixture_acceptance)
}

fn fixture_title_comparisons_from_titles(
    observations: &[Option<&str>],
    fixture_acceptance: &FixtureAcceptance,
) -> (FixtureTitleComparison, FixtureTitleComparison) {
    let comparison = |accepted: &str, known_variant: &dyn Fn(&str) -> bool| {
        let exact_match_count = observations
            .iter()
            .filter(|title| **title == Some(accepted))
            .count();
        let safe_known_variant_count = observations
            .iter()
            .filter_map(|title| *title)
            .filter(|title| *title != accepted && known_variant(title))
            .count();
        let missing_or_redacted_count = observations
            .iter()
            .filter(|title| title.is_none_or(|title| !fixture_title_allowed(title)))
            .count();
        FixtureTitleComparison {
            accepted_ui_title: accepted.to_owned(),
            exact_match_count,
            safe_known_variant_count,
            missing_or_redacted_count,
            target_not_observed: exact_match_count == 0 && safe_known_variant_count == 0,
        }
    };
    (
        comparison(&fixture_acceptance.p05_title.accepted_title, &|title| {
            matches!(title, P05_TITLE_NFC | P05_TITLE_NFD)
        }),
        comparison(&fixture_acceptance.p09_title.accepted_title, &|title| {
            title.len() >= P09_TITLE_PREFIX.len() && P09_TITLE_FULL.starts_with(title)
        }),
    )
}

fn safe_fixture_title(value: Option<&Value>) -> (Option<String>, SafeTitleCategory) {
    let Some(value) = value else {
        return (None, SafeTitleCategory::Unavailable);
    };
    let Some(title) = value.as_str() else {
        return if value.is_null() {
            (None, SafeTitleCategory::Unavailable)
        } else {
            (None, SafeTitleCategory::Redacted)
        };
    };
    if title.is_empty() {
        return (None, SafeTitleCategory::Empty);
    }
    if fixture_title_allowed(title) {
        (Some(title.to_owned()), SafeTitleCategory::Fixture)
    } else {
        (None, SafeTitleCategory::Redacted)
    }
}

fn fixture_title_allowed(title: &str) -> bool {
    matches!(title, P05_TITLE_NFC | P05_TITLE_NFD)
        || (title.len() >= P09_TITLE_PREFIX.len() && P09_TITLE_FULL.starts_with(title))
        || matches!(
            title,
            "CMCP_E1_ONLY_AUDIO"
                | "CMCP_E8_01"
                | "CMCP_E8_02"
                | "CMCP_E8_03"
                | "CMCP_E8_04"
                | "CMCP_E8_05"
                | "CMCP_E8_06"
                | "CMCP_E8_07"
                | "CMCP_E8_08"
                | "CMCP_01_FOLDER_EMPTY"
                | "CMCP_DUPLICATE"
                | "CMCP_04_MIDI_ASCII"
                | "CMCP_06_GROUP"
                | "CMCP_07_FX"
                | "CMCP_08_HIDDEN"
                | "CMCP_10_MUTATE_RENAME"
                | "CMCP_10_RENAMED_変更後"
                | "CMCP_11_MUTATE_DELETE"
                | "CMCP_12_MUTATION_ANCHOR"
                | "CMCP_13_STATE_S0_M0_SO0"
                | "CMCP_14_STATE_S0_M0_SO1"
                | "CMCP_15_STATE_S0_M1_SO0"
                | "CMCP_16_STATE_S0_M1_SO1"
                | "CMCP_17_STATE_S1_M0_SO0"
                | "CMCP_18_STATE_S1_M0_SO1"
                | "CMCP_19_STATE_S1_M1_SO0"
                | "CMCP_20_STATE_S1_M1_SO1"
                | "CMCP_21_ADDED"
        )
}

fn resolve_host_id(
    observation: &Map<String, Value>,
    wire_items: &[Value],
    aliases: &mut AliasState,
) -> (Option<String>, Option<usize>) {
    let byte_length = observation
        .get("host_id_byte_length")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|length| *length <= MAX_HOST_ID_BYTES);
    if let Some(raw) = observation.get("host_id_raw").and_then(Value::as_str) {
        let valid = byte_length == Some(raw.len())
            && observation
                .get("host_id_fragment_count")
                .and_then(Value::as_u64)
                == Some(0)
            && observation.get("host_id_ref").is_none_or(Value::is_null);
        return if valid {
            (Some(aliases.host_alias(raw.to_owned())), byte_length)
        } else {
            (None, None)
        };
    }
    if observation
        .get("host_id_raw")
        .is_none_or(|raw| !raw.is_null())
    {
        return (None, None);
    }
    let Some(reference) = observation
        .get("host_id_ref")
        .and_then(Value::as_str)
        .filter(|reference| !reference.is_empty() && reference.len() <= 256)
    else {
        return (None, None);
    };
    let Some(fragment_count) = observation
        .get("host_id_fragment_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|count| (1..=MAX_HOST_ID_FRAGMENTS).contains(count))
    else {
        return (None, None);
    };
    let Some(byte_length) = byte_length else {
        return (None, None);
    };
    let mut fragments: Vec<Option<&str>> = vec![None; fragment_count];
    for item in wire_items {
        let Some(fragment) = item.as_object().filter(|object| {
            object.get("record_kind").and_then(Value::as_str) == Some("host_id_fragment")
                && object.get("host_id_ref").and_then(Value::as_str) == Some(reference)
        }) else {
            continue;
        };
        let Some(index) = fragment
            .get("fragment_index")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|index| *index < fragment_count)
        else {
            return (None, None);
        };
        if fragment.get("fragment_count").and_then(Value::as_u64) != Some(fragment_count as u64)
            || fragment.get("host_id_byte_length").and_then(Value::as_u64)
                != Some(byte_length as u64)
            || fragments[index].is_some()
        {
            return (None, None);
        }
        let Some(value) = fragment.get("fragment").and_then(Value::as_str) else {
            return (None, None);
        };
        fragments[index] = Some(value);
    }
    let mut raw = String::with_capacity(byte_length);
    for fragment in fragments {
        let Some(fragment) = fragment else {
            return (None, None);
        };
        if raw.len().saturating_add(fragment.len()) > MAX_HOST_ID_BYTES {
            return (None, None);
        }
        raw.push_str(fragment);
    }
    if raw.len() != byte_length {
        return (None, None);
    }
    (Some(aliases.host_alias(raw)), Some(byte_length))
}

fn observed_direct_access(profile: Profile, evidence: &Evidence) -> AuditResult<bool> {
    validate_capabilities(profile, initial_capability_result(evidence)?)
}

fn initial_capability_result(evidence: &Evidence) -> AuditResult<&Value> {
    let init_commands = evidence
        .commands_by_checkpoint
        .get("INIT")
        .ok_or_else(|| AuditError::new("INITIAL_CAPABILITY_COMMAND_MISSING"))?;
    let mut capability_results = init_commands.iter().filter_map(|request_id| {
        evidence
            .commands
            .get(request_id)
            .filter(|command| command.method == "probe.capabilities.get")
            .and_then(|_| evidence.responses.get(request_id))
            .map(|response| &response.result)
    });
    let result = capability_results
        .next()
        .ok_or_else(|| AuditError::new("INITIAL_CAPABILITY_COMMAND_MISSING"))?;
    if capability_results.next().is_some() {
        return Err(AuditError::new("INITIAL_CAPABILITY_COMMAND_DUPLICATE"));
    }
    Ok(result)
}

fn validate_initial_capability_epoch_binding(evidence: &Evidence) -> AuditResult<()> {
    if capability_current_epoch(initial_capability_result(evidence)?)? != 0 {
        return Err(AuditError::new("INITIAL_CAPABILITY_EPOCH_INVALID"));
    }
    let mut events = evidence
        .capabilities
        .iter()
        .filter(|event| event.checkpoint_id == "INIT");
    let event = events
        .next()
        .ok_or_else(|| AuditError::new("INITIAL_CAPABILITY_EPOCH_INVALID"))?;
    if events.next().is_some() || capability_current_epoch(&event.data)? != 0 {
        return Err(AuditError::new("INITIAL_CAPABILITY_EPOCH_INVALID"));
    }
    Ok(())
}

fn validate_capabilities(profile: Profile, result: &Value) -> AuditResult<bool> {
    let object = result
        .as_object()
        .ok_or_else(|| AuditError::new("CAPABILITY_RESULT_INVALID"))?;
    if !has_exact_keys(
        object,
        &[
            "read_only",
            "integrity_failed",
            "host_version",
            "data_minimization",
            "observation_epoch",
            "mixer_bank",
            "direct_access",
            "limits",
        ],
    ) || object.get("read_only").and_then(Value::as_bool) != Some(true)
        || object.get("integrity_failed").and_then(Value::as_bool) != Some(false)
        || object.get("host_version").and_then(Value::as_str) != Some(profile.expected_host().1)
    {
        return Err(AuditError::new("CAPABILITY_RESULT_INVALID"));
    }
    validate_data_minimization(object)?;
    validate_observation_epoch_capability(object)?;
    let mixer_bank = object
        .get("mixer_bank")
        .and_then(Value::as_object)
        .ok_or_else(|| AuditError::new("CAPABILITY_RESULT_INVALID"))?;
    let configs = mixer_bank
        .get("configs")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("CAPABILITY_RESULT_INVALID"))?;
    if !has_exact_keys(
        mixer_bank,
        &[
            "supported",
            "slot_count",
            "configs",
            "title",
            "selected",
            "mute",
            "solo",
            "unique_id",
            "explicit_main_filter",
        ],
    ) || mixer_bank.get("supported").and_then(Value::as_bool) != Some(true)
        || mixer_bank.get("slot_count").and_then(Value::as_u64) != Some(8)
        || configs.len() != 2
        || configs[0].as_str() != Some("MB_CORE_ALL")
        || configs[1].as_str() != Some("MB_CORE_VISIBLE")
        || mixer_bank
            .get("explicit_main_filter")
            .and_then(Value::as_bool)
            != Some(profile.requires_direct_access())
        || ["title", "selected", "mute", "solo"]
            .iter()
            .any(|field| mixer_bank.get(*field).and_then(Value::as_bool) != Some(true))
        || mixer_bank
            .get("unique_id")
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err(AuditError::new("MIXER_BANK_CAPABILITY_INVALID"));
    }
    let direct_access = object
        .get("direct_access")
        .and_then(Value::as_object)
        .ok_or_else(|| AuditError::new("CAPABILITY_RESULT_INVALID"))?;
    if !has_exact_keys(
        direct_access,
        &[
            "supported",
            "active",
            "activation_error",
            "get_object_unique_name_v1_2",
            "get_object_unique_id_string_v1_2",
            "get_object_title_v1_2",
            "get_object_type_name_v1_3",
            "mixer_visibility_v1_2",
            "mixer_index_v1_2",
            "mixer_zone_v1_2",
            "reason",
        ],
    ) {
        return Err(AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"));
    }
    let supported = direct_access.get("supported").and_then(Value::as_bool);
    let active = direct_access.get("active").and_then(Value::as_bool);
    if supported.is_none() || supported != active {
        return Err(AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"));
    }
    let direct_access_required = supported == Some(true);
    if profile.requires_direct_access() && !direct_access_required {
        return Err(AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"));
    }
    let features = direct_access_features(direct_access)?;
    validate_direct_access_capability_codes(direct_access, direct_access_required)?;
    if (profile.requires_direct_access() && !features.type_name)
        || (!direct_access_required
            && [
                features.unique_name,
                features.unique_id,
                features.title,
                features.type_name,
                features.mixer_visibility,
                features.mixer_index,
                features.mixer_zone,
            ]
            .into_iter()
            .any(|feature| feature))
    {
        return Err(AuditError::new("DIRECT_ACCESS_FEATURE_CAPABILITY_INVALID"));
    }
    validate_capability_limits(object)?;
    Ok(direct_access_required)
}

fn validate_capability_limits(object: &Map<String, Value>) -> AuditResult<()> {
    let limits = required_object(object, "limits")?;
    let expected = [
        ("output_json_bytes", 2_048),
        ("chunk_items", 2),
        ("feedback_queue", 512),
        ("bank_snapshot_queue", 32),
        ("direct_access_snapshot_queue", 16),
        ("host_id_bytes", 4_096),
        ("host_id_fragments", 16),
        ("wire_items_per_snapshot", 1_024),
        ("direct_access_nodes", 256),
        ("direct_access_depth", 32),
        ("direct_access_children", 128),
    ];
    if limits.len() != expected.len()
        || expected
            .iter()
            .any(|(key, value)| limits.get(*key).and_then(Value::as_u64) != Some(*value))
    {
        return Err(AuditError::new("CAPABILITY_LIMITS_INVALID"));
    }
    Ok(())
}

fn validate_observation_epoch_capability(object: &Map<String, Value>) -> AuditResult<()> {
    let epoch = required_object(object, "observation_epoch")?;
    if epoch.len() != 5
        || epoch.get("supported").and_then(Value::as_bool) != Some(true)
        || epoch.get("version").and_then(Value::as_u64) != Some(1)
        || epoch.get("max").and_then(Value::as_u64) != Some(2_147_483_647)
        || epoch.get("rollover_policy").and_then(Value::as_str) != Some("reload_required")
        || epoch
            .get("current")
            .and_then(Value::as_u64)
            .is_none_or(|current| current > 2_147_483_647)
    {
        return Err(AuditError::new("OBSERVATION_EPOCH_CAPABILITY_INVALID"));
    }
    Ok(())
}

fn validate_data_minimization(object: &Map<String, Value>) -> AuditResult<()> {
    let data = required_object(object, "data_minimization")?;
    if data.len() != 6
        || data.get("source_redaction").and_then(Value::as_bool) != Some(true)
        || data.get("fixture_revision").and_then(Value::as_u64) != Some(FIXTURE_REVISION as u64)
        || data.get("unknown_titles").and_then(Value::as_str) != Some("redacted")
        || data.get("unknown_host_ids").and_then(Value::as_str) != Some("omitted")
        || data.get("unique_name_policy").and_then(Value::as_str) != Some("not_invoked")
        || data.get("exception_text").and_then(Value::as_str) != Some("fixed_codes")
    {
        return Err(AuditError::new("DATA_MINIMIZATION_CAPABILITY_INVALID"));
    }
    Ok(())
}

fn validate_direct_access_capability_codes(
    direct_access: &Map<String, Value>,
    active: bool,
) -> AuditResult<()> {
    let reason = direct_access
        .get("reason")
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"))?;
    let activation_error = direct_access
        .get("activation_error")
        .ok_or_else(|| AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"))?;
    let reason_valid = if active {
        reason.is_null()
    } else {
        matches!(
            reason.as_str(),
            Some(
                "make_direct_access_unavailable"
                    | "make_direct_access_failed"
                    | "core_methods_incomplete"
            )
        )
    };
    if !reason_valid || !activation_error.is_null() {
        return Err(AuditError::new("DIRECT_ACCESS_CAPABILITY_INVALID"));
    }
    Ok(())
}

struct DirectAccessFeatures {
    unique_name: bool,
    unique_id: bool,
    title: bool,
    type_name: bool,
    mixer_visibility: bool,
    mixer_index: bool,
    mixer_zone: bool,
}

fn direct_access_features(object: &Map<String, Value>) -> AuditResult<DirectAccessFeatures> {
    Ok(DirectAccessFeatures {
        unique_name: required_bool(object, "get_object_unique_name_v1_2")?,
        unique_id: required_bool(object, "get_object_unique_id_string_v1_2")?,
        title: required_bool(object, "get_object_title_v1_2")?,
        type_name: required_bool(object, "get_object_type_name_v1_3")?,
        mixer_visibility: required_bool(object, "mixer_visibility_v1_2")?,
        mixer_index: required_bool(object, "mixer_index_v1_2")?,
        mixer_zone: required_bool(object, "mixer_zone_v1_2")?,
    })
}

fn capability_report(result: &Value) -> AuditResult<CapabilityReport> {
    let object = result
        .as_object()
        .ok_or_else(|| AuditError::new("CAPABILITY_RESULT_INVALID"))?;
    let mixer = required_object(object, "mixer_bank")?;
    let direct = required_object(object, "direct_access")?;
    let features = direct_access_features(direct)?;
    validate_data_minimization(object)?;
    validate_observation_epoch_capability(object)?;
    Ok(CapabilityReport {
        read_only: required_bool(object, "read_only")?,
        integrity_failed: required_bool(object, "integrity_failed")?,
        data_minimization: DataMinimizationCapabilityReport {
            source_redaction: true,
            fixture_revision: FIXTURE_REVISION as u64,
            unknown_titles: "redacted",
            unknown_host_ids: "omitted",
            unique_name_policy: "not_invoked",
            exception_text: "fixed_codes",
        },
        observation_epoch: ObservationEpochCapabilityReport {
            supported: true,
            version: 1,
            maximum: 2_147_483_647,
            rollover_policy: "reload_required",
        },
        mixer_bank: MixerBankCapabilityReport {
            supported: required_bool(mixer, "supported")?,
            slot_count: required_u64(mixer, "slot_count")?,
            core_all: mixer
                .get("configs")
                .and_then(Value::as_array)
                .is_some_and(|configs| {
                    configs
                        .iter()
                        .any(|config| config.as_str() == Some("MB_CORE_ALL"))
                }),
            core_visible: mixer
                .get("configs")
                .and_then(Value::as_array)
                .is_some_and(|configs| {
                    configs
                        .iter()
                        .any(|config| config.as_str() == Some("MB_CORE_VISIBLE"))
                }),
            title: required_bool(mixer, "title")?,
            selected: required_bool(mixer, "selected")?,
            mute: required_bool(mixer, "mute")?,
            solo: required_bool(mixer, "solo")?,
            unique_id: required_bool(mixer, "unique_id")?,
            explicit_main_filter: required_bool(mixer, "explicit_main_filter")?,
        },
        direct_access: DirectAccessCapabilityReport {
            supported: required_bool(direct, "supported")?,
            active: required_bool(direct, "active")?,
            unique_name: features.unique_name,
            unique_name_policy: "not_invoked",
            unique_id: features.unique_id,
            title: features.title,
            type_name: features.type_name,
            mixer_visibility: features.mixer_visibility,
            mixer_index: features.mixer_index,
            mixer_zone: features.mixer_zone,
        },
    })
}

fn validate_summary(evidence: &Evidence) -> AuditResult<()> {
    let summary = evidence
        .summary
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| AuditError::new("COLLECTOR_SUMMARY_MISSING"))?;
    if !required_bool(summary, "integrity_ok")? || !required_bool(summary, "exit_ok")? {
        return Err(AuditError::new("COLLECTOR_SUMMARY_FAILED"));
    }
    if summary.get("exit_reason").and_then(Value::as_str) != Some("stdin_eof") {
        return Err(AuditError::new("COLLECTOR_SUMMARY_EXIT_REASON_INVALID"));
    }
    if summary.get("session_id").and_then(Value::as_str) != evidence.session_id.as_deref() {
        return Err(AuditError::new("COLLECTOR_SESSION_MISMATCH"));
    }
    let commands = required_object(summary, "commands")?;
    if !has_exact_keys(
        commands,
        &["received", "sent", "local", "deferred", "rejected"],
    ) {
        return Err(AuditError::new("COLLECTOR_COMMAND_SUMMARY_INVALID"));
    }
    let sent = required_u64(commands, "sent")?;
    let local = required_u64(commands, "local")?;
    if sent != evidence.commands.len() as u64
        || local
            != (REQUIRED_CHECKPOINTS.len() * 2
                + REQUIRED_CHECKPOINTS
                    .iter()
                    .filter(|id| !requires_observation_cut(id))
                    .count()) as u64
        || required_u64(commands, "received")? != sent.saturating_add(local)
        || required_u64(commands, "deferred")? != 0
        || required_u64(commands, "rejected")? != 0
    {
        return Err(AuditError::new("COLLECTOR_COMMAND_SUMMARY_INVALID"));
    }
    let drain = required_object(summary, "graceful_drain")?;
    if !has_exact_keys(drain, &["completed", "timed_out", "duration_ms"]) {
        return Err(AuditError::new("COLLECTOR_DRAIN_INVALID"));
    }
    let drain_completed = evidence
        .drain_completed
        .ok_or_else(|| AuditError::new("COLLECTOR_DRAIN_INVALID"))?;
    if !required_bool(drain, "completed")?
        || required_bool(drain, "timed_out")?
        || required_u64(drain, "duration_ms")? != drain_completed.duration_ms
        || required_bool(drain, "completed")? != drain_completed.completed
        || required_bool(drain, "timed_out")? != drain_completed.timed_out
    {
        return Err(AuditError::new("COLLECTOR_DRAIN_INVALID"));
    }
    if required_u64(summary, "orphan_messages")? != 0 {
        return Err(AuditError::new("ORPHAN_MESSAGES_OBSERVED"));
    }
    let protocol = required_object(summary, "protocol_tracking")?;
    if !has_exact_keys(
        protocol,
        &[
            "completed_requests",
            "completed_chunk_streams",
            "completed_snapshot_streams",
            "completed_feedback_streams",
            "completed_checkpoints",
            "checkpoint_messages",
            "checkpoint_messages_processed_after_end",
            "orphan_messages",
            "pending_requests",
            "expected_followups",
            "open_snapshots",
            "selected_source_instance_id",
            "active_source_instance_ids",
        ],
    ) {
        return Err(AuditError::new("PROTOCOL_SUMMARY_INVALID"));
    }
    for key in [
        "pending_requests",
        "expected_followups",
        "open_snapshots",
        "orphan_messages",
    ] {
        if required_u64(protocol, key)? != 0 {
            return Err(AuditError::new("PROTOCOL_WORK_INCOMPLETE"));
        }
    }
    let selected_source = protocol
        .get("selected_source_instance_id")
        .and_then(Value::as_str)
        .filter(|source| !source.is_empty())
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_INVALID"))?;
    let active_sources = protocol
        .get("active_source_instance_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_INVALID"))?;
    if required_u64(protocol, "completed_checkpoints")? != REQUIRED_CHECKPOINTS.len() as u64
        || required_u64(protocol, "completed_requests")? != evidence.commands.len() as u64
        || required_u64(protocol, "completed_chunk_streams")?
            != evidence.completed_chunk_streams as u64
        || required_u64(protocol, "completed_snapshot_streams")? != evidence.snapshots.len() as u64
        || required_u64(protocol, "completed_feedback_streams")?
            != evidence
                .completed_chunk_streams
                .saturating_sub(evidence.snapshots.len()) as u64
        || required_u64(protocol, "checkpoint_messages")? != evidence.probe_records as u64
        || required_u64(protocol, "checkpoint_messages_processed_after_end")? != 0
        || active_sources.len() != 1
        || active_sources[0].as_str() != Some(selected_source)
    {
        return Err(AuditError::new("PROTOCOL_SUMMARY_INVALID"));
    }
    validate_summary_r2_source_binding(evidence, selected_source)?;
    let incoming = required_object(summary, "incoming")?;
    if !has_exact_keys(
        incoming,
        &[
            "frames",
            "messages",
            "events",
            "responses",
            "errors",
            "diagnostics",
            "parse_errors",
            "oversize_frames",
            "source_overflows",
            "queue_drops",
            "sequence_gaps",
            "sequence_duplicates_or_reorders",
            "sources",
        ],
    ) {
        return Err(AuditError::new("INCOMING_SUMMARY_SCHEMA_INVALID"));
    }
    for key in [
        "errors",
        "diagnostics",
        "parse_errors",
        "oversize_frames",
        "source_overflows",
        "queue_drops",
        "sequence_gaps",
        "sequence_duplicates_or_reorders",
    ] {
        if required_u64(incoming, key)? != 0 {
            return Err(AuditError::new("INCOMING_INTEGRITY_SUMMARY_INVALID"));
        }
    }
    if required_u64(incoming, "messages")? != evidence.probe_records as u64
        || required_u64(incoming, "responses")? != evidence.responses.len() as u64
        || required_u64(incoming, "events")?
            != evidence
                .probe_records
                .saturating_sub(evidence.responses.len()) as u64
    {
        return Err(AuditError::new("INCOMING_MESSAGE_COUNT_MISMATCH"));
    }
    if required_u64(incoming, "frames")? != required_u64(incoming, "messages")? {
        return Err(AuditError::new("INCOMING_FRAME_COUNT_INVALID"));
    }
    let sources = incoming
        .get("sources")
        .and_then(Value::as_array)
        .ok_or_else(|| AuditError::new("INCOMING_SOURCE_SUMMARY_INVALID"))?;
    let mut source_set = HashSet::new();
    for source in sources {
        let source = source
            .as_object()
            .filter(|source| has_exact_keys(source, &["source_instance_id", "last_source_seq"]))
            .ok_or_else(|| AuditError::new("INCOMING_SOURCE_SUMMARY_INVALID"))?;
        let source_id = required_string(source, "source_instance_id", 256)?;
        let last_seq = required_u64(source, "last_source_seq")?;
        if !source_set.insert(source_id)
            || evidence.last_source_seq.get(source_id).copied() != Some(last_seq)
        {
            return Err(AuditError::new("INCOMING_SOURCE_SUMMARY_INVALID"));
        }
    }
    if source_set.len() != evidence.probe_source_ids.len()
        || !evidence
            .probe_source_ids
            .iter()
            .all(|source| source_set.contains(source.as_str()))
    {
        return Err(AuditError::new("INCOMING_SOURCE_SUMMARY_INVALID"));
    }
    Ok(())
}

fn validate_summary_r2_source_binding(
    evidence: &Evidence,
    selected_source: &str,
) -> AuditResult<()> {
    let mut loaded = evidence
        .loaded
        .iter()
        .filter(|event| event.checkpoint_id == "R2");
    let r2_loaded = loaded
        .next()
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    if loaded.next().is_some() || r2_loaded.source_instance_id != selected_source {
        return Err(AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"));
    }
    let command_ids = evidence
        .commands_by_checkpoint
        .get("R2")
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    let mut discoveries = command_ids.iter().filter_map(|request_id| {
        evidence
            .commands
            .get(request_id)
            .filter(|command| command.method == "probe.discover")
            .and_then(|_| evidence.discoveries.get(request_id))
    });
    let discovery = discoveries
        .next()
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    if discoveries.next().is_some() || discovery.selected_source_instance_id != selected_source {
        return Err(AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"));
    }
    let final_request_id = command_ids
        .last()
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    let final_command = evidence
        .commands
        .get(final_request_id)
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    let (kind, reason) = command_followup(&final_command.method)
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    let final_snapshot = matching_snapshot(evidence, final_command, kind, reason)
        .ok_or_else(|| AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"))?;
    if final_command.target_instance_id.as_deref() != Some(selected_source)
        || final_snapshot.source_instance_id != selected_source
    {
        return Err(AuditError::new("PROTOCOL_SUMMARY_R2_SOURCE_INVALID"));
    }
    Ok(())
}

fn validate_drain_boundary(records: &[RawRecord], evidence: &Evidence) -> AuditResult<()> {
    let started = evidence
        .drain_started
        .ok_or_else(|| AuditError::new("DRAIN_STARTED_MISSING"))?;
    let completed = evidence
        .drain_completed
        .ok_or_else(|| AuditError::new("DRAIN_COMPLETED_MISSING"))?;
    let last_checkpoint_end = evidence
        .checkpoints
        .get("R2")
        .and_then(|checkpoint| checkpoint.end)
        .ok_or_else(|| AuditError::new("CHECKPOINT_END_MISSING"))?;
    let summary_index = records
        .last()
        .filter(|record| record.record_type == "collector_summary")
        .map(|record| record.index)
        .ok_or_else(|| AuditError::new("COLLECTOR_BOUNDARY_ORDER_INVALID"))?;
    let elapsed = completed
        .marker
        .monotonic_timestamp_ms
        .saturating_sub(started.marker.monotonic_timestamp_ms);
    if started.timeout_ms != 5_000
        || started.deadline_monotonic_timestamp_ms
            != started
                .marker
                .monotonic_timestamp_ms
                .saturating_add(started.timeout_ms)
        || last_checkpoint_end.record_index >= started.marker.record_index
        || started.marker.record_index >= completed.marker.record_index
        || completed.marker.record_index >= summary_index
        || last_checkpoint_end.monotonic_timestamp_ms > started.marker.monotonic_timestamp_ms
        || started.marker.monotonic_timestamp_ms > completed.marker.monotonic_timestamp_ms
        || !completed.completed
        || completed.timed_out
        || completed.duration_ms > started.timeout_ms.saturating_add(1_000)
        || elapsed < completed.duration_ms
        || elapsed.saturating_sub(completed.duration_ms) > 1_000
    {
        return Err(AuditError::new("COLLECTOR_DRAIN_BOUNDARY_INVALID"));
    }
    Ok(())
}

fn validate_reconnects(
    profile: Profile,
    direct_access_required: bool,
    evidence: &Evidence,
) -> AuditResult<Vec<ReconnectReport>> {
    let (mut previous_source, _) =
        validate_activation(profile, direct_access_required, evidence, "INIT", None)?;
    let mut reports = Vec::new();
    let mut same_script_reactivation_count = 0usize;
    for checkpoint_id in REQUIRED_CHECKPOINTS
        .into_iter()
        .filter(|checkpoint_id| !matches!(*checkpoint_id, "INIT" | "R1" | "R2"))
    {
        if checkpoint_has_same_script_reactivation(evidence, checkpoint_id) {
            validate_same_script_reactivation(
                profile,
                direct_access_required,
                evidence,
                checkpoint_id,
                &previous_source,
            )?;
            same_script_reactivation_count += 1;
        }
    }
    for checkpoint_id in ["R1", "R2"] {
        let action = evidence
            .actions
            .get(checkpoint_id)
            .copied()
            .ok_or_else(|| AuditError::new("RECONNECT_ACTION_MISSING"))?;
        let (source, ready_at) = validate_activation(
            profile,
            direct_access_required,
            evidence,
            checkpoint_id,
            Some(&previous_source),
        )?;
        let reconnect_deadline = action
            .monotonic_timestamp_ms
            .saturating_add(RECONNECT_DEADLINE_MS);
        let command_ids = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .ok_or_else(|| AuditError::new("RECONNECT_COMMANDS_MISSING"))?;
        let final_command = evidence
            .commands
            .get(
                command_ids
                    .last()
                    .ok_or_else(|| AuditError::new("RECONNECT_COMMANDS_MISSING"))?,
            )
            .ok_or_else(|| AuditError::new("RECONNECT_COMMANDS_MISSING"))?;
        let (kind, reason) = command_followup(&final_command.method)
            .ok_or_else(|| AuditError::new("RECONNECT_FINAL_SNAPSHOT_MISSING"))?;
        let snapshot = matching_snapshot(evidence, final_command, kind, reason)
            .filter(|snapshot| snapshot.source_instance_id == source)
            .ok_or_else(|| AuditError::new("RECONNECT_FINAL_SNAPSHOT_MISSING"))?;
        let discovery = command_ids
            .iter()
            .filter_map(|request_id| evidence.discoveries.get(request_id))
            .find(|discovery| {
                discovery.selected_source_instance_id == source
                    && discovery.monotonic_timestamp_ms >= action.monotonic_timestamp_ms
                    && discovery.monotonic_timestamp_ms <= reconnect_deadline
            })
            .ok_or_else(|| AuditError::new("RECONNECT_DISCOVERY_SOURCE_MISMATCH"))?;
        let reconnect_complete = ready_at.max(discovery.monotonic_timestamp_ms);
        if snapshot.received_monotonic_timestamp_ms < reconnect_complete
            || snapshot
                .received_monotonic_timestamp_ms
                .saturating_sub(reconnect_complete)
                > RECONNECT_FINALIZATION_MS
        {
            return Err(AuditError::new("RECONNECT_FINAL_SNAPSHOT_TOO_LATE"));
        }
        reports.push(ReconnectReport {
            phase: checkpoint_id,
            ready_elapsed_ms: ready_at.saturating_sub(action.monotonic_timestamp_ms),
            discovery_elapsed_ms: discovery
                .monotonic_timestamp_ms
                .saturating_sub(action.monotonic_timestamp_ms),
            final_snapshot_elapsed_ms: snapshot
                .received_monotonic_timestamp_ms
                .saturating_sub(action.monotonic_timestamp_ms),
        });
        previous_source = source;
    }
    if evidence.loaded.len() != 3
        || evidence.mapping_active.len() != 3 + same_script_reactivation_count
        || evidence.capabilities.len() != 3 + same_script_reactivation_count
        || evidence.ready.len() != 3 + same_script_reactivation_count
        || evidence.not_ready.len() != 2 + same_script_reactivation_count
        || evidence.probe_source_ids.len() != 3
    {
        return Err(AuditError::new("ACTIVATION_EVIDENCE_COUNT_INVALID"));
    }
    Ok(reports)
}

fn checkpoint_has_same_script_reactivation(
    evidence: &Evidence,
    checkpoint_id: &'static str,
) -> bool {
    if matches!(checkpoint_id, "INIT" | "R1" | "R2") {
        return false;
    }
    evidence
        .mapping_active
        .iter()
        .any(|event| event.checkpoint_id == checkpoint_id)
        || evidence
            .capabilities
            .iter()
            .any(|event| event.checkpoint_id == checkpoint_id)
        || evidence
            .ready
            .iter()
            .any(|event| event.checkpoint_id == checkpoint_id)
        || evidence
            .not_ready
            .iter()
            .any(|event| event.checkpoint_id == checkpoint_id)
        || evidence.snapshots.iter().any(|snapshot| {
            snapshot.checkpoint_id == checkpoint_id && snapshot.reason == "page_activate"
        })
        || evidence
            .loaded
            .iter()
            .any(|event| event.checkpoint_id == checkpoint_id)
}

fn validate_same_script_reactivation(
    profile: Profile,
    direct_access_required: bool,
    evidence: &Evidence,
    checkpoint_id: &'static str,
    expected_source: &str,
) -> AuditResult<()> {
    let action = evidence
        .actions
        .get(checkpoint_id)
        .copied()
        .ok_or_else(|| AuditError::new("ACTIVATION_ACTION_MISSING"))?;
    let checkpoint_end = evidence
        .checkpoints
        .get(checkpoint_id)
        .and_then(|checkpoint| checkpoint.end)
        .ok_or_else(|| AuditError::new("CHECKPOINT_END_MISSING"))?;
    if evidence
        .loaded
        .iter()
        .any(|event| event.checkpoint_id == checkpoint_id)
    {
        return Err(AuditError::new(
            "SAME_SCRIPT_REACTIVATION_LOADED_UNEXPECTED",
        ));
    }
    let exactly_one = |events: &[LifecycleEvidence], after_index: usize| {
        let mut matches = events.iter().filter(|event| {
            event.checkpoint_id == checkpoint_id
                && event.source_instance_id == expected_source
                && event.index > after_index
                && event.index < checkpoint_end.record_index
                && event.monotonic_timestamp_ms >= action.monotonic_timestamp_ms
                && event.monotonic_timestamp_ms <= checkpoint_end.monotonic_timestamp_ms
        });
        let first = matches
            .next()
            .map(|event| (event.index, event.monotonic_timestamp_ms));
        (first, matches.next().is_none())
    };
    let (inactive, inactive_unique) = exactly_one(&evidence.not_ready, action.record_index);
    let inactive = inactive.ok_or_else(|| AuditError::new("REACTIVATION_NOT_READY_MISSING"))?;
    if !inactive_unique {
        return Err(AuditError::new("REACTIVATION_NOT_READY_DUPLICATE"));
    }
    if requires_observation_cut(checkpoint_id) {
        let cut_before_inactive = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .into_iter()
            .flatten()
            .filter_map(|request_id| evidence.commands.get(request_id))
            .filter(|command| command.method == "probe.observation.cut")
            .all(|command| command.index < inactive.0);
        if !cut_before_inactive {
            return Err(AuditError::new("OBSERVATION_CUT_AFTER_REACTIVATION"));
        }
    }
    if let Some((_, operation)) = bank_operation(checkpoint_id) {
        let command_ids = evidence
            .commands_by_checkpoint
            .get(checkpoint_id)
            .ok_or_else(|| AuditError::new("REACTIVATION_COMMANDS_MISSING"))?;
        let mut operation_ids = command_ids.iter().filter(|request_id| {
            evidence
                .commands
                .get(*request_id)
                .is_some_and(|command| command.method == operation)
        });
        let request_id = operation_ids
            .next()
            .ok_or_else(|| AuditError::new("REACTIVATION_BANK_ACTION_MISSING"))?;
        if operation_ids.next().is_some() {
            return Err(AuditError::new("REACTIVATION_BANK_ACTION_DUPLICATE"));
        }
        let command = evidence
            .commands
            .get(request_id)
            .ok_or_else(|| AuditError::new("REACTIVATION_BANK_ACTION_MISSING"))?;
        let response = evidence
            .responses
            .get(request_id)
            .ok_or_else(|| AuditError::new("REACTIVATION_BANK_ACTION_RESPONSE_MISSING"))?;
        let (_, reason) = command_followup(operation)
            .ok_or_else(|| AuditError::new("REACTIVATION_BANK_ACTION_SNAPSHOT_MISSING"))?;
        let snapshot = matching_snapshot(evidence, command, SnapshotKind::Bank, reason)
            .ok_or_else(|| AuditError::new("REACTIVATION_BANK_ACTION_SNAPSHOT_MISSING"))?;
        if response.record_index >= inactive.0
            || snapshot.record_index >= inactive.0
            || response.received_monotonic_timestamp_ms > inactive.1
            || snapshot.received_monotonic_timestamp_ms > inactive.1
        {
            return Err(AuditError::new("REACTIVATION_BANK_ACTION_NOT_COMPLETED"));
        }
    }
    let (mapping, mapping_unique) = exactly_one(&evidence.mapping_active, inactive.0);
    let mapping = mapping.ok_or_else(|| AuditError::new("REACTIVATION_MAPPING_MISSING"))?;
    if !mapping_unique {
        return Err(AuditError::new("REACTIVATION_MAPPING_DUPLICATE"));
    }
    let mut capability_matches = evidence.capabilities.iter().filter(|event| {
        event.checkpoint_id == checkpoint_id
            && event.source_instance_id == expected_source
            && event.index > mapping.0
            && event.index < checkpoint_end.record_index
    });
    let capability = capability_matches
        .next()
        .ok_or_else(|| AuditError::new("REACTIVATION_CAPABILITY_MISSING"))?;
    if capability_matches.next().is_some()
        || validate_capabilities(profile, &capability.data)? != direct_access_required
        || capability_report(&capability.data)?
            != capability_report(initial_capability_result(evidence)?)?
        || capability_current_epoch(&capability.data)?
            != latest_cut_epoch_through_checkpoint(evidence, checkpoint_id)?
    {
        return Err(AuditError::new("REACTIVATION_CAPABILITY_INVALID"));
    }
    let snapshots: Vec<_> = evidence
        .snapshots
        .iter()
        .filter(|snapshot| {
            snapshot.checkpoint_id == checkpoint_id
                && snapshot.source_instance_id == expected_source
                && snapshot.reason == "page_activate"
        })
        .collect();
    if snapshots.len() != if direct_access_required { 3 } else { 2 } {
        return Err(AuditError::new("REACTIVATION_SNAPSHOT_COUNT_INVALID"));
    }
    let unique_snapshot = |kind: SnapshotKind, config_id: Option<&str>| {
        let mut matches = snapshots
            .iter()
            .copied()
            .filter(|snapshot| snapshot.kind == kind && snapshot.config_id.as_deref() == config_id);
        let first = matches.next();
        (first, matches.next().is_none())
    };
    let (all, all_unique) = unique_snapshot(SnapshotKind::Bank, Some("MB_CORE_ALL"));
    let all = all.ok_or_else(|| AuditError::new("REACTIVATION_CORE_ALL_SNAPSHOT_MISSING"))?;
    let (visible, visible_unique) = unique_snapshot(SnapshotKind::Bank, Some("MB_CORE_VISIBLE"));
    let visible =
        visible.ok_or_else(|| AuditError::new("REACTIVATION_CORE_VISIBLE_SNAPSHOT_MISSING"))?;
    let direct = if direct_access_required {
        let (direct, direct_unique) = unique_snapshot(SnapshotKind::DirectAccess, None);
        if !direct_unique {
            return Err(AuditError::new("REACTIVATION_DIRECT_SNAPSHOT_DUPLICATE"));
        }
        Some(direct.ok_or_else(|| AuditError::new("REACTIVATION_DIRECT_SNAPSHOT_MISSING"))?)
    } else {
        None
    };
    if !all_unique
        || !visible_unique
        || all.record_index <= capability.index
        || visible.record_index <= all.record_index
        || direct.is_some_and(|snapshot| snapshot.record_index <= visible.record_index)
    {
        return Err(AuditError::new("REACTIVATION_SNAPSHOT_ORDER_INVALID"));
    }
    let last_snapshot_index = direct.map_or(visible.record_index, |snapshot| snapshot.record_index);
    let (ready, ready_unique) = exactly_one(&evidence.ready, last_snapshot_index);
    let ready = ready.ok_or_else(|| AuditError::new("REACTIVATION_READY_MISSING"))?;
    if !ready_unique {
        return Err(AuditError::new("REACTIVATION_READY_DUPLICATE"));
    }
    let command_ids = evidence
        .commands_by_checkpoint
        .get(checkpoint_id)
        .ok_or_else(|| AuditError::new("REACTIVATION_COMMANDS_MISSING"))?;
    let discover_ids: Vec<_> = command_ids
        .iter()
        .filter(|request_id| {
            evidence
                .commands
                .get(*request_id)
                .is_some_and(|command| command.method == "probe.discover")
        })
        .collect();
    if discover_ids.len() != 1 {
        return Err(AuditError::new("REACTIVATION_DISCOVERY_INVALID"));
    }
    let discover_command = evidence
        .commands
        .get(discover_ids[0])
        .ok_or_else(|| AuditError::new("REACTIVATION_DISCOVERY_INVALID"))?;
    let discovery = evidence
        .discoveries
        .get(discover_ids[0])
        .ok_or_else(|| AuditError::new("REACTIVATION_DISCOVERY_INVALID"))?;
    if discover_command.index <= ready.0
        || discovery.selected_source_instance_id != expected_source
        || discovery.monotonic_timestamp_ms < ready.1
    {
        return Err(AuditError::new("REACTIVATION_DISCOVERY_INVALID"));
    }
    if command_ids.iter().any(|request_id| {
        evidence.commands.get(request_id).is_none_or(|command| {
            command.method != "probe.observation.cut"
                && bank_operation(checkpoint_id)
                    .is_none_or(|(_, operation)| command.method != operation)
                && (command.index <= ready.0
                    || evidence
                        .send_results
                        .get(request_id)
                        .is_none_or(|send| send.completed_monotonic_timestamp_ms < ready.1))
        })
    }) {
        return Err(AuditError::new("COMMAND_BEFORE_REACTIVATION_READY"));
    }
    Ok(())
}

fn capability_current_epoch(value: &Value) -> AuditResult<u64> {
    value
        .as_object()
        .and_then(|object| object.get("observation_epoch"))
        .and_then(Value::as_object)
        .and_then(|epoch| epoch.get("current"))
        .and_then(Value::as_u64)
        .ok_or_else(|| AuditError::new("OBSERVATION_EPOCH_CAPABILITY_INVALID"))
}

fn latest_cut_epoch_through_checkpoint(
    evidence: &Evidence,
    checkpoint_id: &'static str,
) -> AuditResult<u64> {
    let mut current = capability_current_epoch(initial_capability_result(evidence)?)?;
    for candidate in REQUIRED_CHECKPOINTS {
        if let Some(epoch) = checkpoint_cut_epoch(evidence, candidate) {
            current = epoch;
        }
        if candidate == checkpoint_id {
            return Ok(current);
        }
    }
    Err(AuditError::new("CHECKPOINT_MISSING"))
}

fn validate_activation(
    profile: Profile,
    direct_access_required: bool,
    evidence: &Evidence,
    checkpoint_id: &'static str,
    previous_source: Option<&str>,
) -> AuditResult<(String, u64)> {
    let action = evidence
        .actions
        .get(checkpoint_id)
        .copied()
        .ok_or_else(|| AuditError::new("ACTIVATION_ACTION_MISSING"))?;
    let deadline = if checkpoint_id == "INIT" {
        u64::MAX
    } else {
        action
            .monotonic_timestamp_ms
            .saturating_add(RECONNECT_DEADLINE_MS)
    };
    let mut loaded_matches = evidence.loaded.iter().filter(|event| {
        event.checkpoint_id == checkpoint_id
            && event.source_seq == 1
            && event.index > action.record_index
            && event.monotonic_timestamp_ms >= action.monotonic_timestamp_ms
            && event.monotonic_timestamp_ms <= deadline
            && previous_source.is_none_or(|previous| event.source_instance_id != previous)
    });
    let loaded = loaded_matches
        .next()
        .ok_or_else(|| AuditError::new("ACTIVATION_NEW_SESSION_MISSING"))?;
    if loaded_matches.next().is_some() {
        return Err(AuditError::new("ACTIVATION_NEW_SESSION_DUPLICATE"));
    }
    if let Some(previous_source) = previous_source {
        let mut inactive = evidence.not_ready.iter().filter(|event| {
            event.checkpoint_id == checkpoint_id
                && event.source_instance_id == previous_source
                && event.index > action.record_index
                && event.index < loaded.index
                && event.monotonic_timestamp_ms >= action.monotonic_timestamp_ms
        });
        if inactive.next().is_none() || inactive.next().is_some() {
            return Err(AuditError::new(
                "PREVIOUS_SESSION_INACTIVE_EVIDENCE_INVALID",
            ));
        }
    }
    let mut mappings = evidence.mapping_active.iter().filter(|event| {
        event.checkpoint_id == checkpoint_id
            && event.source_instance_id == loaded.source_instance_id
            && event.index > loaded.index
            && event.monotonic_timestamp_ms <= deadline
    });
    let mapping = mappings
        .next()
        .ok_or_else(|| AuditError::new("ACTIVATION_MAPPING_MISSING"))?;
    if mappings.next().is_some() {
        return Err(AuditError::new("ACTIVATION_MAPPING_DUPLICATE"));
    }
    let mut capabilities = evidence.capabilities.iter().filter(|event| {
        event.checkpoint_id == checkpoint_id
            && event.source_instance_id == loaded.source_instance_id
            && event.index > mapping.index
            && event.monotonic_timestamp_ms <= deadline
    });
    let capability = capabilities
        .next()
        .ok_or_else(|| AuditError::new("ACTIVATION_CAPABILITY_MISSING"))?;
    if capabilities.next().is_some()
        || validate_capabilities(profile, &capability.data)? != direct_access_required
        || capability_report(&capability.data)?
            != capability_report(initial_capability_result(evidence)?)?
        || capability_current_epoch(&capability.data)? != 0
    {
        return Err(AuditError::new("ACTIVATION_CAPABILITY_INVALID"));
    }
    let expected_initial_snapshots = if direct_access_required { 3 } else { 2 };
    let initial_snapshots: Vec<_> = evidence
        .snapshots
        .iter()
        .filter(|snapshot| {
            snapshot.checkpoint_id == checkpoint_id
                && snapshot.source_instance_id == loaded.source_instance_id
                && snapshot.reason == "page_activate"
        })
        .collect();
    if initial_snapshots.len() != expected_initial_snapshots {
        return Err(AuditError::new("ACTIVATION_SNAPSHOT_COUNT_INVALID"));
    }
    let required_snapshot = |kind: SnapshotKind, config: Option<&str>| {
        let mut matches = initial_snapshots
            .iter()
            .copied()
            .filter(|snapshot| snapshot.kind == kind && snapshot.config_id.as_deref() == config);
        let snapshot = matches.next()?;
        matches.next().is_none().then_some(snapshot)
    };
    let all = required_snapshot(SnapshotKind::Bank, Some("MB_CORE_ALL"))
        .ok_or_else(|| AuditError::new("ACTIVATION_CORE_ALL_SNAPSHOT_MISSING"))?;
    let visible = required_snapshot(SnapshotKind::Bank, Some("MB_CORE_VISIBLE"))
        .ok_or_else(|| AuditError::new("ACTIVATION_CORE_VISIBLE_SNAPSHOT_MISSING"))?;
    let direct = if direct_access_required {
        Some(
            required_snapshot(SnapshotKind::DirectAccess, None)
                .ok_or_else(|| AuditError::new("ACTIVATION_DIRECT_SNAPSHOT_MISSING"))?,
        )
    } else {
        None
    };
    let last_snapshot_index = direct
        .map_or(visible.record_index, |snapshot| snapshot.record_index)
        .max(all.record_index)
        .max(visible.record_index);
    if all.record_index <= capability.index
        || visible.record_index <= all.record_index
        || direct.is_some_and(|snapshot| snapshot.record_index <= visible.record_index)
    {
        return Err(AuditError::new("ACTIVATION_SNAPSHOT_ORDER_INVALID"));
    }
    let mut ready_matches = evidence.ready.iter().filter(|event| {
        event.checkpoint_id == checkpoint_id
            && event.source_instance_id == loaded.source_instance_id
            && event.index > last_snapshot_index
            && event.monotonic_timestamp_ms <= deadline
    });
    let ready = ready_matches
        .next()
        .ok_or_else(|| AuditError::new("ACTIVATION_READY_MISSING"))?;
    if ready_matches.next().is_some() {
        return Err(AuditError::new("ACTIVATION_READY_DUPLICATE"));
    }
    if evidence
        .commands_by_checkpoint
        .get(checkpoint_id)
        .into_iter()
        .flatten()
        .any(|request_id| {
            evidence.commands.get(request_id).is_none_or(|command| {
                command.index <= ready.index
                    || evidence.send_results.get(request_id).is_none_or(|send| {
                        send.completed_monotonic_timestamp_ms < ready.monotonic_timestamp_ms
                    })
            })
        })
    {
        return Err(AuditError::new("COMMAND_BEFORE_ACTIVATION_READY"));
    }
    Ok((
        loaded.source_instance_id.clone(),
        ready.monotonic_timestamp_ms,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RUN_ID: &str = "audit-test-run";
    const TEST_UNIX_BASE: u64 = 1_800_000_000_000;
    const PROBE_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const COLLECTOR_HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const COMMIT_HASH: &str = "cccccccccccccccccccccccccccccccccccccccc";

    fn direct_access_test_observation(
        observation_epoch: u64,
        object_id: u64,
        parent_id: Option<u64>,
        depth: u64,
        child_index: Option<u64>,
        child_count: u64,
    ) -> Value {
        json!({
            "record_kind": "observation",
            "observation_epoch": observation_epoch,
            "observation_epoch_status": "snapshot_observed",
            "object_id": object_id,
            "parent_id": parent_id,
            "depth": depth,
            "child_index": child_index,
            "unique_name": null,
            "unique_name_redacted": true,
            "unique_name_status": "not_invoked_by_policy",
            "host_id_raw": null,
            "host_id_redacted": false,
            "host_id_byte_length": null,
            "host_id_ref": null,
            "host_id_fragment_count": 0,
            "title": null,
            "title_redacted": false,
            "type_name": null,
            "type_name_redacted": false,
            "mixer_visible": null,
            "mixer_index": null,
            "mixer_zone": null,
            "child_count": child_count,
            "child_enumeration_error": null,
            "metadata_error_count": 0,
            "metadata_errors": [],
            "redacted_string_count": 1
        })
    }

    fn direct_access_test_reference(
        observation_epoch: u64,
        object_id: u64,
        parent_id: u64,
        depth: u64,
        child_index: u64,
        target_observation_index: u64,
        reference_kind: &str,
    ) -> Value {
        json!({
            "record_kind": "object_reference",
            "observation_epoch": observation_epoch,
            "observation_epoch_status": "snapshot_observed",
            "object_id": object_id,
            "parent_id": parent_id,
            "depth": depth,
            "child_index": child_index,
            "target_observation_index": target_observation_index,
            "reference_kind": reference_kind
        })
    }

    fn direct_access_test_assembly(
        items: Vec<Value>,
        observation_items: usize,
        reference_items: usize,
        cycle_count: u64,
        shared_reference_count: u64,
    ) -> RawSnapshotAssembly {
        let item_count = items.len();
        RawSnapshotAssembly {
            checkpoint_id: "INIT",
            source_instance_id: "SECRET_GRAPH_SOURCE".into(),
            kind: SnapshotKind::DirectAccess,
            is_snapshot: true,
            config_id: None,
            reason: "page_activate".into(),
            observation_epoch: Some(7),
            observation_epoch_status: Some("snapshot_observed".into()),
            bank_generation: None,
            observation_items,
            first_observation_seq: None,
            last_observation_seq: None,
            remaining_feedback_items: None,
            direct_access_base_object_id: Some(0),
            direct_access_reference_items: Some(reference_items),
            direct_access_cycle_count: Some(cycle_count),
            direct_access_shared_reference_count: Some(shared_reference_count),
            direct_access_error_count: Some(0),
            chunk_count: 1,
            next_chunk: 1,
            total_items: item_count,
            items,
            item_positions: vec![
                ProbePosition {
                    monotonic_timestamp_ms: 1,
                    record_index: 1,
                };
                item_count
            ],
            chunk_positions: vec![ProbePosition {
                monotonic_timestamp_ms: 1,
                record_index: 1,
            }],
            completed: true,
        }
    }

    fn validate_direct_access_test_assembly(assembly: &RawSnapshotAssembly) -> AuditResult<()> {
        validate_snapshot_privacy(assembly)?;
        validate_chunk_aggregate_metadata(assembly)?;
        validate_direct_access_graph(assembly)
    }

    struct TestArtifact {
        manifest: Value,
        records: Vec<Value>,
    }

    struct TestLog {
        records: Vec<Value>,
        monotonic_ms: u64,
        request_sequence: u64,
        source_sequences: HashMap<String, u64>,
        current_source: String,
        observation_epoch: u64,
        observation_sequence: u64,
        last_bank_title_feedback: HashMap<String, u64>,
        same_script_reactivation: Option<&'static str>,
    }

    impl TestLog {
        fn with_same_script_reactivation(same_script_reactivation: Option<&'static str>) -> Self {
            let mut log = Self {
                records: Vec::new(),
                monotonic_ms: 0,
                request_sequence: 0,
                source_sequences: HashMap::new(),
                current_source: "SECRET_INSTANCE_INIT".into(),
                observation_epoch: 0,
                observation_sequence: 0,
                last_bank_title_feedback: HashMap::new(),
                same_script_reactivation,
            };
            log.push_at(
                0,
                json!({
                    "record_type": "collector_started",
                    "session_id": "SECRET_COLLECTOR_SESSION",
                    "collector_version": "0.1.0",
                    "collector_binary_sha256": COLLECTOR_HASH,
                    "probe_transport_version": 1,
                    "midi_mode": "existing",
                    "virtual_to_cubase_port": null,
                    "virtual_from_cubase_port": null,
                    "configured_midi_input_port": "SECRET_INPUT_PORT",
                    "configured_midi_output_port": "SECRET_OUTPUT_PORT",
                    "max_json_bytes": 65536,
                    "max_sysex_bytes": 131080,
                    "max_outbound_json_bytes": 4096,
                    "queue_capacity": 1024,
                    "ingress_barrier_timeout_ms": 5000,
                    "discovery_window_ms": 1000,
                    "graceful_drain_timeout_ms": 5000,
                    "checkpoint_quiet_period_ms": 1000,
                    "resolved_midi_input_port": "/Users/private/SECRET_PORT",
                    "resolved_midi_output_port": "SECRET_OUTPUT_PORT"
                }),
            );
            log
        }

        fn push_at(&mut self, monotonic_ms: u64, mut value: Value) {
            assert!(monotonic_ms >= self.monotonic_ms);
            self.monotonic_ms = monotonic_ms;
            let object = value.as_object_mut().expect("test record is an object");
            object.insert("record_format_version".into(), json!(1));
            object.insert("run_id".into(), json!(TEST_RUN_ID));
            object.insert(
                "timestamp_unix_ms".into(),
                json!(TEST_UNIX_BASE + monotonic_ms),
            );
            object.insert("monotonic_timestamp_ms".into(), json!(monotonic_ms));
            self.records.push(value);
        }

        fn next_source_seq(&mut self) -> u64 {
            let sequence = self
                .source_sequences
                .entry(self.current_source.clone())
                .or_insert(0);
            *sequence += 1;
            *sequence
        }

        fn checkpoint_elapsed(&self, checkpoint_id: &str, monotonic_ms: u64) -> u64 {
            let begin = self
                .records
                .iter()
                .rev()
                .find(|record| {
                    record["record_type"] == "collector_checkpoint"
                        && record["phase"] == "begin"
                        && record["checkpoint_id"] == checkpoint_id
                })
                .and_then(|record| record["monotonic_timestamp_ms"].as_u64())
                .expect("probe test record has an active checkpoint");
            monotonic_ms.saturating_sub(begin)
        }

        fn probe_outer_fields(&self, checkpoint_id: &str, monotonic_ms: u64) -> Map<String, Value> {
            let elapsed = self.checkpoint_elapsed(checkpoint_id, monotonic_ms);
            Map::from_iter([
                ("midi_timestamp".into(), json!(monotonic_ms)),
                ("probe_transport_version".into(), json!(1)),
                ("checkpoint_elapsed_ms".into(), json!(elapsed)),
                ("checkpoint_window_ms".into(), json!(5000)),
                ("checkpoint_window_expired".into(), json!(elapsed >= 5000)),
            ])
        }

        fn event_at(&mut self, checkpoint_id: &str, monotonic_ms: u64, event: &str, data: Value) {
            let source_seq = self.next_source_seq();
            let outer = self.probe_outer_fields(checkpoint_id, monotonic_ms);
            let mut record = json!({
                "record_type": "probe_event",
                "integrity_ok_at_emit": true,
                "source_instance_id": self.current_source,
                "source_seq": source_seq,
                "checkpoint_id": checkpoint_id,
                "received_at_unix_ms": TEST_UNIX_BASE + monotonic_ms,
                "received_at_monotonic_timestamp_ms": monotonic_ms,
                "orphan": false,
                "processed_after_checkpoint_end": false,
                "checkpoint_quiet_period_violated": false,
                "message": {
                    "version": 1,
                    "type": "event",
                    "event": event,
                    "data": data
                }
            });
            record
                .as_object_mut()
                .expect("probe record is an object")
                .extend(outer);
            self.push_at(monotonic_ms, record);
        }

        fn capability_payload(&self, profile: Profile, direct_access_required: bool) -> Value {
            json!({
                "read_only": true,
                "integrity_failed": false,
                "host_version": profile.expected_host().1,
                "data_minimization": {
                    "source_redaction": true,
                    "fixture_revision": 2,
                    "unknown_titles": "redacted",
                    "unknown_host_ids": "omitted",
                    "unique_name_policy": "not_invoked",
                    "exception_text": "fixed_codes"
                },
                "observation_epoch": {
                    "supported": true,
                    "version": 1,
                    "current": self.observation_epoch,
                    "max": 2147483647,
                    "rollover_policy": "reload_required"
                },
                "mixer_bank": {
                    "supported": true,
                    "slot_count": 8,
                    "configs": ["MB_CORE_ALL", "MB_CORE_VISIBLE"],
                    "title": true,
                    "selected": true,
                    "mute": true,
                    "solo": true,
                    "unique_id": direct_access_required,
                    "explicit_main_filter": profile.requires_direct_access()
                },
                "direct_access": {
                    "supported": direct_access_required,
                    "active": direct_access_required,
                    "activation_error": null,
                    "get_object_unique_name_v1_2": direct_access_required,
                    "get_object_unique_id_string_v1_2": direct_access_required,
                    "get_object_title_v1_2": direct_access_required,
                    "get_object_type_name_v1_3": profile.requires_direct_access(),
                    "mixer_visibility_v1_2": direct_access_required,
                    "mixer_index_v1_2": direct_access_required,
                    "mixer_zone_v1_2": direct_access_required,
                    "reason": if direct_access_required { Value::Null } else { Value::String("make_direct_access_unavailable".into()) }
                },
                "limits": {
                    "output_json_bytes": 2048,
                    "chunk_items": 2,
                    "feedback_queue": 512,
                    "bank_snapshot_queue": 32,
                    "direct_access_snapshot_queue": 16,
                    "host_id_bytes": 4096,
                    "host_id_fragments": 16,
                    "wire_items_per_snapshot": 1024,
                    "direct_access_nodes": 256,
                    "direct_access_depth": 32,
                    "direct_access_children": 128
                }
            })
        }

        fn command_at(
            &mut self,
            checkpoint_id: &str,
            monotonic_ms: u64,
            expected: ExpectedCommand,
            profile: Profile,
            direct_access_required: bool,
        ) {
            self.request_sequence += 1;
            let request_id = format!("request-{}", self.request_sequence);
            let target = if expected.method != "probe.discover" {
                Value::String(self.current_source.clone())
            } else {
                Value::Null
            };
            let params = expected
                .config_id
                .map(|config_id| json!({"config_id": config_id}))
                .unwrap_or_else(|| json!({}));
            let mut command_record = json!({
                "record_type": "probe_command",
                "phase": "started",
                "request_id": request_id,
                "checkpoint_id": checkpoint_id,
                "request": {
                    "probe_transport_version": 1,
                    "target_instance_id": target,
                    "message": {
                        "version": 1,
                        "id": request_id,
                        "type": "request",
                        "method": expected.method,
                        "params": params
                    }
                }
            });
            if expected.method != "probe.discover" {
                command_record["evidence_emission"] = json!("after_midi_send_attempt");
            }
            self.push_at(monotonic_ms, command_record);
            let mut send_record = json!({
                "record_type": "probe_command_send_result",
                "request_id": request_id,
                "checkpoint_id": checkpoint_id,
                "sent": true,
                "sysex_bytes": 128,
                "send_completed_monotonic_timestamp_ms": monotonic_ms
            });
            if expected.method != "probe.discover" {
                send_record["evidence_emission"] = json!("after_midi_send_attempt");
            }
            self.push_at(monotonic_ms + 1, send_record);
            let source_seq = self.next_source_seq();
            let result = if expected.method == "probe.capabilities.get" {
                self.capability_payload(profile, direct_access_required)
            } else if expected.method == "probe.observation.cut" {
                self.observation_epoch += 1;
                json!({"observation_epoch": self.observation_epoch})
            } else if matches!(
                expected.method,
                "probe.bank.reset" | "probe.bank.next" | "probe.bank.prev"
            ) {
                json!({
                    "config_id": expected.config_id,
                    "action": expected.method.strip_prefix("probe.bank.").unwrap()
                })
            } else if expected.method == "probe.discover" {
                json!({
                    "instance_id": self.current_source,
                    "ready": true,
                    "read_only": true
                })
            } else if expected.method == "probe.bank.snapshot" {
                json!({"config_id": expected.config_id})
            } else if expected.method == "probe.direct_access.snapshot" {
                json!({})
            } else {
                unreachable!("unexpected test command method")
            };
            let response_at = monotonic_ms + 2;
            let outer = self.probe_outer_fields(checkpoint_id, response_at);
            let mut response_record = json!({
                "record_type": "probe_response",
                "integrity_ok_at_emit": true,
                "source_instance_id": self.current_source,
                "source_seq": source_seq,
                "checkpoint_id": checkpoint_id,
                "received_at_unix_ms": TEST_UNIX_BASE + monotonic_ms + 2,
                "received_at_monotonic_timestamp_ms": monotonic_ms + 2,
                "orphan": false,
                "processed_after_checkpoint_end": false,
                "checkpoint_quiet_period_violated": false,
                "message": {
                    "version": 1,
                    "id": request_id,
                    "type": "response",
                    "result": result
                }
            });
            response_record
                .as_object_mut()
                .expect("probe response is an object")
                .extend(outer);
            self.push_at(response_at, response_record);
            if expected.method == "probe.discover" {
                self.push_at(
                    monotonic_ms + 3,
                    json!({
                        "record_type": "collector_discovery_completed",
                        "request_id": request_id,
                        "checkpoint_id": checkpoint_id,
                        "responder_count": 1,
                        "outcome": "selected",
                        "window_closed": true,
                        "source_instance_ids": [self.current_source],
                        "observed_source_instance_ids": [self.current_source],
                        "selected_source_instance_id": self.current_source
                    }),
                );
            }
            if let Some((kind, reason)) = command_followup(expected.method) {
                let (event, stream, config_id) = match kind {
                    SnapshotKind::Bank => (
                        "probe.bank.chunk",
                        "mixer_bank_snapshot",
                        expected.config_id.map(Value::from).unwrap_or(Value::Null),
                    ),
                    SnapshotKind::DirectAccess => (
                        "probe.direct_access.chunk",
                        "direct_access_snapshot",
                        Value::Null,
                    ),
                };
                let items = match kind {
                    SnapshotKind::Bank => (0..8)
                        .map(|slot_index| {
                            let raw_id = (slot_index == 0)
                                .then(|| "ghp_SECRET_HOST_ID_0".to_owned());
                            let raw_id_len = raw_id.as_ref().map(String::len);
                            let title = match slot_index {
                                0 => Value::String("CMCP_E8_01".into()),
                                _ => Value::Null,
                            };
                            let title_redacted = slot_index == 1;
                            let host_id_redacted = slot_index == 1;
                            let host_observed = slot_index == 0;
                            let title_sequence = if slot_index == 0 {
                                expected
                                    .config_id
                                    .and_then(|config| self.last_bank_title_feedback.get(config))
                                    .copied()
                                    .or(Some(100))
                            } else if slot_index == 1 {
                                Some(101)
                            } else {
                                None
                            };
                            json!({
                                "record_kind": "observation",
                                "config_id": expected.config_id,
                                "bank_generation": 1,
                                "slot_index": slot_index,
                                "title": title,
                                "title_redacted": title_redacted,
                                "selected": slot_index == 0,
                                "mute": false,
                                "solo": false,
                                "host_id_raw": raw_id,
                                "host_id_redacted": host_id_redacted,
                                "host_id_observed_with_title_callback": host_observed,
                                "host_id_observation_status": match slot_index {
                                    0 => "observed_with_title_callback",
                                    1 => "title_not_authorized",
                                    _ => "not_observed"
                                },
                                "redacted_string_count": u64::from(title_redacted) + u64::from(host_id_redacted),
                                "field_observation_generation": {
                                    "title": if slot_index <= 1 { 1 } else { -1 },
                                    "selected": 1,
                                    "mute": 1,
                                    "solo": 1,
                                    "host_id_raw": if host_observed { 1 } else { -1 }
                                },
                                "field_last_observation_seq": {
                                    "title": title_sequence,
                                    "selected": 200 + slot_index,
                                    "mute": 300 + slot_index,
                                    "solo": 400 + slot_index,
                                    "host_id_raw": if host_observed { title_sequence } else { None }
                                },
                                "field_last_observation_epoch": {
                                    "title": if slot_index <= 1 { Some(self.observation_epoch) } else { None },
                                    "selected": self.observation_epoch,
                                    "mute": self.observation_epoch,
                                    "solo": self.observation_epoch,
                                    "host_id_raw": if host_observed { Some(self.observation_epoch) } else { None }
                                },
                                "host_id_byte_length": raw_id_len,
                                "host_id_ref": null,
                                "host_id_fragment_count": 0
                            })
                        })
                        .collect::<Vec<_>>(),
                    SnapshotKind::DirectAccess => vec![json!({
                        "record_kind": "observation",
                        "observation_epoch": self.observation_epoch,
                        "observation_epoch_status": "snapshot_observed",
                        "object_id": 42,
                        "parent_id": null,
                        "depth": 0,
                        "child_index": null,
                        "unique_name": null,
                        "unique_name_redacted": direct_access_required,
                        "unique_name_status": if direct_access_required { "not_invoked_by_policy" } else { "not_available" },
                        "host_id_raw": "ghp_SECRET_DIRECT_HOST_ID",
                        "host_id_redacted": false,
                        "host_id_byte_length": "ghp_SECRET_DIRECT_HOST_ID".len(),
                        "host_id_ref": null,
                        "host_id_fragment_count": 0,
                        "title": "CMCP_E8_01",
                        "title_redacted": false,
                        "type_name": if profile.requires_direct_access() { Value::String("AudioChannel".into()) } else { Value::Null },
                        "type_name_redacted": false,
                        "mixer_visible": true,
                        "mixer_index": 0,
                        "mixer_zone": 0,
                        "child_count": 0,
                        "child_enumeration_error": null,
                        "metadata_error_count": 0,
                        "metadata_errors": [],
                        "redacted_string_count": usize::from(direct_access_required)
                    })],
                };
                let snapshot_id = format!("SECRET_SNAPSHOT_{}", self.request_sequence);
                let chunks = items.chunks(2).collect::<Vec<_>>();
                let chunk_count = chunks.len();
                for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                    let mut data = json!({
                        "snapshot_id": snapshot_id,
                        "stream": stream,
                        "reason": reason,
                        "chunk_index": chunk_index,
                        "chunk_count": chunk_count,
                        "snapshot_complete": chunk_index + 1 == chunk_count,
                        "truncated": false,
                        "overflow_safe": true,
                        "total_items": items.len(),
                        "observation_items": items.len(),
                        "items": chunk
                    });
                    if kind == SnapshotKind::DirectAccess {
                        data["base_object_id"] = json!(42);
                        data["observation_epoch"] = json!(self.observation_epoch);
                        data["observation_epoch_status"] = json!("snapshot_observed");
                        data["reference_items"] = json!(0);
                        data["cycle_count"] = json!(0);
                        data["shared_reference_count"] = json!(0);
                        data["error_count"] = json!(0);
                        data["truncation_reasons"] = json!([]);
                    } else {
                        data["follow_visibility"] =
                            json!(expected.config_id == Some("MB_CORE_VISIBLE"));
                        data["requested_bank_generation"] = json!(1);
                        data["bank_generation"] = json!(1);
                        data["superseded"] = json!(false);
                    }
                    if !config_id.is_null() {
                        data.as_object_mut()
                            .expect("snapshot data is an object")
                            .insert("config_id".into(), config_id.clone());
                    }
                    self.event_at(
                        checkpoint_id,
                        monotonic_ms + 3 + chunk_index as u64,
                        event,
                        data,
                    );
                }
            }
        }

        fn bank_feedback_at(&mut self, checkpoint_id: &str, monotonic_ms: u64, config_id: &str) {
            self.observation_sequence += 1;
            let sequence = self.observation_sequence;
            self.last_bank_title_feedback
                .insert(config_id.to_owned(), sequence);
            let raw_id = "ghp_SECRET_HOST_ID_0";
            self.event_at(
                checkpoint_id,
                monotonic_ms,
                "probe.bank.chunk",
                json!({
                    "snapshot_id": format!("SECRET_BANK_FEEDBACK_{sequence}"),
                    "stream": "mixer_bank_feedback",
                    "reason": "feedback",
                    "chunk_index": 0,
                    "chunk_count": 1,
                    "snapshot_complete": true,
                    "truncated": false,
                    "overflow_safe": true,
                    "total_items": 1,
                    "observation_items": 1,
                    "first_observation_seq": sequence,
                    "last_observation_seq": sequence,
                    "remaining_items": 0,
                    "items": [{
                        "record_kind": "observation",
                        "config_id": config_id,
                        "bank_generation": 1,
                        "slot_index": 0,
                        "title": "CMCP_E8_01",
                        "title_redacted": false,
                        "selected": true,
                        "mute": false,
                        "solo": false,
                        "host_id_raw": raw_id,
                        "host_id_redacted": false,
                        "host_id_observed_with_title_callback": true,
                        "host_id_observation_status": "observed_with_title_callback",
                        "redacted_string_count": 0,
                        "field_observation_generation": {
                            "title": 1,
                            "selected": 1,
                            "mute": 1,
                            "solo": 1,
                            "host_id_raw": 1
                        },
                        "field_last_observation_seq": {
                            "title": sequence,
                            "selected": null,
                            "mute": null,
                            "solo": null,
                            "host_id_raw": sequence
                        },
                        "field_last_observation_epoch": {
                            "title": self.observation_epoch,
                            "selected": null,
                            "mute": null,
                            "solo": null,
                            "host_id_raw": self.observation_epoch
                        },
                        "observation_seq": sequence,
                        "observation_epoch": self.observation_epoch,
                        "observation_epoch_status": "callback_observed",
                        "changed_field": "title",
                        "changed_value": "CMCP_E8_01",
                        "changed_value_redacted": false,
                        "callback_source": "mixer_bank_channel",
                        "host_id_byte_length": raw_id.len(),
                        "host_id_ref": null,
                        "host_id_fragment_count": 0
                    }]
                }),
            );
        }

        fn direct_feedback_at(&mut self, checkpoint_id: &str, monotonic_ms: u64) {
            self.observation_sequence += 1;
            let sequence = self.observation_sequence;
            self.event_at(
                checkpoint_id,
                monotonic_ms,
                "probe.direct_access.chunk",
                json!({
                    "snapshot_id": format!("SECRET_DIRECT_FEEDBACK_{sequence}"),
                    "stream": "direct_access_feedback",
                    "reason": "feedback",
                    "chunk_index": 0,
                    "chunk_count": 1,
                    "snapshot_complete": true,
                    "truncated": false,
                    "overflow_safe": true,
                    "total_items": 1,
                    "observation_items": 1,
                    "first_observation_seq": sequence,
                    "last_observation_seq": sequence,
                    "remaining_items": 0,
                    "items": [{
                        "observation_seq": sequence,
                        "observation_epoch": self.observation_epoch,
                        "observation_epoch_status": "callback_observed",
                        "change": "object_change",
                        "object_id": 42,
                        "parameter_tag": null
                    }]
                }),
            );
        }

        fn initial_snapshot_at(
            &mut self,
            checkpoint_id: &str,
            monotonic_ms: u64,
            kind: SnapshotKind,
            config_id: Option<&str>,
        ) {
            self.request_sequence += 1;
            let (event, stream) = match kind {
                SnapshotKind::Bank => ("probe.bank.chunk", "mixer_bank_snapshot"),
                SnapshotKind::DirectAccess => {
                    ("probe.direct_access.chunk", "direct_access_snapshot")
                }
            };
            let mut data = json!({
                "snapshot_id": format!("SECRET_INITIAL_SNAPSHOT_{}", self.request_sequence),
                "stream": stream,
                "reason": "page_activate",
                "chunk_index": 0,
                "chunk_count": 1,
                "snapshot_complete": true,
                "truncated": false,
                "overflow_safe": true,
                "total_items": 0,
                "observation_items": 0,
                "items": []
            });
            if kind == SnapshotKind::DirectAccess {
                data["total_items"] = json!(1);
                data["observation_items"] = json!(1);
                data["items"] = json!([direct_access_test_observation(
                    self.observation_epoch,
                    42,
                    None,
                    0,
                    None,
                    0,
                )]);
                data["base_object_id"] = json!(42);
                data["observation_epoch"] = json!(self.observation_epoch);
                data["observation_epoch_status"] = json!("snapshot_observed");
                data["reference_items"] = json!(0);
                data["cycle_count"] = json!(0);
                data["shared_reference_count"] = json!(0);
                data["error_count"] = json!(0);
                data["truncation_reasons"] = json!([]);
            } else {
                data["follow_visibility"] = json!(config_id == Some("MB_CORE_VISIBLE"));
                data["requested_bank_generation"] = json!(1);
                data["bank_generation"] = json!(1);
                data["superseded"] = json!(false);
            }
            if let Some(config_id) = config_id {
                data["config_id"] = json!(config_id);
            }
            self.event_at(checkpoint_id, monotonic_ms, event, data);
        }

        fn capability_event_at(
            &mut self,
            checkpoint_id: &str,
            monotonic_ms: u64,
            profile: Profile,
            direct_access_required: bool,
        ) {
            self.event_at(
                checkpoint_id,
                monotonic_ms,
                "probe.capabilities",
                self.capability_payload(profile, direct_access_required),
            );
        }

        fn same_script_reactivation_at(
            &mut self,
            checkpoint_id: &str,
            begin: u64,
            profile: Profile,
            direct_access_required: bool,
        ) {
            self.event_at(
                checkpoint_id,
                begin + 210,
                "probe.ready",
                json!({
                    "probe_session_id": self.current_source,
                    "ready": false,
                    "initial_snapshots_complete": true,
                    "read_only": true,
                    "protocol_version": 1
                }),
            );
            self.event_at(
                checkpoint_id,
                begin + 215,
                "probe.mapping_active",
                json!({
                    "probe_session_id": self.current_source,
                    "mapping_active": true,
                    "read_only": true,
                    "protocol_version": 1
                }),
            );
            self.capability_event_at(checkpoint_id, begin + 220, profile, direct_access_required);
            self.initial_snapshot_at(
                checkpoint_id,
                begin + 225,
                SnapshotKind::Bank,
                Some("MB_CORE_ALL"),
            );
            self.initial_snapshot_at(
                checkpoint_id,
                begin + 230,
                SnapshotKind::Bank,
                Some("MB_CORE_VISIBLE"),
            );
            if direct_access_required {
                self.initial_snapshot_at(
                    checkpoint_id,
                    begin + 235,
                    SnapshotKind::DirectAccess,
                    None,
                );
            }
            self.event_at(
                checkpoint_id,
                begin + 240,
                "probe.ready",
                json!({
                    "probe_session_id": self.current_source,
                    "ready": true,
                    "initial_snapshots_complete": true,
                    "read_only": true,
                    "protocol_version": 1
                }),
            );
        }

        fn checkpoint(
            &mut self,
            checkpoint_id: &'static str,
            profile: Profile,
            direct_access_required: bool,
        ) {
            let begin = self.monotonic_ms + 100;
            self.push_at(
                begin,
                json!({
                    "record_type": "collector_checkpoint",
                    "phase": "begin",
                    "checkpoint_id": checkpoint_id,
                    "window_ms": 5000
                }),
            );
            let activation_checkpoint = matches!(checkpoint_id, "INIT" | "R1" | "R2");
            let same_script_reactivation = self.same_script_reactivation == Some(checkpoint_id);
            let expected = expected_commands_with_activation(
                checkpoint_id,
                direct_access_required,
                same_script_reactivation,
            );
            let cut_required = requires_observation_cut(checkpoint_id);
            let command_start = if cut_required {
                let cut = expected
                    .first()
                    .copied()
                    .expect("cut checkpoint has a command");
                assert_eq!(cut.method, "probe.observation.cut");
                self.command_at(
                    checkpoint_id,
                    begin + 100,
                    cut,
                    profile,
                    direct_access_required,
                );
                self.push_at(
                    begin + 102,
                    json!({
                        "record_type": "collector_action",
                        "phase": "marked",
                        "checkpoint_id": checkpoint_id,
                        "boundary_source": "probe.observation.cut_response",
                        "request_id": format!("request-{}", self.request_sequence),
                        "observation_epoch": self.observation_epoch
                    }),
                );
                if checkpoint_id == "C1" {
                    self.bank_feedback_at(checkpoint_id, begin + 115, "MB_CORE_ALL");
                    self.bank_feedback_at(checkpoint_id, begin + 117, "MB_CORE_VISIBLE");
                    if direct_access_required {
                        self.direct_feedback_at(checkpoint_id, begin + 119);
                    }
                }
                if same_script_reactivation {
                    self.same_script_reactivation_at(
                        checkpoint_id,
                        begin,
                        profile,
                        direct_access_required,
                    );
                }
                1
            } else {
                let action_offset = if activation_checkpoint { 10 } else { 100 };
                self.push_at(
                    begin + action_offset,
                    json!({
                        "record_type": "collector_action",
                        "phase": "marked",
                        "checkpoint_id": checkpoint_id
                    }),
                );
                0
            };
            if matches!(checkpoint_id, "R1" | "R2") {
                self.event_at(
                    checkpoint_id,
                    begin + 15,
                    "probe.ready",
                    json!({
                        "probe_session_id": self.current_source,
                        "ready": false,
                        "initial_snapshots_complete": true,
                        "read_only": true,
                        "protocol_version": 1
                    }),
                );
                self.current_source = format!("SECRET_INSTANCE_{checkpoint_id}");
                self.observation_epoch = 0;
            }
            if activation_checkpoint {
                self.event_at(
                    checkpoint_id,
                    begin + 20,
                    "probe.loaded",
                    json!({
                        "probe_session_id": self.current_source,
                        "mapping_active": true,
                        "read_only": true,
                        "protocol_version": 1
                    }),
                );
                self.event_at(
                    checkpoint_id,
                    begin + 25,
                    "probe.mapping_active",
                    json!({
                        "probe_session_id": self.current_source,
                        "mapping_active": true,
                        "read_only": true,
                        "protocol_version": 1
                    }),
                );
                self.capability_event_at(
                    checkpoint_id,
                    begin + 30,
                    profile,
                    direct_access_required,
                );
                if checkpoint_id == "INIT" {
                    self.bank_feedback_at(checkpoint_id, begin + 31, "MB_CORE_ALL");
                    self.bank_feedback_at(checkpoint_id, begin + 32, "MB_CORE_VISIBLE");
                    if direct_access_required {
                        self.direct_feedback_at(checkpoint_id, begin + 33);
                    }
                }
                self.initial_snapshot_at(
                    checkpoint_id,
                    begin + 35,
                    SnapshotKind::Bank,
                    Some("MB_CORE_ALL"),
                );
                self.initial_snapshot_at(
                    checkpoint_id,
                    begin + 40,
                    SnapshotKind::Bank,
                    Some("MB_CORE_VISIBLE"),
                );
                if direct_access_required {
                    self.initial_snapshot_at(
                        checkpoint_id,
                        begin + 45,
                        SnapshotKind::DirectAccess,
                        None,
                    );
                }
                self.event_at(
                    checkpoint_id,
                    begin + 50,
                    "probe.ready",
                    json!({
                        "probe_session_id": self.current_source,
                        "ready": true,
                        "initial_snapshots_complete": true,
                        "read_only": true,
                        "protocol_version": 1
                    }),
                );
            }

            let mut early_offset = if cut_required && same_script_reactivation {
                300
            } else {
                200
            };
            let mut final_offset = 5_300;
            for command in expected.into_iter().skip(command_start) {
                let offset = if matches!(
                    command.method,
                    "probe.bank.snapshot" | "probe.direct_access.snapshot"
                ) {
                    let offset = final_offset;
                    final_offset += 20;
                    offset
                } else {
                    let offset = early_offset;
                    early_offset += 20;
                    offset
                };
                self.command_at(
                    checkpoint_id,
                    begin + offset,
                    command,
                    profile,
                    direct_access_required,
                );
                if same_script_reactivation
                    && bank_operation(checkpoint_id)
                        .is_some_and(|(_, operation)| command.method == operation)
                {
                    self.same_script_reactivation_at(
                        checkpoint_id,
                        begin,
                        profile,
                        direct_access_required,
                    );
                    early_offset = 300;
                }
            }
            let end = begin + final_offset + 1_100;
            self.push_at(
                end,
                json!({
                    "record_type": "collector_checkpoint",
                    "phase": "end",
                    "checkpoint_id": checkpoint_id,
                    "window_ms": 5000,
                    "observed_duration_ms": end - begin,
                    "window_satisfied": true,
                    "quiet_period_required_ms": 1000,
                    "quiet_period_observed_ms": 1100,
                    "quiet_period_satisfied": true,
                    "messages_processed_before_end_marker": self.records.iter().filter(|record| {
                        matches!(record["record_type"].as_str(), Some("probe_event" | "probe_response" | "probe_error"))
                            && record["checkpoint_id"] == checkpoint_id
                    }).count(),
                    "late_received_frames_may_be_classified_by_receive_timestamp": true
                }),
            );
        }

        fn finish(mut self) -> Vec<Value> {
            let command_count = self
                .records
                .iter()
                .filter(|record| record["record_type"] == "probe_command")
                .count() as u64;
            let message_count = self
                .records
                .iter()
                .filter(|record| {
                    matches!(
                        record["record_type"].as_str(),
                        Some("probe_event" | "probe_response" | "probe_error")
                    )
                })
                .count() as u64;
            let event_count = self
                .records
                .iter()
                .filter(|record| record["record_type"] == "probe_event")
                .count() as u64;
            let response_count = self
                .records
                .iter()
                .filter(|record| record["record_type"] == "probe_response")
                .count() as u64;
            let chunk_stream_count = self
                .records
                .iter()
                .filter(|record| {
                    record["record_type"] == "probe_event"
                        && matches!(
                            record["message"]["event"].as_str(),
                            Some("probe.bank.chunk" | "probe.direct_access.chunk")
                        )
                        && record["message"]["data"]["snapshot_complete"] == true
                })
                .count() as u64;
            let feedback_count = self
                .records
                .iter()
                .filter(|record| {
                    record["record_type"] == "probe_event"
                        && matches!(
                            record["message"]["data"]["stream"].as_str(),
                            Some("mixer_bank_feedback" | "direct_access_feedback")
                        )
                        && record["message"]["data"]["snapshot_complete"] == true
                })
                .count() as u64;
            let current_source = self.current_source.clone();
            let mut source_summaries = self
                .source_sequences
                .iter()
                .map(|(source_instance_id, last_source_seq)| {
                    json!({
                        "source_instance_id": source_instance_id,
                        "last_source_seq": last_source_seq
                    })
                })
                .collect::<Vec<_>>();
            source_summaries.sort_by(|left, right| {
                left["source_instance_id"]
                    .as_str()
                    .cmp(&right["source_instance_id"].as_str())
            });
            let drain_started = self.monotonic_ms + 100;
            self.push_at(
                drain_started,
                json!({
                    "record_type": "collector_drain_started",
                    "timeout_ms": 5000,
                    "deadline_monotonic_timestamp_ms": drain_started + 5000
                }),
            );
            self.push_at(
                drain_started + 5000,
                json!({
                    "record_type": "collector_drain_completed",
                    "completed": true,
                    "timed_out": false,
                    "duration_ms": 5000
                }),
            );
            self.push_at(
                self.monotonic_ms + 1,
                json!({
                    "record_type": "collector_summary",
                    "exit_reason": "stdin_eof",
                    "session_id": "SECRET_COLLECTOR_SESSION",
                    "integrity_ok": true,
                    "exit_ok": true,
                    "commands": {
                        "received": command_count + (REQUIRED_CHECKPOINTS.len() * 2 + REQUIRED_CHECKPOINTS.iter().filter(|id| !requires_observation_cut(id)).count()) as u64,
                        "sent": command_count,
                        "local": (REQUIRED_CHECKPOINTS.len() * 2 + REQUIRED_CHECKPOINTS.iter().filter(|id| !requires_observation_cut(id)).count()) as u64,
                        "deferred": 0,
                        "rejected": 0
                    },
                    "graceful_drain": {"completed": true, "timed_out": false, "duration_ms": 5000},
                    "orphan_messages": 0,
                    "protocol_tracking": {
                        "completed_requests": command_count,
                        "completed_chunk_streams": chunk_stream_count,
                        "completed_snapshot_streams": chunk_stream_count - feedback_count,
                        "completed_feedback_streams": feedback_count,
                        "completed_checkpoints": REQUIRED_CHECKPOINTS.len(),
                        "checkpoint_messages": message_count,
                        "checkpoint_messages_processed_after_end": 0,
                        "orphan_messages": 0,
                        "pending_requests": 0,
                        "expected_followups": 0,
                        "open_snapshots": 0,
                        "selected_source_instance_id": current_source,
                        "active_source_instance_ids": [current_source]
                    },
                    "incoming": {
                        "frames": message_count,
                        "messages": message_count,
                        "events": event_count,
                        "responses": response_count,
                        "errors": 0,
                        "diagnostics": 0,
                        "parse_errors": 0,
                        "oversize_frames": 0,
                        "source_overflows": 0,
                        "queue_drops": 0,
                        "sequence_gaps": 0,
                        "sequence_duplicates_or_reorders": 0,
                        "sources": source_summaries
                    }
                }),
            );
            self.records
        }
    }

    fn valid_artifact(profile: Profile) -> TestArtifact {
        valid_artifact_with_direct_access(profile, profile.requires_direct_access())
    }

    fn valid_artifact_with_direct_access(
        profile: Profile,
        direct_access_required: bool,
    ) -> TestArtifact {
        valid_artifact_with_options(profile, direct_access_required, None)
    }

    fn valid_artifact_with_options(
        profile: Profile,
        direct_access_required: bool,
        same_script_reactivation: Option<&'static str>,
    ) -> TestArtifact {
        let mut log = TestLog::with_same_script_reactivation(same_script_reactivation);
        for checkpoint_id in REQUIRED_CHECKPOINTS {
            log.checkpoint(checkpoint_id, profile, direct_access_required);
        }
        let (product, version, api_version) = profile.expected_host();
        let manifest = json!({
            "audit_manifest_version": 1,
            "fixture_revision": 2,
            "profile": match profile { Profile::C13MixerBank => "c13_mixer_bank", Profile::C15Combined => "c15_combined" },
            "run_id": TEST_RUN_ID,
            "run_started_at": "2027-01-15T08:00:00.000+00:00",
            "environment": {
                "host": {"product": product, "version": version, "api_version": api_version},
                "os": {"name": "macOS", "version": "26.5.1", "build": "25F80", "architecture": "arm64"},
                "repository_commit": COMMIT_HASH,
                "probe_source_sha256": PROBE_HASH,
                "installer_embedded_sha256": PROBE_HASH,
                "collector_binary_sha256": COLLECTOR_HASH,
                "deployed_probe_sha256": PROBE_HASH
            },
            "mixconsole": {
                "surface": "separate",
                "visibility_sync_initial": "off",
                "visibility_sync_during_baseline": "on",
                "visibility_sync_restored": true
            },
            "filters": {
                "bank_width": 8,
                "core_all_follow_visibility": false,
                "core_visible_follow_visibility": true,
                "included_channel_types": ["audio", "instrument", "midi", "group", "fx"],
                "excluded_channel_types": ["sampler", "vca", "input", "output"],
                "left_zone": "excluded",
                "right_zone": "excluded",
                "main_filter": if profile.requires_direct_access() { "explicit" } else { "implicit" }
            },
            "fixture_acceptance": {
                "alternate_plugins": {
                    "instrument": {"status": "none"},
                    "effect": {"status": "none"}
                },
                "p05_title": {
                    "policy": "nfc_or_nfd_exact",
                    "accepted_title": P05_TITLE_NFC,
                    "unicode_scalar_count": P05_TITLE_NFC.chars().count(),
                    "utf8_byte_length": P05_TITLE_NFC.len(),
                    "setup_variance": false
                },
                "p09_title": {
                    "policy": "fixed_name_prefix",
                    "accepted_title": P09_TITLE_FULL,
                    "unicode_scalar_count": P09_TITLE_FULL.chars().count(),
                    "utf8_byte_length": P09_TITLE_FULL.len(),
                    "setup_variance": false
                }
            },
            "callback_window_ms": 5000,
            "reconnect_deadline_ms": 30000,
            "optional_o1": {"status": "skipped", "reason": "not_separately_authorized"},
            "annotations": REQUIRED_CHECKPOINTS.into_iter().map(|checkpoint_id| json!({
                "checkpoint_id": checkpoint_id,
                "result": "observed",
                "ui_ground_truth_confirmed": true,
                "action_confirmed": true
            })).collect::<Vec<_>>()
        });
        TestArtifact {
            manifest,
            records: log.finish(),
        }
    }

    fn encode_json(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).expect("test JSON encodes")
    }

    fn encode_jsonl(records: &[Value]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            serde_json::to_writer(&mut bytes, record).expect("test JSONL record encodes");
            bytes.push(b'\n');
        }
        bytes
    }

    fn audit_artifact(artifact: &TestArtifact) -> AuditResult<AuditReport> {
        audit_bytes(
            &encode_json(&artifact.manifest),
            &encode_jsonl(&artifact.records),
        )
    }

    fn remove_records(artifact: &mut TestArtifact, predicate: impl Fn(&Value) -> bool) {
        artifact.records.retain(|record| !predicate(record));
    }

    fn direct_access_command_snapshot_data_mut<'a>(
        artifact: &'a mut TestArtifact,
        checkpoint_id: &str,
    ) -> &'a mut Value {
        &mut artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["message"]["event"] == "probe.direct_access.chunk"
                    && record["message"]["data"]["stream"] == "direct_access_snapshot"
                    && record["message"]["data"]["reason"] == "command_snapshot"
            })
            .expect("checkpoint has a DirectAccess command snapshot")["message"]["data"]
    }

    fn move_response_before_command(
        artifact: &mut TestArtifact,
        checkpoint_id: &str,
        method: &str,
    ) {
        let command_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["request"]["message"]["method"] == method
            })
            .unwrap();
        let request_id = artifact.records[command_index]["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let response_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_response" && record["message"]["id"] == request_id
            })
            .unwrap();
        let response = artifact.records.remove(response_index);
        let response_monotonic = response["monotonic_timestamp_ms"].clone();
        let response_unix = response["timestamp_unix_ms"].clone();
        artifact.records[command_index]["monotonic_timestamp_ms"] = response_monotonic.clone();
        artifact.records[command_index]["timestamp_unix_ms"] = response_unix.clone();
        artifact.records[command_index + 1]["monotonic_timestamp_ms"] = response_monotonic;
        artifact.records[command_index + 1]["timestamp_unix_ms"] = response_unix;
        artifact.records.insert(command_index, response);
    }

    fn insert_unsolicited_after_first_final(artifact: &mut TestArtifact, checkpoint_id: &str) {
        let first_final_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["message"]["event"] == "probe.direct_access.chunk"
                    && record["message"]["data"]["reason"] == "command_snapshot"
            })
            .unwrap();
        let source = artifact.records[first_final_index]["source_instance_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let prior_sequence = artifact.records[first_final_index]["source_seq"]
            .as_u64()
            .unwrap();
        let prior_monotonic =
            artifact.records[first_final_index]["received_at_monotonic_timestamp_ms"]
                .as_u64()
                .unwrap();
        let observation_epoch =
            artifact.records[first_final_index]["message"]["data"]["observation_epoch"]
                .as_u64()
                .unwrap();
        for record in artifact.records.iter_mut().skip(first_final_index + 1) {
            if record["source_instance_id"].as_str() == Some(&source)
                && matches!(
                    record["record_type"].as_str(),
                    Some("probe_event" | "probe_response" | "probe_error")
                )
            {
                let sequence = record["source_seq"].as_u64().unwrap();
                record["source_seq"] = json!(sequence + 1);
            }
        }
        let monotonic = prior_monotonic + 5;
        let checkpoint_begin = artifact.records[..=first_final_index]
            .iter()
            .rev()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "begin"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        artifact.records.insert(
            first_final_index + 1,
            json!({
                "record_format_version": 1,
                "run_id": TEST_RUN_ID,
                "record_type": "probe_event",
                "timestamp_unix_ms": TEST_UNIX_BASE + monotonic,
                "monotonic_timestamp_ms": monotonic,
                "midi_timestamp": monotonic,
                "integrity_ok_at_emit": true,
                "probe_transport_version": 1,
                "source_instance_id": source,
                "source_seq": prior_sequence + 1,
                "checkpoint_id": checkpoint_id,
                "received_at_unix_ms": TEST_UNIX_BASE + monotonic,
                "received_at_monotonic_timestamp_ms": monotonic,
                "orphan": false,
                "checkpoint_elapsed_ms": monotonic - checkpoint_begin,
                "checkpoint_window_ms": 5000,
                "checkpoint_window_expired": monotonic - checkpoint_begin >= 5000,
                "processed_after_checkpoint_end": false,
                "checkpoint_quiet_period_violated": false,
                "message": {
                    "version": 1,
                    "type": "event",
                    "event": "probe.direct_access.chunk",
                    "data": {
                        "snapshot_id": "SECRET_LATE_OBJECT_CHANGE",
                        "stream": "direct_access_snapshot",
                        "reason": "object_change",
                        "observation_epoch": observation_epoch,
                        "observation_epoch_status": "snapshot_observed",
                        "chunk_index": 0,
                        "chunk_count": 1,
                        "snapshot_complete": true,
                        "truncated": false,
                        "overflow_safe": true,
                        "total_items": 1,
                        "observation_items": 1,
                        "base_object_id": 42,
                        "reference_items": 0,
                        "cycle_count": 0,
                        "shared_reference_count": 0,
                        "error_count": 0,
                        "truncation_reasons": [],
                        "items": [direct_access_test_observation(
                            observation_epoch, 42, None, 0, None, 0
                        )]
                    }
                }
            }),
        );
        let summary = artifact.records.last_mut().unwrap();
        for field in ["completed_chunk_streams", "completed_snapshot_streams"] {
            let count = summary["protocol_tracking"][field].as_u64().unwrap();
            summary["protocol_tracking"][field] = json!(count + 1);
        }
        let checkpoint_messages = summary["protocol_tracking"]["checkpoint_messages"]
            .as_u64()
            .unwrap();
        summary["protocol_tracking"]["checkpoint_messages"] = json!(checkpoint_messages + 1);
        let messages = summary["incoming"]["messages"].as_u64().unwrap();
        let events = summary["incoming"]["events"].as_u64().unwrap();
        summary["incoming"]["messages"] = json!(messages + 1);
        summary["incoming"]["frames"] = json!(messages + 1);
        summary["incoming"]["events"] = json!(events + 1);
        let source_summary = summary["incoming"]["sources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["source_instance_id"] == source)
            .unwrap();
        let last = source_summary["last_source_seq"].as_u64().unwrap();
        source_summary["last_source_seq"] = json!(last + 1);
        let checkpoint_end = artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "end"
            })
            .unwrap();
        checkpoint_end["messages_processed_before_end_marker"] = json!(
            checkpoint_end["messages_processed_before_end_marker"]
                .as_u64()
                .unwrap()
                + 1
        );
    }

    fn insert_pre_cut_direct_access_snapshot(artifact: &mut TestArtifact, checkpoint_id: &str) {
        let cut_command_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["request"]["message"]["method"] == "probe.observation.cut"
            })
            .unwrap();
        let source = artifact.records[cut_command_index]["request"]["target_instance_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let prior_sequence = artifact.records[..cut_command_index]
            .iter()
            .rev()
            .find(|record| {
                record["source_instance_id"].as_str() == Some(&source)
                    && matches!(
                        record["record_type"].as_str(),
                        Some("probe_event" | "probe_response" | "probe_error")
                    )
            })
            .unwrap()["source_seq"]
            .as_u64()
            .unwrap();
        for record in artifact.records.iter_mut().skip(cut_command_index) {
            if record["source_instance_id"].as_str() == Some(&source)
                && matches!(
                    record["record_type"].as_str(),
                    Some("probe_event" | "probe_response" | "probe_error")
                )
            {
                let sequence = record["source_seq"].as_u64().unwrap();
                record["source_seq"] = json!(sequence + 1);
            }
        }
        let cut_command_at = artifact.records[cut_command_index]["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let monotonic = cut_command_at - 50;
        let checkpoint_begin = artifact.records[..cut_command_index]
            .iter()
            .rev()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "begin"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        artifact.records.insert(
            cut_command_index,
            json!({
                "record_format_version": 1,
                "run_id": TEST_RUN_ID,
                "record_type": "probe_event",
                "timestamp_unix_ms": TEST_UNIX_BASE + monotonic,
                "monotonic_timestamp_ms": monotonic,
                "midi_timestamp": monotonic,
                "integrity_ok_at_emit": true,
                "probe_transport_version": 1,
                "source_instance_id": source,
                "source_seq": prior_sequence + 1,
                "checkpoint_id": checkpoint_id,
                "received_at_unix_ms": TEST_UNIX_BASE + monotonic,
                "received_at_monotonic_timestamp_ms": monotonic,
                "orphan": false,
                "checkpoint_elapsed_ms": monotonic - checkpoint_begin,
                "checkpoint_window_ms": 5000,
                "checkpoint_window_expired": false,
                "processed_after_checkpoint_end": false,
                "checkpoint_quiet_period_violated": false,
                "message": {
                    "version": 1,
                    "type": "event",
                    "event": "probe.direct_access.chunk",
                    "data": {
                        "snapshot_id": "SECRET_PRE_CUT_OBJECT_CHANGE",
                        "stream": "direct_access_snapshot",
                        "reason": "object_change",
                        "observation_epoch": 0,
                        "observation_epoch_status": "snapshot_observed",
                        "chunk_index": 0,
                        "chunk_count": 1,
                        "snapshot_complete": true,
                        "truncated": false,
                        "overflow_safe": true,
                        "total_items": 1,
                        "observation_items": 1,
                        "base_object_id": 42,
                        "reference_items": 0,
                        "cycle_count": 0,
                        "shared_reference_count": 0,
                        "error_count": 0,
                        "truncation_reasons": [],
                        "items": [direct_access_test_observation(
                            0, 42, None, 0, None, 0
                        )]
                    }
                }
            }),
        );
        let summary = artifact.records.last_mut().unwrap();
        for field in ["completed_chunk_streams", "completed_snapshot_streams"] {
            let count = summary["protocol_tracking"][field].as_u64().unwrap();
            summary["protocol_tracking"][field] = json!(count + 1);
        }
        let checkpoint_messages = summary["protocol_tracking"]["checkpoint_messages"]
            .as_u64()
            .unwrap();
        summary["protocol_tracking"]["checkpoint_messages"] = json!(checkpoint_messages + 1);
        let messages = summary["incoming"]["messages"].as_u64().unwrap();
        let events = summary["incoming"]["events"].as_u64().unwrap();
        summary["incoming"]["messages"] = json!(messages + 1);
        summary["incoming"]["frames"] = json!(messages + 1);
        summary["incoming"]["events"] = json!(events + 1);
        let source_summary = summary["incoming"]["sources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["source_instance_id"] == source)
            .unwrap();
        let last = source_summary["last_source_seq"].as_u64().unwrap();
        source_summary["last_source_seq"] = json!(last + 1);
        let checkpoint_end = artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "end"
            })
            .unwrap();
        checkpoint_end["messages_processed_before_end_marker"] = json!(
            checkpoint_end["messages_processed_before_end_marker"]
                .as_u64()
                .unwrap()
                + 1
        );
    }

    #[test]
    fn valid_c13_and_c15_profiles_pass() {
        let c13 = audit_artifact(&valid_artifact(Profile::C13MixerBank)).unwrap();
        assert_eq!(c13.checkpoint_count, 44);
        assert!(!c13.capabilities.direct_access.supported);
        let c13_direct = audit_artifact(&valid_artifact_with_direct_access(
            Profile::C13MixerBank,
            true,
        ))
        .unwrap();
        assert!(c13_direct.capabilities.direct_access.supported);
        assert!(
            c13_direct
                .projections
                .iter()
                .any(|projection| matches!(projection, SemanticProjection::DirectAccess { .. }))
        );
        let mut c13_unexpected_type =
            valid_artifact_with_direct_access(Profile::C13MixerBank, true);
        for record in &mut c13_unexpected_type.records {
            if record["record_type"] == "probe_event"
                && record["message"]["event"] == "probe.capabilities"
            {
                record["message"]["data"]["direct_access"]["get_object_type_name_v1_3"] =
                    json!(true);
            }
            if record["record_type"] == "probe_response"
                && record["message"]["result"]["direct_access"].is_object()
            {
                record["message"]["result"]["direct_access"]["get_object_type_name_v1_3"] =
                    json!(true);
            }
        }
        let c13_unexpected_type = audit_artifact(&c13_unexpected_type).unwrap();
        assert!(c13_unexpected_type.capabilities.direct_access.type_name);
        let c15 = audit_artifact(&valid_artifact(Profile::C15Combined)).unwrap();
        assert!(c15.probe_command_count > c13.probe_command_count);
        assert!(matches!(
            &c15.projections[0],
            SemanticProjection::DirectAccess {
                checkpoint_id: "INIT",
                ..
            }
        ));
        assert!(matches!(
            &c15.projections[1],
            SemanticProjection::MixerBank {
                checkpoint_id: "INIT",
                config_id,
                ..
            } if config_id == "MB_CORE_ALL"
        ));
        assert!(matches!(
            &c15.projections[2],
            SemanticProjection::MixerBank {
                checkpoint_id: "INIT",
                config_id,
                ..
            } if config_id == "MB_CORE_VISIBLE"
        ));

        let mut bank_before_direct = valid_artifact(Profile::C15Combined);
        let direct_index = bank_before_direct
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == "INIT"
                    && record["request"]["message"]["method"] == "probe.direct_access.snapshot"
            })
            .unwrap();
        let bank_index = bank_before_direct
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == "INIT"
                    && record["request"]["message"]["method"] == "probe.bank.snapshot"
                    && record["request"]["message"]["params"]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap();
        let direct_method =
            bank_before_direct.records[direct_index]["request"]["message"]["method"].clone();
        let direct_params =
            bank_before_direct.records[direct_index]["request"]["message"]["params"].clone();
        let bank_method =
            bank_before_direct.records[bank_index]["request"]["message"]["method"].clone();
        let bank_params =
            bank_before_direct.records[bank_index]["request"]["message"]["params"].clone();
        bank_before_direct.records[direct_index]["request"]["message"]["method"] = bank_method;
        bank_before_direct.records[direct_index]["request"]["message"]["params"] = bank_params;
        bank_before_direct.records[bank_index]["request"]["message"]["method"] = direct_method;
        bank_before_direct.records[bank_index]["request"]["message"]["params"] = direct_params;
        assert_eq!(
            audit_artifact(&bank_before_direct).unwrap_err().code,
            "CHECKPOINT_COMMAND_SEQUENCE_INVALID"
        );
    }

    #[test]
    fn direct_access_cycle_references_are_valid_and_projected_explicitly() {
        let mut artifact = valid_artifact(Profile::C15Combined);
        let data = direct_access_command_snapshot_data_mut(&mut artifact, "E1");
        let epoch = data["observation_epoch"].as_u64().unwrap();
        data["items"][0]["child_count"] = json!(1);
        data["items"]
            .as_array_mut()
            .unwrap()
            .push(direct_access_test_reference(
                epoch,
                42,
                42,
                1,
                0,
                0,
                "ancestor_cycle",
            ));
        data["total_items"] = json!(2);
        data["observation_items"] = json!(1);
        data["reference_items"] = json!(1);
        data["cycle_count"] = json!(1);
        data["shared_reference_count"] = json!(0);

        let report = audit_artifact(&artifact).unwrap();
        assert_eq!(report.audit_report_version, 2);
        let projection = report
            .projections
            .iter()
            .find(|projection| {
                matches!(
                    projection,
                    SemanticProjection::DirectAccess {
                        checkpoint_id: "E1",
                        ..
                    }
                )
            })
            .unwrap();
        match projection {
            SemanticProjection::DirectAccess {
                observation_count,
                reference_count,
                cycle_reference_count,
                shared_reference_count,
                unknown_count,
                references,
                ..
            } => {
                assert_eq!(*observation_count, 1);
                assert_eq!(*reference_count, 1);
                assert_eq!(*cycle_reference_count, 1);
                assert_eq!(*shared_reference_count, 0);
                assert_eq!(*unknown_count, 0);
                assert_eq!(references.len(), 1);
                assert_eq!(references[0].reference_index, 0);
                assert_eq!(references[0].target_observation_index, 0);
                assert_eq!(references[0].parent_alias, references[0].target_alias);
                assert_eq!(
                    references[0].reference_kind,
                    DirectAccessReferenceKind::AncestorCycle
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn direct_access_shared_references_and_depth_32_leaf_are_complete() {
        let shared = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 2),
                direct_access_test_observation(7, 1, Some(0), 1, Some(0), 1),
                direct_access_test_observation(7, 2, Some(1), 2, Some(0), 0),
                direct_access_test_reference(7, 2, 0, 1, 1, 2, "shared_reference"),
            ],
            3,
            1,
            0,
            1,
        );
        validate_direct_access_test_assembly(&shared).unwrap();

        let mut depth_items = Vec::new();
        for depth in 0_u64..=32 {
            depth_items.push(direct_access_test_observation(
                7,
                depth,
                depth.checked_sub(1),
                depth,
                (depth > 0).then_some(0),
                u64::from(depth < 32),
            ));
        }
        let depth_boundary = direct_access_test_assembly(depth_items, 33, 0, 0, 0);
        validate_direct_access_test_assembly(&depth_boundary).unwrap();

        let mut reference_depth_items = Vec::new();
        for depth in 0_u64..=31 {
            reference_depth_items.push(direct_access_test_observation(
                7,
                depth,
                depth.checked_sub(1),
                depth,
                (depth > 0).then_some(0),
                1,
            ));
        }
        reference_depth_items.push(direct_access_test_reference(
            7,
            0,
            31,
            32,
            0,
            0,
            "ancestor_cycle",
        ));
        let reference_depth_boundary =
            direct_access_test_assembly(reference_depth_items, 32, 1, 1, 0);
        validate_direct_access_test_assembly(&reference_depth_boundary).unwrap();
    }

    #[test]
    fn direct_access_graph_rejects_dangling_misclassified_count_and_missing_edges() {
        let mut extra_field_reference =
            direct_access_test_reference(7, 0, 0, 1, 0, 0, "ancestor_cycle");
        extra_field_reference["unexpected"] = json!("not allowed");
        let extra_field = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                extra_field_reference,
            ],
            1,
            1,
            1,
            0,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&extra_field)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_REFERENCE_PRIVACY_INVALID"
        );

        let dangling = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                direct_access_test_reference(7, 99, 0, 1, 0, 0, "shared_reference"),
            ],
            1,
            1,
            0,
            1,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&dangling)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let misclassified = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                direct_access_test_reference(7, 0, 0, 1, 0, 0, "shared_reference"),
            ],
            1,
            1,
            0,
            1,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&misclassified)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let count_mismatch = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                direct_access_test_reference(7, 0, 0, 1, 0, 0, "ancestor_cycle"),
            ],
            1,
            1,
            0,
            1,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&count_mismatch)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let missing_edge = direct_access_test_assembly(
            vec![direct_access_test_observation(7, 0, None, 0, None, 1)],
            1,
            0,
            0,
            0,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&missing_edge)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );
    }

    #[test]
    fn direct_access_graph_rejects_forward_and_source_impossible_order() {
        let forward_parent = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                direct_access_test_observation(7, 1, Some(2), 1, Some(0), 0),
            ],
            2,
            0,
            0,
            0,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&forward_parent)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let forward_reference = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 1),
                direct_access_test_reference(7, 1, 0, 1, 0, 1, "shared_reference"),
            ],
            1,
            1,
            0,
            1,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&forward_reference)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let reversed_siblings = direct_access_test_assembly(
            vec![
                direct_access_test_observation(7, 0, None, 0, None, 2),
                direct_access_test_observation(7, 2, Some(0), 1, Some(1), 0),
                direct_access_test_observation(7, 1, Some(0), 1, Some(0), 0),
            ],
            3,
            0,
            0,
            0,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&reversed_siblings)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );
    }

    #[test]
    fn direct_access_graph_enforces_probe_record_depth_and_child_bounds() {
        let mut unsafe_id = direct_access_test_assembly(
            vec![direct_access_test_observation(
                7,
                MAX_DIRECT_ACCESS_OBJECT_ID + 1,
                None,
                0,
                None,
                0,
            )],
            1,
            0,
            0,
            0,
        );
        unsafe_id.direct_access_base_object_id = Some(MAX_DIRECT_ACCESS_OBJECT_ID + 1);
        assert_eq!(
            validate_direct_access_test_assembly(&unsafe_id)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let child_limit = direct_access_test_assembly(
            vec![direct_access_test_observation(7, 0, None, 0, None, 129)],
            1,
            0,
            0,
            0,
        );
        assert_eq!(
            validate_direct_access_test_assembly(&child_limit)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let mut incomplete_depth_boundary_items = Vec::new();
        for depth in 0_u64..=32 {
            incomplete_depth_boundary_items.push(direct_access_test_observation(
                7,
                depth,
                depth.checked_sub(1),
                depth,
                (depth > 0).then_some(0),
                1,
            ));
        }
        let incomplete_depth_boundary =
            direct_access_test_assembly(incomplete_depth_boundary_items, 33, 0, 0, 0);
        assert_eq!(
            validate_direct_access_test_assembly(&incomplete_depth_boundary)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let mut excessive_depth_items = Vec::new();
        for depth in 0_u64..=33 {
            excessive_depth_items.push(direct_access_test_observation(
                7,
                depth,
                depth.checked_sub(1),
                depth,
                (depth > 0).then_some(0),
                u64::from(depth < 33),
            ));
        }
        let depth_limit = direct_access_test_assembly(excessive_depth_items, 34, 0, 0, 0);
        assert_eq!(
            validate_direct_access_test_assembly(&depth_limit)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );

        let mut record_limit_items = vec![
            direct_access_test_observation(7, 0, None, 0, None, 128),
            direct_access_test_observation(7, 1, Some(0), 1, Some(0), 128),
        ];
        for child_index in 0_u64..128 {
            record_limit_items.push(direct_access_test_reference(
                7,
                1,
                1,
                2,
                child_index,
                1,
                "ancestor_cycle",
            ));
        }
        for child_index in 1_u64..128 {
            record_limit_items.push(direct_access_test_reference(
                7,
                0,
                0,
                1,
                child_index,
                0,
                "ancestor_cycle",
            ));
        }
        assert_eq!(record_limit_items.len(), 257);
        let record_limit = direct_access_test_assembly(record_limit_items, 2, 255, 255, 0);
        assert_eq!(
            validate_direct_access_test_assembly(&record_limit)
                .unwrap_err()
                .code,
            "DIRECT_ACCESS_GRAPH_INVALID"
        );
    }

    #[test]
    fn direct_access_chunk_graph_metadata_is_stable_across_chunks() {
        let root = direct_access_test_observation(7, 0, None, 0, None, 1);
        let child = direct_access_test_observation(7, 1, Some(0), 1, Some(0), 1);
        let reference = direct_access_test_reference(7, 0, 1, 2, 0, 0, "ancestor_cycle");
        let make_chunk = |chunk_index: u64,
                          items: Vec<Value>,
                          base_object_id: u64,
                          cycle_count: u64,
                          shared_reference_count: u64| {
            json!({
                "snapshot_id": "SECRET_MULTICHUNK_GRAPH",
                "stream": "direct_access_snapshot",
                "reason": "page_activate",
                "chunk_index": chunk_index,
                "chunk_count": 2,
                "total_items": 3,
                "items": items,
                "snapshot_complete": chunk_index == 1,
                "truncated": false,
                "overflow_safe": true,
                "observation_items": 2,
                "base_object_id": base_object_id,
                "observation_epoch": 7,
                "observation_epoch_status": "snapshot_observed",
                "reference_items": 1,
                "cycle_count": cycle_count,
                "shared_reference_count": shared_reference_count,
                "error_count": 0,
                "truncation_reasons": []
            })
        };

        let first = make_chunk(0, vec![root.clone(), child.clone()], 0, 1, 0);
        let changed_base = make_chunk(1, vec![reference.clone()], 1, 1, 0);
        let mut evidence = Evidence::default();
        collect_snapshot_chunk(
            &mut evidence,
            "INIT",
            "SECRET_GRAPH_SOURCE",
            "probe.direct_access.chunk",
            first.as_object().unwrap(),
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            collect_snapshot_chunk(
                &mut evidence,
                "INIT",
                "SECRET_GRAPH_SOURCE",
                "probe.direct_access.chunk",
                changed_base.as_object().unwrap(),
                2,
                2,
            )
            .unwrap_err()
            .code,
            "SNAPSHOT_CHUNK_SEQUENCE_INVALID"
        );

        let first = make_chunk(0, vec![root, child], 0, 1, 0);
        let changed_counts = make_chunk(1, vec![reference], 0, 0, 1);
        let mut evidence = Evidence::default();
        collect_snapshot_chunk(
            &mut evidence,
            "INIT",
            "SECRET_GRAPH_SOURCE",
            "probe.direct_access.chunk",
            first.as_object().unwrap(),
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            collect_snapshot_chunk(
                &mut evidence,
                "INIT",
                "SECRET_GRAPH_SOURCE",
                "probe.direct_access.chunk",
                changed_counts.as_object().unwrap(),
                2,
                2,
            )
            .unwrap_err()
            .code,
            "SNAPSHOT_CHUNK_SEQUENCE_INVALID"
        );
    }

    #[test]
    fn legacy_and_new_truncated_cycle_snapshots_remain_rejected() {
        let mut legacy = valid_artifact(Profile::C15Combined);
        let data = direct_access_command_snapshot_data_mut(&mut legacy, "E1");
        data["truncated"] = json!(true);
        data["cycle_count"] = json!(1);
        data["truncation_reasons"] = json!(["cycle_detected"]);
        data.as_object_mut().unwrap().remove("reference_items");
        data.as_object_mut()
            .unwrap()
            .remove("shared_reference_count");
        assert_eq!(
            audit_artifact(&legacy).unwrap_err().code,
            "SNAPSHOT_CHUNK_SCHEMA_INVALID"
        );

        let mut truncated = valid_artifact(Profile::C15Combined);
        let data = direct_access_command_snapshot_data_mut(&mut truncated, "E1");
        data["truncated"] = json!(true);
        data["truncation_reasons"] = json!(["cycle_detected"]);
        assert_eq!(
            audit_artifact(&truncated).unwrap_err().code,
            "SNAPSHOT_CHUNK_INVALID"
        );
    }

    #[test]
    fn exact_start_time_calendar_and_os_build_are_enforced() {
        let mut mismatch = valid_artifact(Profile::C13MixerBank);
        mismatch.manifest["run_started_at"] = json!("2027-01-15T08:00:00.001+00:00");
        assert_eq!(
            audit_artifact(&mismatch).unwrap_err().code,
            "RUN_START_TIME_MISMATCH"
        );

        let mut invalid_calendar = valid_artifact(Profile::C13MixerBank);
        invalid_calendar.manifest["run_started_at"] = json!("2027-02-29T08:00:00.000+00:00");
        assert_eq!(
            audit_artifact(&invalid_calendar).unwrap_err().code,
            "RUN_STARTED_AT_INVALID"
        );

        let mut invalid_shape = valid_artifact(Profile::C13MixerBank);
        invalid_shape.manifest["run_started_at"] = json!("2027-01-15T08:00:00Z");
        assert_eq!(
            audit_artifact(&invalid_shape).unwrap_err().code,
            "RUN_STARTED_AT_INVALID"
        );

        let mut invalid_offset = valid_artifact(Profile::C13MixerBank);
        invalid_offset.manifest["run_started_at"] = json!("2027-01-15T08:00:00.000+24:00");
        assert_eq!(
            audit_artifact(&invalid_offset).unwrap_err().code,
            "RUN_STARTED_AT_INVALID"
        );

        let mut secret_build = valid_artifact(Profile::C13MixerBank);
        secret_build.manifest["environment"]["os"]["build"] = json!("ghp_SECRET123");
        assert_eq!(
            audit_artifact(&secret_build).unwrap_err().code,
            "OS_METADATA_INVALID"
        );
    }

    #[test]
    fn action_activation_and_exact_window_contracts_fail_closed() {
        let mut missing_action = valid_artifact(Profile::C13MixerBank);
        remove_records(&mut missing_action, |record| {
            record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
        });
        assert_eq!(
            audit_artifact(&missing_action).unwrap_err().code,
            "CHECKPOINT_COVERAGE_INVALID"
        );

        let mut incomplete_activation = valid_artifact(Profile::C13MixerBank);
        let initial_visible = incomplete_activation
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "INIT"
                    && record["message"]["data"]["reason"] == "page_activate"
                    && record["message"]["data"]["config_id"] == "MB_CORE_VISIBLE"
            })
            .unwrap();
        initial_visible["message"]["data"]["reason"] = json!("unexpected_initial_reason");
        assert_eq!(
            audit_artifact(&incomplete_activation).unwrap_err().code,
            "SNAPSHOT_CHUNK_INVALID"
        );

        let mut loaded_before_action = valid_artifact(Profile::C13MixerBank);
        let action_monotonic = loaded_before_action
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "INIT"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let loaded = loaded_before_action
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "INIT"
                    && record["message"]["event"] == "probe.loaded"
            })
            .unwrap();
        loaded["received_at_monotonic_timestamp_ms"] = json!(action_monotonic - 1);
        assert_eq!(
            audit_artifact(&loaded_before_action).unwrap_err().code,
            "ACTIVATION_NEW_SESSION_MISSING"
        );

        let mut missing_inactive = valid_artifact(Profile::C13MixerBank);
        let inactive = missing_inactive
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "R1"
                    && record["message"]["event"] == "probe.ready"
                    && record["message"]["data"]["ready"] == false
            })
            .unwrap();
        inactive["message"]["data"]["ready"] = json!(true);
        assert_eq!(
            audit_artifact(&missing_inactive).unwrap_err().code,
            "PROBE_SOURCE_LIFECYCLE_INVALID"
        );

        let mut nonzero_new_session = valid_artifact(Profile::C13MixerBank);
        let capability = nonzero_new_session
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "R1"
                    && record["message"]["event"] == "probe.capabilities"
            })
            .unwrap();
        capability["message"]["data"]["observation_epoch"]["current"] = json!(1);
        assert_eq!(
            audit_artifact(&nonzero_new_session).unwrap_err().code,
            "ACTIVATION_CAPABILITY_INVALID"
        );

        let mut wrong_window = valid_artifact(Profile::C13MixerBank);
        let begin = wrong_window
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == "E0"
                    && record["phase"] == "begin"
            })
            .unwrap();
        begin["window_ms"] = json!(5001);
        assert_eq!(
            audit_artifact(&wrong_window).unwrap_err().code,
            "CHECKPOINT_BEGIN_INVALID"
        );
    }

    #[test]
    fn responses_may_precede_selected_evidence_but_selected_proof_is_required() {
        let mut reordered = valid_artifact(Profile::C13MixerBank);
        move_response_before_command(&mut reordered, "E0", "probe.bank.snapshot");
        audit_artifact(&reordered).unwrap();

        let mut missing_selected_proof = valid_artifact(Profile::C13MixerBank);
        let command = missing_selected_proof
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["request"]["message"]["method"] == "probe.bank.snapshot"
            })
            .unwrap();
        command.as_object_mut().unwrap().remove("evidence_emission");
        assert_eq!(
            audit_artifact(&missing_selected_proof).unwrap_err().code,
            "SELECTED_COMMAND_EVIDENCE_MISSING"
        );
    }

    #[test]
    fn unsolicited_activity_between_final_projections_is_rejected() {
        let mut unknown = valid_artifact(Profile::C13MixerBank);
        unknown
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
            })
            .unwrap()["message"]["event"] = json!("probe.unknown_event");
        assert_eq!(
            audit_artifact(&unknown).unwrap_err().code,
            "PROBE_EVENT_UNEXPECTED"
        );

        let mut before_cut = valid_artifact(Profile::C15Combined);
        insert_pre_cut_direct_access_snapshot(&mut before_cut, "E0");
        audit_artifact(&before_cut).unwrap();

        let mut artifact = valid_artifact(Profile::C15Combined);
        insert_unsolicited_after_first_final(&mut artifact, "E0");
        assert_eq!(
            audit_artifact(&artifact).unwrap_err().code,
            "PROBE_ACTIVITY_BETWEEN_FINAL_PROJECTIONS"
        );
    }

    #[test]
    fn empty_and_zero_command_logs_are_rejected() {
        let artifact = valid_artifact(Profile::C13MixerBank);
        assert_eq!(
            audit_bytes(&encode_json(&artifact.manifest), b"")
                .unwrap_err()
                .code,
            "LOG_SIZE_INVALID"
        );

        let mut artifact = valid_artifact(Profile::C13MixerBank);
        remove_records(&mut artifact, |record| {
            matches!(
                record["record_type"].as_str(),
                Some(
                    "probe_command"
                        | "probe_command_send_result"
                        | "probe_event"
                        | "probe_response"
                        | "probe_error"
                        | "collector_discovery_completed"
                )
            )
        });
        for record in &mut artifact.records {
            if record["record_type"] == "collector_checkpoint" && record["phase"] == "end" {
                record["messages_processed_before_end_marker"] = json!(0);
            }
        }
        let summary = artifact.records.last_mut().unwrap();
        summary["commands"]["sent"] = json!(0);
        summary["commands"]["received"] = json!((REQUIRED_CHECKPOINTS.len() * 3) as u64);
        assert_eq!(
            audit_artifact(&artifact).unwrap_err().code,
            "NO_PROBE_COMMANDS"
        );
    }

    #[test]
    fn missing_and_duplicate_checkpoints_are_rejected() {
        let mut missing = valid_artifact(Profile::C13MixerBank);
        remove_records(&mut missing, |record| {
            record["record_type"] == "collector_checkpoint" && record["checkpoint_id"] == "E0"
        });
        assert_eq!(
            audit_artifact(&missing).unwrap_err().code,
            "PROBE_CHECKPOINT_CONTEXT_INVALID"
        );

        let mut duplicate = valid_artifact(Profile::C13MixerBank);
        let begin = duplicate
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == "E0"
                    && record["phase"] == "begin"
            })
            .unwrap()
            .clone();
        let insertion = duplicate
            .records
            .iter()
            .position(|record| record == &begin)
            .unwrap()
            + 1;
        duplicate.records.insert(insertion, begin);
        assert_eq!(
            audit_artifact(&duplicate).unwrap_err().code,
            "CHECKPOINT_BEGIN_INVALID"
        );
    }

    #[test]
    fn invalid_hash_profile_and_revision_are_rejected() {
        let mut invalid_hash = valid_artifact(Profile::C13MixerBank);
        invalid_hash.manifest["environment"]["deployed_probe_sha256"] = json!("A".repeat(64));
        assert_eq!(
            audit_artifact(&invalid_hash).unwrap_err().code,
            "ARTIFACT_HASH_INVALID"
        );

        let mut invalid_profile = valid_artifact(Profile::C13MixerBank);
        invalid_profile.manifest["profile"] = json!("unknown_profile");
        assert_eq!(
            audit_artifact(&invalid_profile).unwrap_err().code,
            "MANIFEST_JSON_INVALID"
        );

        let mut invalid_revision = valid_artifact(Profile::C13MixerBank);
        invalid_revision.manifest["fixture_revision"] = json!(1);
        assert_eq!(
            audit_artifact(&invalid_revision).unwrap_err().code,
            "FIXTURE_REVISION_INVALID"
        );
    }

    #[test]
    fn error_response_and_absent_final_snapshot_are_rejected() {
        let mut error = valid_artifact(Profile::C13MixerBank);
        let response = error
            .records
            .iter_mut()
            .find(|record| record["record_type"] == "probe_response")
            .unwrap();
        response["record_type"] = json!("probe_error");
        response["message"]["type"] = json!("error");
        response["message"]
            .as_object_mut()
            .unwrap()
            .remove("result");
        response["message"]["error"] = json!({"code": "TEST", "message": "test"});
        assert_eq!(
            audit_artifact(&error).unwrap_err().code,
            "PROBE_ERROR_OBSERVED"
        );

        let mut missing = valid_artifact(Profile::C13MixerBank);
        let missing_snapshot_id = missing
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "E0"
                    && record["message"]["event"] == "probe.bank.chunk"
                    && record["message"]["data"]["reason"] == "command_snapshot"
                    && record["message"]["data"]["config_id"] == "MB_CORE_VISIBLE"
            })
            .unwrap()["message"]["data"]["snapshot_id"]
            .as_str()
            .unwrap()
            .to_owned();
        for chunk in missing.records.iter_mut().filter(|record| {
            record["record_type"] == "probe_event"
                && record["message"]["data"]["snapshot_id"] == missing_snapshot_id
        }) {
            chunk["message"]["data"]["reason"] = json!("command_reset");
        }
        assert_eq!(
            audit_artifact(&missing).unwrap_err().code,
            "COMMAND_SNAPSHOT_FOLLOWUP_MISSING"
        );
    }

    #[test]
    fn bounded_input_and_line_limits_fail_closed() {
        let artifact = valid_artifact(Profile::C13MixerBank);
        let manifest = encode_json(&artifact.manifest);
        assert_eq!(
            audit_bytes(&vec![b' '; MAX_MANIFEST_BYTES + 1], b"{}\n")
                .unwrap_err()
                .code,
            "MANIFEST_SIZE_INVALID"
        );
        let mut oversized_line = vec![b'x'; MAX_LOG_LINE_BYTES + 1];
        oversized_line.push(b'\n');
        assert_eq!(
            audit_bytes(&manifest, &oversized_line).unwrap_err().code,
            "LOG_LINE_TOO_LARGE"
        );

        let mut no_newline = encode_jsonl(&artifact.records);
        no_newline.pop();
        assert_eq!(
            audit_bytes(&manifest, &no_newline).unwrap_err().code,
            "LOG_FINAL_NEWLINE_MISSING"
        );

        for duplicate in [
            br#"{"record_type":"x","record_type":"y","timestamp_unix_ms":1,"monotonic_timestamp_ms":1}
"#
                .as_slice(),
            br#"{"record_type":"x","timestamp_unix_ms":1,"monotonic_timestamp_ms":1,"nested":{"key":1,"key":2}}
"#
                .as_slice(),
        ] {
            assert_eq!(
                parse_jsonl(duplicate).unwrap_err().code,
                "LOG_RECORD_JSON_INVALID"
            );
        }
    }

    #[test]
    fn report_is_redacted_even_when_raw_log_contains_sensitive_values() {
        let mut artifact = valid_artifact(Profile::C15Combined);
        artifact.manifest["fixture_acceptance"]["alternate_plugins"]["instrument"] = json!({
            "status": "used",
            "accepted_name": "SUPER_SECRET_PLUGIN_NAME"
        });
        let manifest_bytes = encode_json(&artifact.manifest);
        let raw_bytes = encode_jsonl(&artifact.records);
        let report = audit_bytes(&manifest_bytes, &raw_bytes).unwrap();
        let encoded = serde_json::to_string(&report).unwrap();
        for secret in [
            "SECRET_INSTANCE",
            "SECRET_SNAPSHOT",
            "SECRET_COLLECTOR_SESSION",
            "SECRET_PORT",
            "/Users/private",
            "target_instance_id",
            "request-",
            TEST_RUN_ID,
            "SUPER_SECRET_PLUGIN_NAME",
            "TOP_SECRET_TRACK_TITLE",
            "TOP_SECRET_UNIQUE_NAME",
            "TOP_SECRET_TYPE",
            "SECRET_HOST_ID",
            "ghp_SECRET",
        ] {
            assert!(!encoded.contains(secret));
        }
        assert!(encoded.contains("c15_combined"));
        assert!(encoded.contains(COLLECTOR_HASH));
        assert_eq!(report.evidence_sha256.manifest, sha256_hex(&manifest_bytes));
        assert_eq!(report.evidence_sha256.raw_jsonl, sha256_hex(&raw_bytes));
        assert!(report.fixture_acceptance.alternate_instrument_plugin_used);
        assert!(report.run_alias.starts_with("run-"));
    }

    #[test]
    fn fixture_acceptance_is_strict_but_known_host_variants_remain_semantic() {
        let mut invalid = valid_artifact(Profile::C13MixerBank);
        invalid.manifest["fixture_acceptance"]["p05_title"]["unicode_scalar_count"] = json!(999);
        assert_eq!(
            audit_artifact(&invalid).unwrap_err().code,
            "P05_ACCEPTANCE_INVALID"
        );

        let mut observed_variant = valid_artifact(Profile::C13MixerBank);
        let item = observed_variant
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["reason"] == "command_snapshot"
                    && record["message"]["data"]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap();
        item["message"]["data"]["items"][0]["title"] = json!(P05_TITLE_NFD);
        let feedback = observed_variant
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
                    && record["message"]["data"]["items"][0]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap();
        feedback["message"]["data"]["items"][0]["title"] = json!(P05_TITLE_NFD);
        feedback["message"]["data"]["items"][0]["changed_value"] = json!(P05_TITLE_NFD);
        let report = audit_artifact(&observed_variant).unwrap();
        assert!(report.projections.iter().any(|projection| {
            matches!(
                projection,
                SemanticProjection::MixerBank {
                    checkpoint_id: "C1",
                    p05_title,
                    ..
                } if p05_title.safe_known_variant_count == 1
            )
        }));
    }

    #[test]
    fn fragmented_host_ids_are_aliased_only_after_complete_validation() {
        let observation = json!({
            "host_id_raw": null,
            "host_id_byte_length": 4,
            "host_id_ref": "SECRET_FRAGMENT_REF",
            "host_id_fragment_count": 2
        });
        let items = vec![
            observation.clone(),
            json!({
                "record_kind": "host_id_fragment",
                "host_id_ref": "SECRET_FRAGMENT_REF",
                "host_id_byte_length": 4,
                "fragment_index": 0,
                "fragment_count": 2,
                "fragment": "SE"
            }),
            json!({
                "record_kind": "host_id_fragment",
                "host_id_ref": "SECRET_FRAGMENT_REF",
                "host_id_byte_length": 4,
                "fragment_index": 1,
                "fragment_count": 2,
                "fragment": "CR"
            }),
        ];
        let mut aliases = AliasState::default();
        let (alias, length) =
            resolve_host_id(observation.as_object().unwrap(), &items, &mut aliases);
        assert_eq!(alias.as_deref(), Some("H000001"));
        assert_eq!(length, Some(4));
        let mut incomplete = items;
        incomplete.pop();
        assert_eq!(
            resolve_host_id(observation.as_object().unwrap(), &incomplete, &mut aliases,),
            (None, None)
        );
    }

    #[test]
    fn short_p09_prefix_and_unsafe_snapshot_strings_are_rejected() {
        let mut short_prefix = valid_artifact(Profile::C13MixerBank);
        short_prefix.manifest["fixture_acceptance"]["p09_title"]["accepted_title"] = json!("CMCP");
        short_prefix.manifest["fixture_acceptance"]["p09_title"]["unicode_scalar_count"] = json!(4);
        short_prefix.manifest["fixture_acceptance"]["p09_title"]["utf8_byte_length"] = json!(4);
        short_prefix.manifest["fixture_acceptance"]["p09_title"]["setup_variance"] = json!(true);
        assert_eq!(
            audit_artifact(&short_prefix).unwrap_err().code,
            "P09_ACCEPTANCE_INVALID"
        );

        let mut leaked_title = valid_artifact(Profile::C13MixerBank);
        let item = leaked_title
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["reason"] == "command_snapshot"
                    && record["message"]["data"]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap();
        item["message"]["data"]["items"][0]["title"] = json!("ghp_SECRET123");
        assert_eq!(
            audit_artifact(&leaked_title).unwrap_err().code,
            "TITLE_PRIVACY_INVALID"
        );
    }

    #[test]
    fn checkpoint_order_and_observation_cut_are_fixed_contracts() {
        let mut reordered = valid_artifact(Profile::C13MixerBank);
        for record in &mut reordered.records {
            if record["checkpoint_id"] == "E0" {
                record["checkpoint_id"] = json!("TEMP");
            } else if record["checkpoint_id"] == "E1" {
                record["checkpoint_id"] = json!("E0");
            }
        }
        for record in &mut reordered.records {
            if record["checkpoint_id"] == "TEMP" {
                record["checkpoint_id"] = json!("E1");
            }
        }
        assert_eq!(
            audit_artifact(&reordered).unwrap_err().code,
            "CHECKPOINT_ORDER_INVALID"
        );

        let mut wrong_epoch = valid_artifact(Profile::C13MixerBank);
        let response = wrong_epoch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["checkpoint_id"] == "E0"
                    && record["message"]["result"]
                        .get("observation_epoch")
                        .is_some()
            })
            .unwrap();
        response["message"]["result"]["observation_epoch"] = json!(99);
        wrong_epoch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap()["observation_epoch"] = json!(99);
        assert_eq!(
            audit_artifact(&wrong_epoch).unwrap_err().code,
            "OBSERVATION_EPOCH_SEQUENCE_INVALID"
        );

        let mut missing_cut = valid_artifact(Profile::C13MixerBank);
        let cut_request = missing_cut
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == "E0"
                    && record["request"]["message"]["method"] == "probe.observation.cut"
            })
            .unwrap()["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        remove_records(&mut missing_cut, |record| {
            matches!(
                record["record_type"].as_str(),
                Some("probe_command" | "probe_command_send_result")
            ) && record["request_id"].as_str() == Some(&cut_request)
        });
        assert_eq!(
            audit_artifact(&missing_cut).unwrap_err().code,
            "CHECKPOINT_COMMAND_SEQUENCE_INVALID"
        );

        let mut late_cut = valid_artifact(Profile::C13MixerBank);
        let cut_response_at = late_cut
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["checkpoint_id"] == "E0"
                    && record["message"]["result"]
                        .get("observation_epoch")
                        .is_some()
            })
            .unwrap()["received_at_monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let action = late_cut
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap();
        action["monotonic_timestamp_ms"] = json!(cut_response_at + 1_001);
        action["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + cut_response_at + 1_001);
        assert_eq!(
            audit_artifact(&late_cut).unwrap_err().code,
            "OBSERVATION_CUT_ACTION_ORDER_INVALID"
        );

        let mut early_after_cut = valid_artifact(Profile::C13MixerBank);
        let action_at = early_after_cut
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let final_request = early_after_cut
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == "E0"
                    && record["request"]["message"]["method"] == "probe.bank.snapshot"
                    && record["request"]["message"]["params"]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap()["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        early_after_cut
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_command_send_result"
                    && record["request_id"].as_str() == Some(&final_request)
            })
            .unwrap()["send_completed_monotonic_timestamp_ms"] =
            json!(action_at + CALLBACK_WINDOW_MS - 1);
        assert_eq!(
            audit_artifact(&early_after_cut).unwrap_err().code,
            "FINAL_SNAPSHOT_STARTED_TOO_EARLY"
        );

        let mut early_after_navigation = valid_artifact(Profile::C13MixerBank);
        let checkpoint_id = "E8-MB_CORE_ALL-B0-reset";
        let operation_request = early_after_navigation
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["request"]["message"]["method"] == "probe.bank.reset"
            })
            .unwrap()["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let operation_response_at = early_after_navigation
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["message"]["id"].as_str() == Some(&operation_request)
            })
            .unwrap()["received_at_monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let final_request = early_after_navigation
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["request"]["message"]["method"] == "probe.bank.snapshot"
            })
            .unwrap()["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        early_after_navigation
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_command_send_result"
                    && record["request_id"].as_str() == Some(&final_request)
            })
            .unwrap()["send_completed_monotonic_timestamp_ms"] =
            json!(operation_response_at + CALLBACK_WINDOW_MS - 1);
        assert_eq!(
            audit_artifact(&early_after_navigation).unwrap_err().code,
            "FINAL_SNAPSHOT_STARTED_TOO_EARLY"
        );
    }

    #[test]
    fn epoch_mismatch_is_reported_as_stale_or_rejected_for_live_direct_access() {
        let mut stale_bank = valid_artifact(Profile::C13MixerBank);
        for record in &mut stale_bank.records {
            if record["record_type"] == "probe_event"
                && record["checkpoint_id"] == "C1"
                && record["message"]["data"]["stream"] == "mixer_bank_feedback"
                && record["message"]["data"]["items"][0]["config_id"] == "MB_CORE_ALL"
            {
                record["message"]["data"]["items"][0]["observation_epoch"] = json!(3);
                record["message"]["data"]["items"][0]["field_last_observation_epoch"]["title"] =
                    json!(3);
                record["message"]["data"]["items"][0]["field_last_observation_epoch"]["host_id_raw"] =
                    json!(3);
            }
            if record["record_type"] == "probe_event"
                && record["checkpoint_id"] == "C1"
                && record["message"]["data"]["reason"] == "command_snapshot"
                && record["message"]["data"]["config_id"] == "MB_CORE_ALL"
            {
                record["message"]["data"]["items"][0]["field_last_observation_epoch"]["title"] =
                    json!(3);
                record["message"]["data"]["items"][0]["field_last_observation_epoch"]["host_id_raw"] =
                    json!(3);
            }
        }
        let report = audit_artifact(&stale_bank).unwrap();
        assert!(report.projections.iter().any(|projection| {
            matches!(projection, SemanticProjection::MixerBank {
                checkpoint_id: "C1",
                config_id,
                slots,
                ..
            } if config_id == "MB_CORE_ALL" && slots.iter().any(|slot| {
                slot.slot_index == 0
                    && slot.title.is_none()
                    && slot.title_freshness == FieldFreshness::StaleOrUnobserved
                    && slot.host_id_alias.is_none()
            }))
        }));

        let mut direct_mismatch = valid_artifact(Profile::C15Combined);
        let chunk = direct_mismatch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["reason"] == "command_snapshot"
                    && record["message"]["data"]["stream"] == "direct_access_snapshot"
            })
            .unwrap();
        chunk["message"]["data"]["observation_epoch"] = json!(3);
        chunk["message"]["data"]["items"][0]["observation_epoch"] = json!(3);
        assert_eq!(
            audit_artifact(&direct_mismatch).unwrap_err().code,
            "DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"
        );

        let mut navigation_mismatch = valid_artifact(Profile::C15Combined);
        let chunk = navigation_mismatch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "E8-MB_CORE_ALL-B0-reset"
                    && record["message"]["data"]["reason"] == "command_snapshot"
                    && record["message"]["data"]["stream"] == "direct_access_snapshot"
            })
            .unwrap();
        chunk["message"]["data"]["observation_epoch"] = json!(0);
        chunk["message"]["data"]["items"][0]["observation_epoch"] = json!(0);
        assert_eq!(
            audit_artifact(&navigation_mismatch).unwrap_err().code,
            "DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"
        );

        let mut activation_mismatch = valid_artifact(Profile::C15Combined);
        let chunk = activation_mismatch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "R1"
                    && record["message"]["data"]["reason"] == "page_activate"
                    && record["message"]["data"]["stream"] == "direct_access_snapshot"
            })
            .unwrap();
        chunk["message"]["data"]["observation_epoch"] = json!(1);
        chunk["message"]["data"]["items"][0]["observation_epoch"] = json!(1);
        assert_eq!(
            audit_artifact(&activation_mismatch).unwrap_err().code,
            "DIRECT_ACCESS_OBSERVATION_EPOCH_MISMATCH"
        );
    }

    #[test]
    fn feedback_sequence_and_epoch_are_source_global_and_bounded() {
        let mutate_direct_feedback =
            |artifact: &mut TestArtifact, mut mutation: Box<dyn FnMut(&mut Value)>| {
                let record = artifact
                    .records
                    .iter_mut()
                    .find(|record| {
                        record["record_type"] == "probe_event"
                            && record["checkpoint_id"] == "C1"
                            && record["message"]["data"]["stream"] == "direct_access_feedback"
                    })
                    .unwrap();
                mutation(&mut record["message"]["data"]["items"][0]);
                let sequence = record["message"]["data"]["items"][0]["observation_seq"].clone();
                record["message"]["data"]["first_observation_seq"] = sequence.clone();
                record["message"]["data"]["last_observation_seq"] = sequence;
            };

        let mut future = valid_artifact(Profile::C15Combined);
        mutate_direct_feedback(
            &mut future,
            Box::new(|item| item["observation_epoch"] = json!(5)),
        );
        assert_eq!(
            audit_artifact(&future).unwrap_err().code,
            "FEEDBACK_OBSERVATION_EPOCH_FUTURE"
        );

        let mut duplicate = valid_artifact(Profile::C15Combined);
        mutate_direct_feedback(
            &mut duplicate,
            Box::new(|item| item["observation_seq"] = json!(2)),
        );
        assert_eq!(
            audit_artifact(&duplicate).unwrap_err().code,
            "FEEDBACK_OBSERVATION_SEQUENCE_INVALID"
        );

        let mut reordered = valid_artifact(Profile::C15Combined);
        mutate_direct_feedback(
            &mut reordered,
            Box::new(|item| item["observation_seq"] = json!(1)),
        );
        assert_eq!(
            audit_artifact(&reordered).unwrap_err().code,
            "FEEDBACK_OBSERVATION_SEQUENCE_INVALID"
        );
    }

    #[test]
    fn drain_boundary_is_mandatory_ordered_and_summary_bound() {
        let mut missing = valid_artifact(Profile::C13MixerBank);
        remove_records(&mut missing, |record| {
            record["record_type"] == "collector_drain_started"
        });
        assert_eq!(
            audit_artifact(&missing).unwrap_err().code,
            "DRAIN_STARTED_MISSING"
        );

        let mut duplicate = valid_artifact(Profile::C13MixerBank);
        let start_index = duplicate
            .records
            .iter()
            .position(|record| record["record_type"] == "collector_drain_started")
            .unwrap();
        let cloned = duplicate.records[start_index].clone();
        duplicate.records.insert(start_index + 1, cloned);
        assert_eq!(
            audit_artifact(&duplicate).unwrap_err().code,
            "DRAIN_STARTED_DUPLICATE"
        );

        let mut misordered = valid_artifact(Profile::C13MixerBank);
        let start_index = misordered
            .records
            .iter()
            .position(|record| record["record_type"] == "collector_drain_started")
            .unwrap();
        let mut start = misordered.records.remove(start_index);
        let r2_end = misordered
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == "R2"
                    && record["phase"] == "end"
            })
            .unwrap();
        let prior_monotonic = misordered.records[r2_end - 1]["monotonic_timestamp_ms"].clone();
        let prior_unix = misordered.records[r2_end - 1]["timestamp_unix_ms"].clone();
        start["monotonic_timestamp_ms"] = prior_monotonic.clone();
        start["timestamp_unix_ms"] = prior_unix;
        start["deadline_monotonic_timestamp_ms"] = json!(prior_monotonic.as_u64().unwrap() + 5000);
        misordered.records.insert(r2_end, start);
        assert_eq!(
            audit_artifact(&misordered).unwrap_err().code,
            "COLLECTOR_DRAIN_BOUNDARY_INVALID"
        );

        let mut mismatch = valid_artifact(Profile::C13MixerBank);
        for record in &mut mismatch.records {
            if record["record_type"] == "collector_drain_completed" {
                record["duration_ms"] = json!(1);
            }
            if record["record_type"] == "collector_summary" {
                record["graceful_drain"]["duration_ms"] = json!(1);
            }
        }
        assert_eq!(
            audit_artifact(&mismatch).unwrap_err().code,
            "COLLECTOR_DRAIN_BOUNDARY_INVALID"
        );
    }

    #[test]
    fn auto_cut_action_is_atomically_bound_to_the_response() {
        let mut fast_response = valid_artifact(Profile::C13MixerBank);
        let request_id = fast_response
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == "E0"
                    && record["request"]["message"]["method"] == "probe.observation.cut"
            })
            .unwrap()["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let first_index = fast_response
            .records
            .iter()
            .position(|record| record["request_id"].as_str() == Some(&request_id))
            .unwrap();
        let mut command = fast_response
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command"
                    && record["request_id"].as_str() == Some(&request_id)
            })
            .unwrap()
            .clone();
        let mut send = fast_response
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_command_send_result"
                    && record["request_id"].as_str() == Some(&request_id)
            })
            .unwrap()
            .clone();
        let response = fast_response
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["message"]["id"].as_str() == Some(&request_id)
            })
            .unwrap()
            .clone();
        let action = fast_response
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_action"
                    && record["request_id"].as_str() == Some(&request_id)
            })
            .unwrap()
            .clone();
        fast_response.records.retain(|record| {
            record["request_id"].as_str() != Some(&request_id)
                && !(record["record_type"] == "probe_response"
                    && record["message"]["id"].as_str() == Some(&request_id))
        });
        let action_at = action["monotonic_timestamp_ms"].as_u64().unwrap();
        command["monotonic_timestamp_ms"] = json!(action_at + 1);
        command["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + action_at + 1);
        send["monotonic_timestamp_ms"] = json!(action_at + 2);
        send["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + action_at + 2);
        fast_response
            .records
            .splice(first_index..first_index, [response, action, command, send]);
        audit_artifact(&fast_response).unwrap();

        let mut wrong_source = valid_artifact(Profile::C13MixerBank);
        wrong_source
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap()["boundary_source"] = json!("manual");
        assert_eq!(
            audit_artifact(&wrong_source).unwrap_err().code,
            "ACTION_BOUNDARY_INVALID"
        );

        let mut wrong_epoch = valid_artifact(Profile::C13MixerBank);
        wrong_epoch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap()["observation_epoch"] = json!(99);
        assert_eq!(
            audit_artifact(&wrong_epoch).unwrap_err().code,
            "OBSERVATION_CUT_ACTION_ORDER_INVALID"
        );

        let mut nonadjacent = valid_artifact(Profile::C13MixerBank);
        let action_index = nonadjacent
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "E0"
            })
            .unwrap();
        let mut action = nonadjacent.records.remove(action_index);
        let next_command_index = nonadjacent
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command" && record["checkpoint_id"] == "E0"
            })
            .unwrap();
        let at = nonadjacent.records[next_command_index + 1]["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        action["monotonic_timestamp_ms"] = json!(at);
        action["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + at);
        nonadjacent.records.insert(next_command_index + 2, action);
        assert_eq!(
            audit_artifact(&nonadjacent).unwrap_err().code,
            "OBSERVATION_CUT_ACTION_ORDER_INVALID"
        );
    }

    #[test]
    fn exact_probe_capability_chunk_response_and_summary_schemas_fail_closed() {
        let mut outer = valid_artifact(Profile::C13MixerBank);
        outer
            .records
            .iter_mut()
            .find(|record| record["record_type"] == "probe_event")
            .unwrap()["unexpected"] = json!("ghp_SECRET");
        assert_eq!(
            audit_artifact(&outer).unwrap_err().code,
            "PROBE_RECORD_SCHEMA_INVALID"
        );

        let mut capability = valid_artifact(Profile::C13MixerBank);
        capability
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["message"]["event"] == "probe.capabilities"
            })
            .unwrap()["message"]["data"]["unexpected"] = json!(true);
        assert_eq!(
            audit_artifact(&capability).unwrap_err().code,
            "CAPABILITY_RESULT_INVALID"
        );

        let mut mixer_false = valid_artifact(Profile::C13MixerBank);
        for record in &mut mixer_false.records {
            if record["record_type"] == "probe_event"
                && record["message"]["event"] == "probe.capabilities"
            {
                record["message"]["data"]["mixer_bank"]["mute"] = json!(false);
            }
            if record["record_type"] == "probe_response"
                && record["message"]["result"]["mixer_bank"].is_object()
            {
                record["message"]["result"]["mixer_bank"]["mute"] = json!(false);
            }
        }
        assert_eq!(
            audit_artifact(&mixer_false).unwrap_err().code,
            "MIXER_BANK_CAPABILITY_INVALID"
        );

        let mut chunk = valid_artifact(Profile::C13MixerBank);
        chunk
            .records
            .iter_mut()
            .find(|record| record["message"]["data"]["snapshot_id"].is_string())
            .unwrap()["message"]["data"]["unexpected"] = json!(true);
        assert_eq!(
            audit_artifact(&chunk).unwrap_err().code,
            "SNAPSHOT_CHUNK_SCHEMA_INVALID"
        );

        let mut response = valid_artifact(Profile::C13MixerBank);
        response
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["message"]["result"]["config_id"].is_string()
                    && record["message"]["result"].get("action").is_none()
            })
            .unwrap()["message"]["result"]["unexpected"] = json!("SECRET");
        assert_eq!(
            audit_artifact(&response).unwrap_err().code,
            "COMMAND_RESPONSE_RESULT_INVALID"
        );

        let mut summary = valid_artifact(Profile::C13MixerBank);
        let events = summary.records.last().unwrap()["incoming"]["events"]
            .as_u64()
            .unwrap();
        summary.records.last_mut().unwrap()["incoming"]["events"] = json!(events + 1);
        assert_eq!(
            audit_artifact(&summary).unwrap_err().code,
            "INCOMING_MESSAGE_COUNT_MISMATCH"
        );
    }

    #[test]
    fn summary_and_init_epoch_are_bound_to_the_final_session() {
        let mut wrong_selected = valid_artifact(Profile::C13MixerBank);
        let protocol = &mut wrong_selected.records.last_mut().unwrap()["protocol_tracking"];
        protocol["selected_source_instance_id"] = json!("SECRET_INSTANCE_INIT");
        protocol["active_source_instance_ids"] = json!(["SECRET_INSTANCE_INIT"]);
        assert_eq!(
            audit_artifact(&wrong_selected).unwrap_err().code,
            "PROTOCOL_SUMMARY_R2_SOURCE_INVALID"
        );

        let mut wrong_source_seq = valid_artifact(Profile::C13MixerBank);
        wrong_source_seq.records.last_mut().unwrap()["incoming"]["sources"][0]["last_source_seq"] =
            json!(999);
        assert_eq!(
            audit_artifact(&wrong_source_seq).unwrap_err().code,
            "INCOMING_SOURCE_SUMMARY_INVALID"
        );

        let mut init_epoch = valid_artifact(Profile::C13MixerBank);
        init_epoch
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_response"
                    && record["checkpoint_id"] == "INIT"
                    && record["message"]["result"]["observation_epoch"].is_object()
            })
            .unwrap()["message"]["result"]["observation_epoch"]["current"] = json!(1);
        assert_eq!(
            audit_artifact(&init_epoch).unwrap_err().code,
            "INITIAL_CAPABILITY_EPOCH_INVALID"
        );
    }

    #[test]
    fn inactive_source_messages_and_callback_source_spoofing_fail_closed() {
        let mut stale = valid_artifact(Profile::C13MixerBank);
        let response_index = stale
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_response"
                    && record["checkpoint_id"] == "R1"
                    && record["message"]["result"]["instance_id"].is_string()
            })
            .unwrap();
        let removed_seq = stale.records[response_index]["source_seq"]
            .as_u64()
            .unwrap();
        stale.records[response_index]["source_instance_id"] = json!("SECRET_INSTANCE_INIT");
        stale.records[response_index]["source_seq"] =
            stale.records.last().unwrap()["incoming"]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["source_instance_id"] == "SECRET_INSTANCE_INIT")
                .unwrap()["last_source_seq"]
                .as_u64()
                .map(|sequence| json!(sequence + 1))
                .unwrap();
        stale.records[response_index]["message"]["result"]["instance_id"] =
            json!("SECRET_INSTANCE_INIT");
        for record in stale.records.iter_mut().skip(response_index + 1) {
            if record["source_instance_id"] == "SECRET_INSTANCE_R1"
                && matches!(
                    record["record_type"].as_str(),
                    Some("probe_event" | "probe_response" | "probe_error")
                )
                && record["source_seq"]
                    .as_u64()
                    .is_some_and(|seq| seq > removed_seq)
            {
                record["source_seq"] = json!(record["source_seq"].as_u64().unwrap() - 1);
            }
        }
        for entry in stale.records.last_mut().unwrap()["incoming"]["sources"]
            .as_array_mut()
            .unwrap()
        {
            if entry["source_instance_id"] == "SECRET_INSTANCE_INIT" {
                entry["last_source_seq"] = json!(entry["last_source_seq"].as_u64().unwrap() + 1);
            } else if entry["source_instance_id"] == "SECRET_INSTANCE_R1" {
                entry["last_source_seq"] = json!(entry["last_source_seq"].as_u64().unwrap() - 1);
            }
        }
        assert_eq!(
            audit_artifact(&stale).unwrap_err().code,
            "PROBE_SOURCE_LIFECYCLE_INVALID"
        );

        let mut callback = valid_artifact(Profile::C15Combined);
        callback
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
            })
            .unwrap()["message"]["data"]["items"][0]["callback_source"] = json!("mute_binding");
        assert_eq!(
            audit_artifact(&callback).unwrap_err().code,
            "BANK_FEEDBACK_PRIVACY_INVALID"
        );
    }

    #[test]
    fn activation_feedback_is_bounded_by_initial_snapshot_completion() {
        let mut artifact = valid_artifact(Profile::C15Combined);
        let feedback_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "INIT"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
                    && record["message"]["data"]["items"][0]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap();
        let mut feedback = artifact.records.remove(feedback_index);
        let ready_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "INIT"
                    && record["message"]["event"] == "probe.ready"
                    && record["message"]["data"]["ready"] == true
            })
            .unwrap();
        let ready_at = artifact.records[ready_index]["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let begin_at = artifact
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == "INIT"
                    && record["phase"] == "begin"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let moved_at = ready_at - 1;
        feedback["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + moved_at);
        feedback["monotonic_timestamp_ms"] = json!(moved_at);
        feedback["received_at_unix_ms"] = json!(TEST_UNIX_BASE + moved_at);
        feedback["received_at_monotonic_timestamp_ms"] = json!(moved_at);
        feedback["midi_timestamp"] = json!(moved_at);
        feedback["checkpoint_elapsed_ms"] = json!(moved_at - begin_at);
        artifact.records.insert(ready_index, feedback);

        let mut last_source_seq = 0_u64;
        for record in &mut artifact.records {
            if record["source_instance_id"] == "SECRET_INSTANCE_INIT"
                && matches!(
                    record["record_type"].as_str(),
                    Some("probe_event" | "probe_response" | "probe_error")
                )
            {
                last_source_seq += 1;
                record["source_seq"] = json!(last_source_seq);
            }
        }
        let source_summary = artifact.records.last_mut().unwrap()["incoming"]["sources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["source_instance_id"] == "SECRET_INSTANCE_INIT")
            .unwrap();
        source_summary["last_source_seq"] = json!(last_source_seq);

        assert_eq!(
            audit_artifact(&artifact).unwrap_err().code,
            "PROBE_SOURCE_LIFECYCLE_INVALID"
        );
    }

    #[test]
    fn every_command_snapshot_is_reverse_correlated_to_one_send() {
        let mut artifact = valid_artifact(Profile::C15Combined);
        let checkpoint_id = "E0";
        let command_index = artifact
            .records
            .iter()
            .position(|record| {
                record["record_type"] == "probe_command"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["request"]["message"]["method"] == "probe.direct_access.snapshot"
            })
            .unwrap();
        let mut extra = artifact
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["message"]["data"]["stream"] == "direct_access_snapshot"
                    && record["message"]["data"]["reason"] == "command_snapshot"
            })
            .unwrap()
            .clone();
        let source = extra["source_instance_id"].as_str().unwrap().to_owned();
        let prior_sequence = artifact.records[..command_index]
            .iter()
            .rev()
            .find(|record| {
                record["source_instance_id"].as_str() == Some(&source)
                    && matches!(
                        record["record_type"].as_str(),
                        Some("probe_event" | "probe_response" | "probe_error")
                    )
            })
            .unwrap()["source_seq"]
            .as_u64()
            .unwrap();
        for record in artifact.records.iter_mut().skip(command_index) {
            if record["source_instance_id"].as_str() == Some(&source)
                && matches!(
                    record["record_type"].as_str(),
                    Some("probe_event" | "probe_response" | "probe_error")
                )
            {
                record["source_seq"] = json!(record["source_seq"].as_u64().unwrap() + 1);
            }
        }
        let begin_at = artifact
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "begin"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let action_at = artifact
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_action"
                    && record["checkpoint_id"] == checkpoint_id
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        let extra_at = action_at + 1_000;
        extra["timestamp_unix_ms"] = json!(TEST_UNIX_BASE + extra_at);
        extra["monotonic_timestamp_ms"] = json!(extra_at);
        extra["received_at_unix_ms"] = json!(TEST_UNIX_BASE + extra_at);
        extra["received_at_monotonic_timestamp_ms"] = json!(extra_at);
        extra["midi_timestamp"] = json!(extra_at);
        extra["checkpoint_elapsed_ms"] = json!(extra_at - begin_at);
        extra["checkpoint_window_expired"] = json!(false);
        extra["source_seq"] = json!(prior_sequence + 1);
        extra["message"]["data"]["snapshot_id"] = json!("SECRET_EARLY_COMMAND_SNAPSHOT");
        artifact.records.insert(command_index, extra);

        let checkpoint_end = artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "collector_checkpoint"
                    && record["checkpoint_id"] == checkpoint_id
                    && record["phase"] == "end"
            })
            .unwrap();
        checkpoint_end["messages_processed_before_end_marker"] = json!(
            checkpoint_end["messages_processed_before_end_marker"]
                .as_u64()
                .unwrap()
                + 1
        );
        let summary = artifact.records.last_mut().unwrap();
        summary["protocol_tracking"]["completed_chunk_streams"] = json!(
            summary["protocol_tracking"]["completed_chunk_streams"]
                .as_u64()
                .unwrap()
                + 1
        );
        summary["protocol_tracking"]["completed_snapshot_streams"] = json!(
            summary["protocol_tracking"]["completed_snapshot_streams"]
                .as_u64()
                .unwrap()
                + 1
        );
        summary["protocol_tracking"]["checkpoint_messages"] = json!(
            summary["protocol_tracking"]["checkpoint_messages"]
                .as_u64()
                .unwrap()
                + 1
        );
        for field in ["frames", "messages", "events"] {
            summary["incoming"][field] = json!(summary["incoming"][field].as_u64().unwrap() + 1);
        }
        let source_summary = summary["incoming"]["sources"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["source_instance_id"] == source)
            .unwrap();
        source_summary["last_source_seq"] =
            json!(source_summary["last_source_seq"].as_u64().unwrap() + 1);

        assert_eq!(
            audit_artifact(&artifact).unwrap_err().code,
            "SNAPSHOT_COMMAND_COVERAGE_INVALID"
        );
    }

    #[test]
    fn feedback_received_at_the_auto_marker_is_semantically_stale() {
        let mut artifact = valid_artifact(Profile::C15Combined);
        let action_at = artifact
            .records
            .iter()
            .find(|record| {
                record["record_type"] == "collector_action" && record["checkpoint_id"] == "C1"
            })
            .unwrap()["monotonic_timestamp_ms"]
            .as_u64()
            .unwrap();
        artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
                    && record["message"]["data"]["items"][0]["config_id"] == "MB_CORE_ALL"
            })
            .unwrap()["received_at_monotonic_timestamp_ms"] = json!(action_at);
        let report = audit_artifact(&artifact).unwrap();
        let slot = report
            .projections
            .iter()
            .find_map(|projection| match projection {
                SemanticProjection::MixerBank {
                    checkpoint_id: "C1",
                    config_id,
                    slots,
                    ..
                } if config_id == "MB_CORE_ALL" => slots.first(),
                _ => None,
            })
            .unwrap();
        assert_eq!(slot.title_freshness, FieldFreshness::StaleOrUnobserved);
        assert!(slot.title.is_none());
        assert_eq!(slot.host_id_status, AliasStatus::Unavailable);
    }

    #[test]
    fn partial_same_script_reactivation_fails_closed() {
        let mut artifact = valid_artifact(Profile::C13MixerBank);
        let feedback = artifact
            .records
            .iter_mut()
            .find(|record| {
                record["record_type"] == "probe_event"
                    && record["checkpoint_id"] == "C1"
                    && record["message"]["data"]["stream"] == "mixer_bank_feedback"
            })
            .unwrap();
        feedback["message"]["event"] = json!("probe.mapping_active");
        feedback["message"]["data"] = json!({
            "probe_session_id": feedback["source_instance_id"].clone(),
            "mapping_active": true,
            "read_only": true,
            "protocol_version": 1
        });
        assert_eq!(
            audit_artifact(&artifact).unwrap_err().code,
            "PROBE_SOURCE_LIFECYCLE_INVALID"
        );

        let complete = valid_artifact_with_options(Profile::C13MixerBank, false, Some("S0"));
        let report = audit_artifact(&complete).unwrap();
        assert_eq!(report.checkpoint_count, REQUIRED_CHECKPOINTS.len());

        let navigation = valid_artifact_with_options(
            Profile::C13MixerBank,
            false,
            Some("E8-MB_CORE_ALL-B0-reset"),
        );
        let report = audit_artifact(&navigation).unwrap();
        assert_eq!(report.checkpoint_count, REQUIRED_CHECKPOINTS.len());
    }
}
