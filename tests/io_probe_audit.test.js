'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')
const { test } = require('node:test')
const { audit } = require('../scripts/audit-io-probe')
const PROFILE = 'io-existing-v1'
const CONFIGS = ['IO_INPUT_ALL', 'IO_OUTPUT_ALL']
const SHA = 'a'.repeat(64)
const CANARY = 'PRIVATE_TITLE_ID_ERROR_PATH_DO_NOT_EMIT'

function fixture({ source = 'synthetic-source', hostIds = false, feedback = false, navigation = false, capabilities = false } = {}) {
    const manifest = { version: 1, profile: PROFILE, expected_collector_sha256: SHA,
        host: { cubase_version: '13.0.30', api_version: '1.1' }, ui_review: 'pending',
        expected_probe_files: { 'CubaseMCPIOProbe_CubaseMCPIOProbe.js': SHA, 'io-profile.js': SHA, 'wire.js': SHA } }
    const records = []; let clock = 0; let sequence = 0; let events = 0; let responses = 0
    let requests = 0; let snapshots = 0; let observation = 0; let lastReceive = 10
    const generations = { IO_INPUT_ALL: 1, IO_OUTPUT_ALL: 1 }
    const add = record => records.push({ record_format_version: 1, run_id: 'synthetic-run',
        timestamp_unix_ms: 100000 + clock, monotonic_timestamp_ms: clock, ...record })
    const emit = message => {
        clock += 10; lastReceive = clock
        const type = message.type === 'event' ? 'probe_event' : 'probe_response'
        if (type === 'probe_event') events++; else responses++
        add({ record_type: type, received_at_unix_ms: 100000 + clock,
            received_at_monotonic_timestamp_ms: clock, midi_timestamp: 0, integrity_ok_at_emit: true,
            probe_transport_version: 1, source_instance_id: source, source_seq: ++sequence,
            checkpoint_id: 'IO', orphan: false, checkpoint_elapsed_ms: clock - 10, checkpoint_window_ms: 1000,
            checkpoint_window_expired: clock - 10 >= 1000, processed_after_checkpoint_end: false,
            checkpoint_quiet_period_violated: false, message: { version: 1, ...message } })
    }
    const event = (name, data) => emit({ type: 'event', event: name, data: { profile: PROFILE, ...data } })
    const response = (id, result) => emit({ type: 'response', id, result: { profile: PROFILE, ...result } })
    const request = (method, params = {}) => {
        const id = 'request-' + (++requests); clock += 10
        add({ record_type: 'probe_command', phase: 'started', request_id: id, checkpoint_id: 'IO',
            request: { probe_transport_version: 1, target_instance_id: method === 'probe.discover' ? null : source,
                message: { version: 1, id, type: 'request', method, params } } })
        clock += 10
        add({ record_type: 'probe_command_send_result', request_id: id, checkpoint_id: 'IO', sent: true,
            sysex_bytes: 256, send_completed_monotonic_timestamp_ms: clock })
        return id
    }
    const item = (index, withConfig, configId) => ({ ...(withConfig ? { config_id: configId } : {}), slot_index: index,
        title_observed: feedback && index === 0, title_state: feedback && index === 0 ? 'nonempty' : 'unobserved',
        title_alias: feedback && index === 0 ? 'title-1' : null,
        host_id_supported: hostIds, host_id_status: hostIds ? 'supported' : 'unsupported',
        host_id_alias: hostIds ? 'host-' + (index + 1) : null })
    add({ record_type: 'collector_started', session_id: 'synthetic-session', collector_binary_sha256: SHA,
        probe_profile: PROFILE, probe_transport_version: 1, resolved_midi_input_port: CANARY })
    clock = 10; add({ record_type: 'collector_checkpoint', phase: 'begin', checkpoint_id: 'IO', window_ms: 1000 })
    const lifecycle = { probe_session_id: source, mapping_active: true, read_only: true, protocol_version: 1 }
    event('probe.loaded', lifecycle); event('probe.mapping_active', lifecycle)
    event('probe.capabilities', { active: true, activation_epoch: 1, fatal_error: null, slot_count: 8,
        metadata_only: true, complete: false, bus_mutation: false, integrity_failed: false,
        limits: { aliases: 256, raw_string_length: 4096, pending_feedback: 128 },
        configs: CONFIGS.map((id, index) => ({ config_id: id, requested_channel_kind: ['input', 'output'][index],
            follow_visibility: false, window_zones: 'all_requested', explicit_main_filter: false,
            host_id_supported: Array(8).fill(hostIds), actions: { reset: true, next: true, prev: true } })) })
    event('probe.ready', { ...lifecycle, activation_epoch: 1, ready: true, initial_snapshots_complete: true })
    const discover = request('probe.discover')
    response(discover, { instance_id: source, ready: true, read_only: true })
    clock += 10
    add({ record_type: 'collector_discovery_completed', request_id: discover, checkpoint_id: 'IO',
        responder_count: 1, source_instance_ids: [source], observed_source_instance_ids: [source],
        selected_source_instance_id: source, outcome: 'selected', window_closed: true })
    if (capabilities) {
        response(request('probe.capabilities.get'), records.find(record => record.message?.event === 'probe.capabilities').message.data)
    }
    CONFIGS.forEach(configId => {
        if (feedback) {
            const state = item(0, false, configId)
            if (hostIds) { state.host_id_status = 'unobserved'; state.host_id_alias = null }
            event('probe.io.feedback', { config_id: configId, activation_epoch: 1,
                generation: generations[configId], observation_id: ++observation, item: state })
        }
        const method = navigation ? 'probe.bank.next' : 'probe.bank.snapshot'
        const id = request(method, { config_id: configId })
        if (navigation) generations[configId]++
        response(id, { config_id: configId, scheduled: true, ...(navigation ? { action: 'next' } : {}) })
        const snapshotId = 'snapshot-' + (++snapshots)
        for (let chunk = 0; chunk < 4; chunk++) event('probe.bank.chunk', {
            activation_epoch: 1, generation: generations[configId], observation_id: observation,
            complete: false, metadata_only: true, snapshot_id: snapshotId, stream: 'mixer_bank_snapshot',
            config_id: configId, reason: navigation ? 'command_next' : 'command_snapshot',
            chunk_index: chunk, chunk_count: 4, total_items: 8, snapshot_complete: chunk === 3,
            truncated: false, overflow_safe: true,
            items: [item(chunk * 2, true, configId), item(chunk * 2 + 1, true, configId)]
        })
    })
    clock += 1000
    add({ record_type: 'collector_checkpoint', phase: 'end', checkpoint_id: 'IO', window_ms: 1000,
        observed_duration_ms: clock - 10, window_satisfied: true, quiet_period_required_ms: 1000,
        quiet_period_observed_ms: clock - lastReceive, quiet_period_satisfied: true,
        messages_processed_before_end_marker: sequence, late_received_frames_may_be_classified_by_receive_timestamp: true })
    clock += 10; add({ record_type: 'collector_drain_started', timeout_ms: 5000, deadline_monotonic_timestamp_ms: clock + 5000 })
    clock += 5000; add({ record_type: 'collector_drain_completed', completed: true, timed_out: false, duration_ms: 5000 })
    clock += 10
    add({ record_type: 'collector_summary', session_id: 'synthetic-session', integrity_ok: true, exit_ok: true,
        exit_reason: 'stdin_eof', commands: { received: requests + 2, sent: requests, local: 2, deferred: 0, rejected: 0 },
        graceful_drain: { completed: true, timed_out: false, duration_ms: 5000 }, orphan_messages: 0,
        protocol_tracking: { completed_requests: requests, completed_chunk_streams: snapshots,
            completed_snapshot_streams: snapshots, completed_feedback_streams: 0, completed_checkpoints: 1,
            checkpoint_messages: sequence, checkpoint_messages_processed_after_end: 0, orphan_messages: 0,
            pending_requests: 0, expected_followups: 0, open_snapshots: 0,
            selected_source_instance_id: source, active_source_instance_ids: [source] },
        incoming: { frames: sequence, messages: sequence, events, responses, errors: 0, diagnostics: 0,
            parse_errors: 0, oversize_frames: 0, source_overflows: 0, queue_drops: 0, sequence_gaps: 0,
            sequence_duplicates_or_reorders: 0, sources: [{ source_instance_id: source, last_source_seq: sequence }] } })
    return { records, manifest }
}

