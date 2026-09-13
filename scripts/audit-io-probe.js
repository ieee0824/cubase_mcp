'use strict'

// Structural validation only: this cannot establish the contents or completeness
// of a user's bus inventory, installed script provenance, or host version.
const fs = require('node:fs')
const PROFILE = 'io-existing-v1'
const CONFIGS = ['IO_INPUT_ALL', 'IO_OUTPUT_ALL']
const FILES = ['CubaseMCPIOProbe_CubaseMCPIOProbe.js', 'io-profile.js', 'wire.js']
const MAX_RECORDS = 20000
const MAX_RAW_BYTES = 16 * 1024 * 1024
const ITEM_KEYS = ['slot_index', 'title_observed', 'title_state', 'title_alias',
    'host_id_supported', 'host_id_status', 'host_id_alias']
const TITLE_KEYS = ['title_observed', 'title_state', 'title_alias']
const REASONS = { 'probe.bank.snapshot': 'command_snapshot', 'probe.bank.reset': 'command_reset',
    'probe.bank.next': 'command_next', 'probe.bank.prev': 'command_prev' }

function check(value, code = 'INVALID_CAPTURE') {
    if (!value) { const error = new Error(code); error.code = code; throw error }
}
function object(value) {
    check(value !== null && typeof value === 'object' && !Array.isArray(value))
    return value
}
function keys(value, required, optional = []) {
    object(value)
    check(required.every(key => Object.hasOwn(value, key)) &&
        Object.keys(value).every(key => required.includes(key) || optional.includes(key)), 'INVALID_FIELDS')
}
function integer(value, min = 0, max = Number.MAX_SAFE_INTEGER) {
    check(Number.isSafeInteger(value) && value >= min && value <= max)
}
function identifier(value) { check(typeof value === 'string' && value.length > 0 && value.length <= 256) }
function boolean(value) { check(typeof value === 'boolean') }
function profile(value) { check(value === PROFILE, 'WRONG_PROFILE') }
function config(value) { check(CONFIGS.includes(value), 'INVALID_CONFIG') }
function sha(value) { check(typeof value === 'string' && /^[a-f0-9]{64}$/.test(value), 'INVALID_DIGEST') }
function zeroes(value, names) { object(value); names.forEach(name => check(value[name] === 0, 'INCOMPLETE_CAPTURE')) }
function oneSource(values, source) { check(Array.isArray(values) && values.length === 1 && values[0] === source) }

function validateManifest(manifest) {
    keys(manifest, ['version', 'profile', 'expected_collector_sha256', 'host', 'expected_probe_files', 'ui_review'])
    check(manifest.version === 1); profile(manifest.profile); sha(manifest.expected_collector_sha256)
    check(manifest.ui_review === 'pending', 'UI_REVIEW_REQUIRED')
    keys(manifest.host, ['cubase_version', 'api_version'])
    check((manifest.host.cubase_version === '13.0.30' && manifest.host.api_version === '1.1') ||
        (manifest.host.cubase_version === '15.0.30' && manifest.host.api_version === '1.3'), 'UNSUPPORTED_HOST')
    keys(manifest.expected_probe_files, FILES)
    FILES.forEach(file => sha(manifest.expected_probe_files[file]))
}

function validateItem(item, configId, withConfig) {
    keys(item, withConfig ? ITEM_KEYS.concat('config_id') : ITEM_KEYS)
    if (withConfig) check(item.config_id === configId)
    integer(item.slot_index, 0, 7); boolean(item.title_observed); boolean(item.host_id_supported)
    check(['unobserved', 'empty', 'nonempty'].includes(item.title_state))
    check(item.title_observed === (item.title_state !== 'unobserved'))
    if (item.title_state === 'nonempty') {
        check(typeof item.title_alias === 'string' && /^title-([1-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-6])$/.test(item.title_alias), 'INVALID_ALIAS')
    } else check(item.title_alias === null, 'INVALID_ALIAS')
    check(['unobserved', 'unsupported', 'supported', 'empty'].includes(item.host_id_status), 'HOST_ID_ERROR')
    check(item.host_id_supported ? item.host_id_status !== 'unsupported' : item.host_id_status === 'unsupported')
    if (item.host_id_status === 'supported') {
        check(typeof item.host_id_alias === 'string' && /^host-([1-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-6])$/.test(item.host_id_alias), 'INVALID_ALIAS')
    } else check(item.host_id_alias === null, 'INVALID_ALIAS')
    // Copy only validated metadata, never an input object, string ID, or title.
    return Object.fromEntries(ITEM_KEYS.map(key => [key, item[key]]))
}