const chunks = records => records.filter(record => record.message?.event === 'probe.bank.chunk')
function delayEvidence(records, requestId, { selected = false, afterChunks = false } = {}) {
    const command = records.find(record => record.record_type === 'probe_command' && record.request_id === requestId)
    const sent = records.find(record => record.record_type === 'probe_command_send_result' && record.request_id === requestId)
    const reply = records.find(record => record.record_type === 'probe_response' && record.message.id === requestId)
    const anchor = afterChunks ? chunks(records).find(record =>
        record.message.data.config_id === command.request.message.params.config_id && record.message.data.chunk_index === 3) : reply
    const moved = selected ? [command, sent] : [sent]
    for (const record of moved) records.splice(records.indexOf(record), 1)
    moved.forEach((record, index) => {
        record.monotonic_timestamp_ms = anchor.monotonic_timestamp_ms + index + 1
        record.timestamp_unix_ms = 100000 + record.monotonic_timestamp_ms
        if (selected) record.evidence_emission = 'after_midi_send_attempt'
    })
    records.splice(records.indexOf(anchor) + 1, 0, ...moved)
}
function rejection(change, options) {
    const { records, manifest } = fixture(options); change(records, manifest)
    assert.throws(() => audit(records, manifest), error => {
        assert.match(error.message, /^[A-Z_]+$/)
        assert.ok(!error.message.includes(CANARY)); return true
    })
}

test('valid structural capture stays metadata-only and pending independent UI review', () => {
    const { records, manifest } = fixture()
    const report = audit(records, manifest)
    assert.equal(report.status, 'structurally_valid')
    assert.equal(report.runtime_acceptance, 'pending_ui_review')
    assert.equal(report.complete, false)
    assert.equal(report.counts.snapshots, 2)
    assert.equal(report.counts.sources, 1)
    assert.deepEqual(report.snapshots.map(snapshot => snapshot.config_id), CONFIGS)
    assert.equal(report.snapshots[0].items[0].host_id_alias, null)
    assert.ok(!JSON.stringify(report).includes(CANARY))
})

test('API 1.3 aliases and navigation are projected without inferring identities or enumeration end', () => {
    const { records, manifest } = fixture({ hostIds: true, feedback: true, navigation: true })
    manifest.host = { cubase_version: '15.0.30', api_version: '1.3' }
    const report = audit(records, manifest)
    assert.equal(report.counts.feedback, 2)
    assert.equal(report.snapshots[0].generation, 2)
    assert.equal(report.snapshots[0].items[0].host_id_alias, 'host-1')
    assert.equal(report.snapshots[0].items[0].title_alias, report.snapshots[1].items[0].title_alias)
    assert.ok(report.limitations.some(value => value.includes('title aliases are not identities')))
})

test('source IDs, paths and source session identifiers are never copied into successful output', () => {
    const { records, manifest } = fixture({ source: CANARY })
    assert.ok(!JSON.stringify(audit(records, manifest)).includes(CANARY))
})

test('a reply and its entire chunk stream may precede the successful send-result emission', () => {
    for (const requestId of ['request-1', 'request-2', 'request-3']) {
        const { records, manifest } = fixture()
        const expected = audit(records, manifest)
        delayEvidence(records, requestId, { afterChunks: requestId !== 'request-1' })
        assert.deepEqual(audit(records, manifest), expected)
    }
})