function validateCapabilities(data) {
    keys(data, ['profile', 'active', 'activation_epoch', 'fatal_error', 'slot_count', 'metadata_only',
        'complete', 'limits', 'configs', 'bus_mutation', 'integrity_failed'])
    profile(data.profile); check(data.active === true && data.activation_epoch === 1 && data.fatal_error === null)
    check(data.slot_count === 8 && data.metadata_only === true && data.complete === false &&
        data.bus_mutation === false && data.integrity_failed === false)
    keys(data.limits, ['aliases', 'raw_string_length', 'pending_feedback'])
    check(data.limits.aliases === 256 && data.limits.raw_string_length === 4096 && data.limits.pending_feedback === 128)
    check(Array.isArray(data.configs) && data.configs.length === 2)
    data.configs.forEach((entry, index) => {
        keys(entry, ['config_id', 'requested_channel_kind', 'follow_visibility', 'window_zones',
            'explicit_main_filter', 'host_id_supported', 'actions'])
        check(entry.config_id === CONFIGS[index] && entry.requested_channel_kind === ['input', 'output'][index])
        check(entry.follow_visibility === false && entry.window_zones === 'all_requested'); boolean(entry.explicit_main_filter)
        check(Array.isArray(entry.host_id_supported) && entry.host_id_supported.length === 8)
        entry.host_id_supported.forEach(boolean)
        keys(entry.actions, ['reset', 'next', 'prev']); Object.values(entry.actions).forEach(boolean)
    })
}

// Collector stdin and receive threads share neither an emission order nor a
// sink lock across MIDI send. @selected writes its command/send pair only after
// releasing the tracker lock, so a valid reply can precede BOTH records. Index
// complete evidence without reordering any source record or mutating the input.
function indexRequests(records) {
    const requests = new Map(); const sends = new Map()
    records.forEach((record, index) => {
        object(record)
        if (record.record_type !== 'probe_command' && record.record_type !== 'probe_command_send_result') return
        identifier(record.request_id)
        const target = record.record_type === 'probe_command' ? requests : sends
        check(!target.has(record.request_id), 'DUPLICATE_REQUEST_EVIDENCE')
        target.set(record.request_id, { record, index })
    })
    check(requests.size === sends.size, 'INCOMPLETE_REQUEST_EVIDENCE')
    requests.forEach((entry, id) => {
        const sent = sends.get(id)
        check(sent && sent.record.sent === true && sent.record.checkpoint_id === entry.record.checkpoint_id, 'INVALID_SEND_EVIDENCE')
        check(sent.index > entry.index, 'INVALID_SEND_EVIDENCE')
        integer(sent.record.sysex_bytes, 1)
        integer(sent.record.send_completed_monotonic_timestamp_ms)
        check(sent.record.send_completed_monotonic_timestamp_ms <= sent.record.monotonic_timestamp_ms)
        const delayed = entry.record.evidence_emission === 'after_midi_send_attempt'
        check(entry.record.evidence_emission === undefined || delayed)
        check(sent.record.evidence_emission === entry.record.evidence_emission)
        if (delayed) {
            check(sent.index === entry.index + 1 && entry.record.monotonic_timestamp_ms >= sent.record.send_completed_monotonic_timestamp_ms)
        } else check(entry.record.monotonic_timestamp_ms <= sent.record.send_completed_monotonic_timestamp_ms)
        entry.sent = sent.record
    })
    return requests
}