test('@selected reply/chunks may precede the atomic command/send pair without losing correlation', () => {
    for (const navigation of [false, true]) {
        for (const afterChunks of [false, true]) {
            const { records, manifest } = fixture({ navigation })
            const expected = audit(records, manifest)
            for (const id of ['request-2', 'request-3']) delayEvidence(records, id, { selected: true, afterChunks })
            assert.deepEqual(audit(records, manifest), expected)
        }
    }
    const { records, manifest } = fixture({ capabilities: true })
    const expected = audit(records, manifest)
    delayEvidence(records, 'request-2', { selected: true })
    assert.deepEqual(audit(records, manifest), expected)
})

test('early replies do not excuse missing, duplicate, failed or mismatched request evidence', () => {
    const mutations = [
        (records, sent) => { records.splice(records.indexOf(sent), 1) },
        (records, sent) => { records.splice(records.indexOf(sent), 0, { ...sent }) },
        (_, sent) => { sent.sent = false },
        (_, sent) => { sent.checkpoint_id = 'OTHER' },
        (_, sent) => { sent.request_id = 'OTHER' },
        (_, sent) => { delete sent.send_completed_monotonic_timestamp_ms },
        (records, sent) => { sent.send_completed_monotonic_timestamp_ms = records.find(record =>
            record.message?.id === 'request-2').monotonic_timestamp_ms + 1 },
        (_, __, command) => { delete command.evidence_emission },
        (_, __, command) => { command.request.target_instance_id = 'OTHER' },
        (_, __, command) => { command.checkpoint_id = 'OTHER' },
        (_, __, command) => { command.request.message.id = 'OTHER' },
        (records, _, command) => { records.splice(records.indexOf(command), 0, { ...command }) },
        records => { chunks(records)[0].source_seq++ }
    ]
    for (const change of mutations) {
        const { records, manifest } = fixture()
        delayEvidence(records, 'request-2', { selected: true, afterChunks: true })
        const command = records.find(record => record.record_type === 'probe_command' && record.request_id === 'request-2')
        const sent = records.find(record => record.record_type === 'probe_command_send_result' && record.request_id === 'request-2')
        change(records, sent, command)
        assert.throws(() => audit(records, manifest))
    }
})

test('delayed begin/end sink records do not change the tracker observation or quiet intervals', () => {
    for (const delayedPhase of ['begin', 'end']) {
        const { records, manifest } = fixture()
        const expected = audit(records, manifest)
        const marker = records.find(record => record.record_type === 'collector_checkpoint' && record.phase === delayedPhase)
        marker.monotonic_timestamp_ms += 25
        let lastEmission = 0
        for (const record of records) {
            record.monotonic_timestamp_ms = Math.max(record.monotonic_timestamp_ms, lastEmission)
            record.timestamp_unix_ms = 100000 + record.monotonic_timestamp_ms
            lastEmission = record.monotonic_timestamp_ms
        }
        assert.deepEqual(audit(records, manifest), expected)
    }
})

test('tracker duration, quiet interval, and common receive-time boundary must remain consistent', () => {
    rejection(records => { chunks(records)[0].checkpoint_elapsed_ms += 20 })
    rejection(records => {
        const end = records.find(record => record.record_type === 'collector_checkpoint' && record.phase === 'end')
        end.observed_duration_ms = end.window_ms - 1
    })
    rejection(records => {
        const end = records.find(record => record.record_type === 'collector_checkpoint' && record.phase === 'end')
        end.quiet_period_observed_ms = 999
    })
    rejection(records => {
        const end = records.find(record => record.record_type === 'collector_checkpoint' && record.phase === 'end')
        end.quiet_period_observed_ms += 20
    })
    rejection(records => {
        const first = records.find(record => record.record_type === 'probe_event')
        first.checkpoint_elapsed_ms = first.received_at_monotonic_timestamp_ms + 1
    })
})