function audit(records, manifest) {
    validateManifest(manifest)
    check(Array.isArray(records) && records.length >= 2 && records.length <= MAX_RECORDS, 'CAPTURE_BOUNDS')
    const first = object(records[0]); const last = object(records[records.length - 1])
    check(first.record_type === 'collector_started' && last.record_type === 'collector_summary', 'INCOMPLETE_CAPTURE')
    profile(first.probe_profile); check(first.probe_transport_version === 1)
    check(first.collector_binary_sha256 === manifest.expected_collector_sha256, 'COLLECTOR_DIGEST_MISMATCH')
    identifier(first.run_id); identifier(first.session_id)
    const requestEvidence = indexRequests(records); const requestOrder = [...requestEvidence.values()]
    const unrecordedSends = new Set(); let nextRequest = 0
    const seenRequests = new Set(); const seenSnapshots = new Set(); const checkpoints = new Set()
    const generations = { IO_INPUT_ALL: 1, IO_OUTPUT_ALL: 1 }
    const titles = { IO_INPUT_ALL: Array(8).fill(null), IO_OUTPUT_ALL: Array(8).fill(null) }
    const snapshots = []; let source = null; let sourceSeq = 0; let lifecycle = 0
    let active = null; let pending = null; let openSnapshot = null; let selected = false
    let monotonic = -1; let feedbackCount = 0; let observation = 0; let events = 0; let responses = 0
    let localCommands = 0; let drain = 0; let checkpointMessages = 0; let startupCapabilities = null
    function member(record) { check(active && record.checkpoint_id === active.id && !drain, 'CHECKPOINT_MISMATCH') }
    function finishPending() { pending = null }
    function startRequest(entry) {
        check(entry && entry === requestOrder[nextRequest], 'REQUEST_ORDER')
        const record = entry.record
        member(record); check(lifecycle === 3 && !pending && !openSnapshot && unrecordedSends.size === 0 && record.phase === 'started')
        identifier(record.request_id); check(!seenRequests.has(record.request_id), 'DUPLICATE_REQUEST')
        const request = object(record.request); keys(request, ['probe_transport_version', 'target_instance_id', 'message'])
        check(request.probe_transport_version === 1)
        const message = request.message; keys(message, ['version', 'id', 'type', 'method', 'params'])
        check(message.version === 1 && message.type === 'request' && message.id === record.request_id)
        check(Object.hasOwn(REASONS, message.method) || ['probe.discover', 'probe.capabilities.get'].includes(message.method), 'METHOD_NOT_ALLOWED')
        if (message.method === 'probe.discover') {
            keys(message.params, []); check(request.target_instance_id === null && record.evidence_emission === undefined)
        } else {
            check(selected && request.target_instance_id === source)
            if (Object.hasOwn(REASONS, message.method)) {
                keys(message.params, ['config_id']); config(message.params.config_id)
            } else keys(message.params, [])
        }
        check(entry.sent.send_completed_monotonic_timestamp_ms >= active.start)
        seenRequests.add(message.id); unrecordedSends.add(message.id); nextRequest++
        pending = { id: message.id, method: message.method, config: message.params.config_id, replied: false }
    }

    for (let index = 0; index < records.length; ++index) {
        const record = object(records[index]); const type = record.record_type
        check(record.record_format_version === 1 && record.run_id === first.run_id)
        integer(record.timestamp_unix_ms, 1); integer(record.monotonic_timestamp_ms)
        check(record.monotonic_timestamp_ms >= monotonic, 'RECORD_REORDER'); monotonic = record.monotonic_timestamp_ms
        if (type === 'collector_started') { check(index === 0); continue }
        if (type === 'collector_summary') { check(index === records.length - 1); continue }
        if (type === 'collector_checkpoint') {
            identifier(record.checkpoint_id); localCommands++
            if (record.phase === 'begin') {
                check(!active && !pending && !drain && !checkpoints.has(record.checkpoint_id))
                integer(record.window_ms, 1, 600000)
                active = { id: record.checkpoint_id, window: record.window_ms, start: monotonic,
                    startLow: 0, startHigh: monotonic, lastReceive: null, messages: 0, action: false }
            } else {
                member(record); check(record.phase === 'end' && !pending && !openSnapshot && unrecordedSends.size === 0)
                check(record.window_ms === active.window && record.window_satisfied === true && record.quiet_period_satisfied === true)
                integer(record.observed_duration_ms, active.window)
                check(record.quiet_period_required_ms === 1000)
                integer(record.quiet_period_observed_ms, 1000)
                // Tracker boundaries precede sink emission. Correlate independent
                // elapsed/quiet durations and receive timestamps, not sink delay.
                let endLow = active.startLow + record.observed_duration_ms
                let endHigh = Math.min(active.startHigh + record.observed_duration_ms + 1, monotonic)
                if (active.lastReceive !== null) {
                    endLow = Math.max(endLow, active.lastReceive + record.quiet_period_observed_ms)
                    endHigh = Math.min(endHigh, active.lastReceive + record.quiet_period_observed_ms + 1)
                } else check(record.quiet_period_observed_ms === record.observed_duration_ms)
                check(endLow <= endHigh, 'CHECKPOINT_TIMING')
                check(record.messages_processed_before_end_marker === active.messages)
                checkpoints.add(active.id); active = null
            }
            continue
        }
        if (type === 'collector_action') {
            member(record); check(record.phase === 'marked' && !active.action && !pending)
            active.action = true; localCommands++; continue
        }
        if (type === 'probe_command') {
            member(record)
            if (!seenRequests.has(record.request_id)) startRequest(requestEvidence.get(record.request_id))
            continue
        }
        if (type === 'probe_command_send_result') {
            member(record); check(unrecordedSends.delete(record.request_id), 'UNMATCHED_SEND')
            continue
        }
        if (type === 'collector_discovery_completed') {
            member(record); check(pending && pending.method === 'probe.discover' && pending.replied && pending.id === record.request_id)
            check(record.responder_count === 1 && record.window_closed === true && record.outcome === 'selected')
            oneSource(record.source_instance_ids, source); oneSource(record.observed_source_instance_ids, source)
            check(record.selected_source_instance_id === source); selected = true; finishPending(); continue
        }
        if (type === 'collector_drain_started') {
            check(!active && !pending && !openSnapshot && unrecordedSends.size === 0 && drain === 0); drain = 1; continue
        }
        if (type === 'collector_drain_completed') {
            check(drain === 1 && record.completed === true && record.timed_out === false); drain = 2; continue
        }
        check(type === 'probe_event' || type === 'probe_response', 'FORBIDDEN_RECORD')
        member(record)
        check(record.integrity_ok_at_emit === true && record.probe_transport_version === 1)
        check(record.orphan === false && record.processed_after_checkpoint_end === false && record.checkpoint_quiet_period_violated === false)
        integer(record.checkpoint_elapsed_ms); check(record.checkpoint_window_ms === active.window)
        check(record.checkpoint_window_expired === (record.checkpoint_elapsed_ms >= active.window))
        integer(record.received_at_monotonic_timestamp_ms, active.lastReceive ?? 0, monotonic)
        // floor(R) - floor(R - B) is either floor(B) or floor(B) + 1.
        // Intersect all observations to retain a common possible begin boundary.
        const beginUpper = record.received_at_monotonic_timestamp_ms - record.checkpoint_elapsed_ms
        active.startLow = Math.max(active.startLow, beginUpper - 1)
        active.startHigh = Math.min(active.startHigh, beginUpper)
        check(active.startLow <= active.startHigh, 'CHECKPOINT_TIMING')
        active.lastReceive = record.received_at_monotonic_timestamp_ms
        if (source === null) { identifier(record.source_instance_id); source = record.source_instance_id }
        check(record.source_instance_id === source && record.source_seq === ++sourceSeq, 'SOURCE_SEQUENCE')
        active.messages++; checkpointMessages++
        const message = object(record.message)
        check(message.version === 1)
        if (type === 'probe_response') {
            responses++; keys(message, ['version', 'id', 'type', 'result'])
            if (!pending) {
                const entry = requestEvidence.get(message.id)
                check(entry && entry.index > index && entry.record.evidence_emission === 'after_midi_send_attempt', 'UNMATCHED_RESPONSE')
                startRequest(entry)
            }
            check(message.type === 'response' && lifecycle === 3 && !pending.replied && message.id === pending.id)
            check(requestEvidence.get(message.id).sent.send_completed_monotonic_timestamp_ms <= monotonic, 'RESPONSE_BEFORE_SEND')
            const result = object(message.result); profile(result.profile)
            if (pending.method === 'probe.discover') {
                keys(result, ['profile', 'instance_id', 'ready', 'read_only'])
                check(result.instance_id === source && result.ready === true && result.read_only === true)
            } else if (pending.method === 'probe.capabilities.get') validateCapabilities(result)
            else {
                const action = pending.method.slice('probe.bank.'.length)
                keys(result, action === 'snapshot' ? ['profile', 'config_id', 'scheduled'] : ['profile', 'config_id', 'scheduled', 'action'])
                check(result.config_id === pending.config && result.scheduled === true)
                if (action !== 'snapshot') {
                    check(result.action === action)
                    // The driver flushes queued old-generation feedback before
                    // navigating. Only a validated success marks the boundary;
                    // command/send evidence may be emitted after the response.
                    generations[pending.config]++
                    titles[pending.config].fill(null)
                }
            }
            pending.replied = true
            if (pending.method === 'probe.capabilities.get') finishPending()
            continue
        }
        events++; keys(message, ['version', 'type', 'event', 'data']); check(message.type === 'event')
        const data = object(message.data); profile(data.profile)
        if (['probe.loaded', 'probe.mapping_active', 'probe.ready'].includes(message.event)) {
            check(message.event === ['probe.loaded', 'probe.mapping_active', 'probe.ready'][lifecycle], 'LIFECYCLE')
            const fields = ['profile', 'probe_session_id', 'mapping_active', 'read_only', 'protocol_version']
            if (lifecycle === 2) fields.push('activation_epoch', 'ready', 'initial_snapshots_complete')
            keys(data, fields)
            check(data.probe_session_id === source && data.read_only === true && data.protocol_version === 1)
            check(data.mapping_active === true)
            if (lifecycle === 2) check(data.activation_epoch === 1 && data.ready === true && data.initial_snapshots_complete === true && startupCapabilities)
            lifecycle++; continue
        }
        check(lifecycle >= 2)
        if (message.event === 'probe.capabilities') {
            check(lifecycle === 2 && startupCapabilities === null)
            validateCapabilities(data); startupCapabilities = data; continue
        }
        if (message.event === 'probe.io.feedback') {
            keys(data, ['profile', 'config_id', 'activation_epoch', 'generation', 'observation_id', 'item'])
            config(data.config_id); check(data.activation_epoch === 1 && data.generation === generations[data.config_id])
            integer(data.observation_id, 1); check(data.observation_id === observation + 1); observation = data.observation_id
            const item = validateItem(data.item, data.config_id, false)
            check(startupCapabilities && data.item.host_id_supported === startupCapabilities.configs[CONFIGS.indexOf(data.config_id)].host_id_supported[data.item.slot_index])
            // A title callback observes a string (possibly empty), but does not
            // call the host-ID getter. IDs are sampled only for snapshots.
            check(item.title_observed && item.host_id_status === (item.host_id_supported ? 'unobserved' : 'unsupported'))
            titles[data.config_id][item.slot_index] = item
            feedbackCount++; continue
        }
        check(message.event === 'probe.bank.chunk', 'FORBIDDEN_EVENT')
        keys(data, ['profile', 'activation_epoch', 'generation', 'observation_id', 'complete', 'metadata_only',
            'snapshot_id', 'stream', 'config_id', 'reason', 'chunk_index', 'chunk_count', 'total_items',
            'snapshot_complete', 'truncated', 'overflow_safe', 'items'])
        check(pending && pending.replied && data.config_id === pending.config && data.reason === REASONS[pending.method])
        check(data.activation_epoch === 1 && data.generation === generations[data.config_id] && data.observation_id === observation)
        check(data.complete === false && data.metadata_only === true && data.stream === 'mixer_bank_snapshot')
        check(data.chunk_count === 4 && data.total_items === 8 && data.truncated === false && data.overflow_safe === true)
        identifier(data.snapshot_id); integer(data.chunk_index, 0, 3)
        check(data.snapshot_complete === (data.chunk_index === 3))
        check(Array.isArray(data.items) && data.items.length === 2)
        if (!openSnapshot) {
            check(data.chunk_index === 0 && !seenSnapshots.has(data.snapshot_id), 'SNAPSHOT_SEQUENCE')
            openSnapshot = { id: data.snapshot_id, config_id: data.config_id, reason: data.reason,
                generation: data.generation, observation_id: data.observation_id, items: [] }
        }
        check(openSnapshot.id === data.snapshot_id && openSnapshot.observation_id === data.observation_id &&
            openSnapshot.items.length === data.chunk_index * 2, 'SNAPSHOT_SEQUENCE')
        data.items.forEach(item => {
            const projected = validateItem(item, data.config_id, true)
            check(item.host_id_supported === startupCapabilities.configs[CONFIGS.indexOf(data.config_id)].host_id_supported[item.slot_index])
            const title = titles[data.config_id][projected.slot_index]
            check(title ? TITLE_KEYS.every(key => projected[key] === title[key]) :
                projected.title_observed === false && projected.title_state === 'unobserved' && projected.title_alias === null,
                'TITLE_FEEDBACK_MISMATCH')
            check(projected.slot_index === openSnapshot.items.length, 'SLOT_SEQUENCE'); openSnapshot.items.push(projected)
        })
        if (data.snapshot_complete) {
            seenSnapshots.add(openSnapshot.id)
            snapshots.push({ config_id: openSnapshot.config_id, reason: openSnapshot.reason, generation: openSnapshot.generation,
                observation_id: openSnapshot.observation_id, items: openSnapshot.items })
            openSnapshot = null; finishPending()
        }
    }
    check(lifecycle === 3 && selected && !active && !pending && !openSnapshot && drain === 2, 'INCOMPLETE_CAPTURE')
    check(CONFIGS.every(id => snapshots.some(snapshot => snapshot.config_id === id)), 'MISSING_IO_CONFIG')
    check(last.session_id === first.session_id && last.exit_ok === true && last.integrity_ok === true && last.exit_reason === 'stdin_eof')
    check(last.graceful_drain?.completed === true && last.graceful_drain.timed_out === false)
    zeroes(last, ['orphan_messages']); zeroes(last.commands, ['rejected', 'deferred'])
    check(last.commands.sent === seenRequests.size && last.commands.local === localCommands && last.commands.received === seenRequests.size + localCommands)
    zeroes(last.incoming, ['errors', 'diagnostics', 'parse_errors', 'oversize_frames', 'source_overflows', 'queue_drops', 'sequence_gaps', 'sequence_duplicates_or_reorders'])
    check(last.incoming.messages === sourceSeq && last.incoming.frames === sourceSeq && last.incoming.events === events && last.incoming.responses === responses)
    check(Array.isArray(last.incoming.sources) && last.incoming.sources.length === 1)
    check(last.incoming.sources[0].source_instance_id === source && last.incoming.sources[0].last_source_seq === sourceSeq)
    const tracking = last.protocol_tracking
    zeroes(tracking, ['orphan_messages', 'pending_requests', 'expected_followups', 'open_snapshots', 'checkpoint_messages_processed_after_end', 'completed_feedback_streams'])
    check(tracking.completed_requests === seenRequests.size && tracking.completed_snapshot_streams === snapshots.length && tracking.completed_chunk_streams === snapshots.length)
    check(tracking.completed_checkpoints === checkpoints.size && tracking.checkpoint_messages === checkpointMessages && tracking.selected_source_instance_id === source)
    oneSource(tracking.active_source_instance_ids, source)
    return { version: 1, profile: PROFILE, status: 'structurally_valid', runtime_acceptance: 'pending_ui_review', complete: false,
        counts: { records: records.length, sources: 1, activations: 1, checkpoints: checkpoints.size, requests: seenRequests.size,
            snapshots: snapshots.length, feedback: feedbackCount }, snapshots,
        limitations: ['Not proof of a full input/output bus inventory or actual channel type.',
            'Empty slots and repeated pages do not establish enumeration end; title aliases are not identities.',
            'Host versions and expected probe-file digests are declarations requiring independent provenance review.',
            'Independent UI review and runtime evidence are required; this is not Issue completion.'] }
}

function readBounded(file, maximum) {
    const descriptor = fs.openSync(file, 'r')
    try {
        check(fs.fstatSync(descriptor).isFile(), 'INPUT_BOUNDS')
        const buffer = Buffer.alloc(maximum + 1); let size = 0
        while (size < buffer.length) { const read = fs.readSync(descriptor, buffer, size, buffer.length - size, null); if (!read) break; size += read }
        check(size <= maximum, 'INPUT_BOUNDS')
        return new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, size))
    } finally { fs.closeSync(descriptor) }
}

if (require.main === module) {
    try {
        check(process.argv.length === 4, 'USAGE')
        const raw = readBounded(process.argv[2], MAX_RAW_BYTES)
        check(raw.endsWith('\n'), 'TRUNCATED_JSONL')
        const lines = raw.slice(0, -1).split('\n'); check(lines.length <= MAX_RECORDS, 'CAPTURE_BOUNDS')
        const report = audit(lines.map(line => JSON.parse(line)), JSON.parse(readBounded(process.argv[3], 64 * 1024)))
        process.stdout.write(JSON.stringify(report) + '\n')
    } catch (_) {
        // Native JSON/filesystem errors can contain paths or secret input text.
        process.stderr.write('IO_PROBE_AUDIT_REJECTED\n'); process.exitCode = 1
    }
}

module.exports = { audit }