test('unknown raw-title/host-ID/error payload fields are rejected', () => {
    for (const name of ['title', 'host_id_raw', 'error', 'path']) {
        rejection(records => { chunks(records)[0].message.data.items[0][name] = CANARY })
        rejection(records => { chunks(records)[0].message.data[name] = CANARY })
        rejection(records => { records.find(record => record.message?.event === 'probe.loaded').message.data[name] = CANARY })
        rejection(records => { records.find(record => record.record_type === 'probe_response').message.result[name] = CANARY })
    }
})

test('aliases reject arbitrary strings, invalid prefixes, zero and overflow', () => {
    for (const alias of [CANARY, 'title-0', 'title-257', 'title-01', 'host-1', null]) {
        rejection(records => { chunks(records)[0].message.data.items[0].title_alias = alias }, { feedback: true })
    }
    for (const alias of [CANARY, 'host-0', 'host-257', 'host-01', 'title-1']) {
        rejection(records => { chunks(records)[0].message.data.items[0].host_id_alias = alias }, { hostIds: true })
    }
})

test('slot sequence, item shape, ID support and observed-title states are enforced', () => {
    for (const change of [item => { item.slot_index = 1 }, item => { item.slot_index = 8 },
        item => { item.title_observed = true }, item => { item.title_alias = 'title-1' },
        item => { item.host_id_status = 'supported' }, item => { item.host_id_status = 'error' },
        item => { item.host_id_alias = 'host-1' }, item => { item.host_id_supported = true },
        item => { delete item.config_id }]) {
        rejection(records => change(chunks(records)[0].message.data.items[0]))
    }
})

test('missing, overlapping and reordered chunks cannot satisfy a requested snapshot', () => {
    rejection(records => { records.splice(records.indexOf(chunks(records)[1]), 1) })
    rejection(records => { chunks(records)[1].message.data.chunk_index = 0 })
    rejection(records => { chunks(records)[1].message.data.snapshot_id = 'other' })
    rejection(records => { chunks(records)[0].message.data.chunk_count = 3 })
    rejection(records => { chunks(records)[0].message.data.items.pop() })
    rejection(records => { chunks(records)[0].message.data.snapshot_complete = true })
    rejection(records => { chunks(records)[0].message.data.reason = 'command_next' })
    rejection(records => { chunks(records)[0].message.data.complete = true })
    rejection(records => { chunks(records)[0].message.data.overflow_safe = false })
    rejection(records => { chunks(records)[0].message.data.truncated = true })
})

test('source gaps, replacement sources, stale callbacks and repeat activation are rejected', () => {
    rejection(records => { chunks(records)[0].source_seq++ })
    rejection(records => { chunks(records)[0].source_instance_id = 'other' })
    rejection(records => { chunks(records)[0].message.data.activation_epoch = 2 })
    rejection(records => { chunks(records)[0].message.event = 'probe.io.stale_callback' })
    rejection(records => { chunks(records)[0].message.event = 'probe.overflow' })
    rejection(records => { chunks(records)[0].message.data.generation++ })
    rejection(records => { chunks(records)[0].message.data.observation_id++ })
})

test('request target, send, response ID and checkpoint correspondence are required', () => {
    rejection(records => { records.find(record => record.record_type === 'probe_command_send_result').sent = false })
    rejection(records => { records.find(record => record.record_type === 'probe_response').message.id = 'other' })
    rejection(records => { records.find(record => record.record_type === 'probe_command').request.target_instance_id = 'other' })
    rejection(records => { chunks(records)[0].checkpoint_id = 'other' })
    rejection(records => { chunks(records)[0].processed_after_checkpoint_end = true })
    rejection(records => { chunks(records)[0].orphan = true })
    rejection(records => { records.find(record => record.record_type === 'collector_discovery_completed').responder_count = 2 })
})

test('wrong profile, missing source-file metadata and unsupported host pair are rejected', () => {
    rejection(records => { records[0].probe_profile = 'primary' })
    rejection(records => { chunks(records)[0].message.data.profile = 'primary' })
    rejection((_, manifest) => { manifest.host.api_version = '1.3' })
    rejection((_, manifest) => { manifest.expected_collector_sha256 = 'b'.repeat(64) })
    rejection((_, manifest) => { delete manifest.expected_probe_files['wire.js'] })
    rejection((_, manifest) => { manifest.expected_probe_files['wire.js'] = CANARY })
    rejection((_, manifest) => { manifest.ui_review = 'accepted' })
})

test('summary fields must agree with actual complete records, not merely claim success', () => {
    for (const field of ['exit_ok', 'integrity_ok']) rejection(records => { records.at(-1)[field] = false })
    for (const field of ['errors', 'diagnostics', 'queue_drops', 'source_overflows', 'sequence_gaps']) {
        rejection(records => { records.at(-1).incoming[field] = 1 })
    }
    rejection(records => { records.pop() })
    rejection(records => { records.at(-1).protocol_tracking.open_snapshots = 1 })
    rejection(records => { records.at(-1).protocol_tracking.completed_requests++ })
    rejection(records => { records.at(-1).graceful_drain.completed = false })
    rejection(records => { records.at(-1).incoming.sources.push({ source_instance_id: 'other', last_source_seq: 1 }) })
    rejection(records => { records.at(-1).incoming.messages++ })
    rejection(records => { records.at(-1).commands.sent++ })
    rejection(records => { records[1].monotonic_timestamp_ms = 9000 })
    rejection(records => { records.find(record => record.record_type === 'collector_checkpoint' && record.phase === 'end').observed_duration_ms += 2 })
})

test('two complete input pages cannot substitute for observing the output configuration', () => {
    rejection(records => {
        for (const record of records) {
            const params = record.request?.message?.params
            if (params?.config_id === 'IO_OUTPUT_ALL') params.config_id = 'IO_INPUT_ALL'
            const result = record.message?.result
            if (result?.config_id === 'IO_OUTPUT_ALL') result.config_id = 'IO_INPUT_ALL'
            const data = record.message?.data
            if (data?.config_id === 'IO_OUTPUT_ALL') {
                data.config_id = 'IO_INPUT_ALL'
                data.items.forEach(item => { item.config_id = 'IO_INPUT_ALL' })
            }
        }
    })
})

test('bounded CLI reports only fixed errors for malformed JSON, paths, raw secrets and truncated captures', () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'io-probe-audit-'))
    const rawFile = path.join(directory, 'raw.jsonl'); const manifestFile = path.join(directory, 'manifest.json')
    const script = path.join(__dirname, '..', 'scripts', 'audit-io-probe.js')
    const { records, manifest } = fixture({ source: CANARY })
    const invoke = (raw = rawFile) => spawnSync(process.execPath, [script, raw, manifestFile], { encoding: 'utf8' })
    try {
        fs.writeFileSync(manifestFile, JSON.stringify(manifest))
        fs.writeFileSync(rawFile, records.map(record => JSON.stringify(record)).join('\n') + '\n')
        let result = invoke(); assert.equal(result.status, 0); assert.ok(!result.stdout.includes(CANARY))
        for (const raw of [CANARY + '\n', '{"secret":"' + CANARY + '"', '{}\n', Buffer.alloc(16 * 1024 * 1024 + 1)]) {
            fs.writeFileSync(rawFile, raw); result = invoke()
            assert.equal(result.status, 1); assert.equal(result.stdout, ''); assert.equal(result.stderr, 'IO_PROBE_AUDIT_REJECTED\n')
        }
        result = invoke(path.join(directory, CANARY)); assert.equal(result.stderr, 'IO_PROBE_AUDIT_REJECTED\n')
    } finally { fs.rmSync(directory, { recursive: true, force: true }) }
})
