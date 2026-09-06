// Existing Input/Output observation only. ES5 / MIDI Remote API v1.1.
// Install all three files together; this is not the primary Track Probe.
var api = require('midiremote_api_v1')
var createProfile = require('./io-profile').createProfile
var wire = require('./wire')
var PROFILE = 'io-existing-v1'
var driver = api.makeDeviceDriver('CubaseMCPIOProbe', 'CubaseMCPIOProbe', 'Cubase MCP contributors')
var input = driver.mPorts.makeMidiInput('Cubase MCP IO Probe Input')
var output = driver.mPorts.makeMidiOutput('Cubase MCP IO Probe Output')
driver.makeDetectionUnit().detectPortPair(input, output)
    .expectInputNameContains('Cubase MCP IO Probe To Cubase')
    .expectOutputNameContains('Cubase MCP IO Probe From Cubase')
var page = driver.mMapping.makePage('Cubase MCP Existing IO Probe')
var instanceId = 'io-probe-' + new Date().getTime().toString(36) + '-' +
    Math.floor(Math.random() * 2147483647).toString(36)
var sourceSequence = 0, snapshotSequence = 0
var activeDevice = null, activeMapping = null, active = false, ready = false, loaded = false
var fatal = null, pending = null, feedback = [], seenIds = {}, requestCount = 0
var core = null

function keysAre(value, expected) {
    if (!value || typeof value !== 'object' || Object.prototype.toString.call(value) === '[object Array]') return false
    var actual = Object.keys(value).sort()
    return actual.join('|') === expected.slice(0).sort().join('|')
}

function fixedCode(error) {
    var codes = ['NOT_CONNECTED', 'NOT_SUPPORTED', 'INVALID_ARGUMENT', 'BUSY', 'INTERNAL_ERROR', 'OVERFLOW']
    return error && codes.indexOf(error.code) >= 0 ? error.code : 'INTERNAL_ERROR'
}

function copy(value) {
    var result = {}
    Object.keys(value).forEach(function (key) { result[key] = value[key] })
    return result
}

function send(message) {
    var envelope = { probe_transport_version: 1, source_instance_id: instanceId,
        source_seq: sourceSequence + 1, message: message }
    var frame = wire.encode(envelope, 2048)
    // Count the attempted frame, so loss cannot be disguised as a contiguous stream.
    sourceSequence++
    output.sendMidi(activeDevice, frame)
}

function event(name, data) {
    data = copy(data)
    data.profile = PROFILE
    send({ version: 1, type: 'event', event: name, data: data })
}

function response(id, result) {
    result = copy(result)
    result.profile = PROFILE
    send({ version: 1, id: id, type: 'response', result: result })
}

function errorResponse(id, code) {
    send({ version: 1, id: id, type: 'error', error: { code: code, message: code } })
}

function fail(code) {
    if (fatal !== null) return
    fatal = code
    ready = false
    pending = null
    feedback = []
    // Never forward exception messages or failed payloads from the host.
    try { event('probe.overflow', { stream: 'io_profile', error_code: code }) } catch (ignored) {}
}

function queueFeedback(value) {
    if (fatal !== null || feedback.length >= 256) {
        fail('OVERFLOW')
        return false
    }
    feedback.push(value)
    return true
}

try { core = createProfile(page.mHostAccess.mMixConsole, queueFeedback) }
catch (error) { fatal = fixedCode(error) }

function capabilities() {
    var result = core ? copy(core.capabilities()) : { profile: PROFILE, active: false,
        activation_epoch: 0, fatal_error: fatal, slot_count: 8, configs: [],
        limits: { aliases: 256, raw_string_length: 4096, pending_feedback: 128 },
        metadata_only: true, complete: false }
    result.bus_mutation = false
    result.integrity_failed = fatal !== null
    return result
}

function flushFeedback() {
    // Each idle has a finite amount of work; reentrant host callbacks remain queued.
    var batch = feedback
    feedback = []
    for (var i = 0; i < batch.length && fatal === null; i++) event(batch[i].event, batch[i].data)
}

driver.mOnActivate = function (device) { activeDevice = device }
driver.mOnDeactivate = function (device) { deactivate(device) }
page.mOnActivate = function (device, mapping) {
    if (active || pending !== null) { fail('INTERNAL_ERROR'); return }
    activeDevice = device
    activeMapping = mapping
    active = true
    ready = false
    try {
        if (!loaded) {
            event('probe.loaded', { probe_session_id: instanceId, mapping_active: true,
                read_only: true, protocol_version: 1 })
            loaded = true
        }
        event('probe.mapping_active', { probe_session_id: instanceId,
            mapping_active: true, read_only: true, protocol_version: 1 })
        if (core && fatal === null) core.activate(device, mapping)
        event('probe.capabilities', capabilities())
    } catch (error) { fail(fixedCode(error)) }
}

function deactivate(device) {
    if (!active || device !== activeDevice) return
    if (pending !== null) fail('NOT_CONNECTED')
    try {
        if (core) core.deactivate()
        flushFeedback()
        event('probe.ready', { probe_session_id: instanceId, protocol_version: 1,
            ready: false, read_only: true, mapping_active: false,
            activation_epoch: core ? core.capabilities().activation_epoch : 0 })
    } catch (error) { fail(fixedCode(error)) }
    active = false
    ready = false
    activeMapping = null
}
page.mOnDeactivate = function (device) { deactivate(device) }

function emitSnapshot(task, snapshot) {
    var id = instanceId + '-snapshot-' + (++snapshotSequence)
    for (var chunk = 0; chunk < 4; chunk++) {
        var items = []
        for (var i = chunk * 2; i < chunk * 2 + 2; i++) {
            var source = snapshot.items[i], item = { config_id: task.config_id }
            var names = ['slot_index', 'title_observed', 'title_state', 'title_alias',
                'host_id_supported', 'host_id_status', 'host_id_alias']
            for (var k = 0; k < names.length; k++) item[names[k]] = source[names[k]]
            items.push(item)
        }
        event('probe.bank.chunk', { snapshot_id: id, stream: 'mixer_bank_snapshot',
            config_id: task.config_id, reason: task.reason, chunk_index: chunk,
            chunk_count: 4, total_items: 8, snapshot_complete: chunk === 3,
            truncated: false, overflow_safe: true, activation_epoch: snapshot.activation_epoch,
            generation: snapshot.generation, observation_id: snapshot.observation_id,
            complete: false, metadata_only: true, items: items })
    }
}

page.mOnIdle = function (device, mapping) {
    if (!active || device !== activeDevice || mapping !== activeMapping || fatal !== null) return
    try {
        var coreFatal = core.capabilities().fatal_error
        if (coreFatal !== null) { fail(coreFatal); return }
        flushFeedback()
        if (fatal !== null) return
        if (!ready) {
            ready = true // Request handling is ready; callback completeness is NOT implied.
            event('probe.ready', { probe_session_id: instanceId, protocol_version: 1,
                ready: true, read_only: true, mapping_active: true, initial_snapshots_complete: true,
                activation_epoch: core.capabilities().activation_epoch })
        }
        if (pending !== null && feedback.length === 0) {
            var task = pending
            var snapshot = core.snapshot(task.config_id)
            flushFeedback()
            if (fatal === null) emitSnapshot(task, snapshot)
            pending = null
        }
    } catch (error) { fail(fixedCode(error)) }
}

input.mOnSysex = function (device, frame) {
    if (device !== activeDevice) return
    var envelope = wire.decode(frame, 4096)
    if (!keysAre(envelope, ['probe_transport_version', 'target_instance_id', 'message']) ||
        envelope.probe_transport_version !== 1) return
    var request = envelope.message
    if (!keysAre(request, ['version', 'id', 'type', 'method', 'params']) ||
        request.version !== 1 || request.type !== 'request' ||
        typeof request.id !== 'string' || !/^[A-Za-z0-9._:-]{1,256}$/.test(request.id) ||
        typeof request.method !== 'string') return
    if (request.method === 'probe.discover' ? envelope.target_instance_id !== null :
        envelope.target_instance_id !== instanceId) return
    try { handle(request) } catch (error) {
        try { errorResponse(request.id, fixedCode(error)) } catch (ignored) {}
        if (fixedCode(error) !== 'INVALID_ARGUMENT' && fixedCode(error) !== 'NOT_SUPPORTED') fail(fixedCode(error))
    }
}

function handle(request) {
    var id = request.id, method = request.method
    if (seenIds['$' + id]) { errorResponse(id, 'INVALID_ARGUMENT'); return }
    if (requestCount >= 4096) { fail('OVERFLOW'); errorResponse(id, 'OVERFLOW'); return }
    seenIds['$' + id] = true
    requestCount++
    if (method === 'probe.discover' || method === 'probe.capabilities.get') {
        if (!keysAre(request.params, [])) { errorResponse(id, 'INVALID_ARGUMENT'); return }
        if (method === 'probe.discover') response(id, { instance_id: instanceId, ready: ready, read_only: true })
        else response(id, capabilities())
        return
    }
    var actions = { 'probe.bank.reset': 'reset', 'probe.bank.next': 'next', 'probe.bank.prev': 'prev' }
    if (method !== 'probe.bank.snapshot' && !Object.prototype.hasOwnProperty.call(actions, method)) {
        errorResponse(id, 'NOT_SUPPORTED'); return
    }
    if (!keysAre(request.params, ['config_id']) ||
        ['IO_INPUT_ALL', 'IO_OUTPUT_ALL'].indexOf(request.params.config_id) < 0) {
        errorResponse(id, 'INVALID_ARGUMENT'); return
    }
    if (!active) { errorResponse(id, 'NOT_CONNECTED'); return }
    if (fatal !== null) { errorResponse(id, 'INTERNAL_ERROR'); return }
    if (!ready || pending !== null) { errorResponse(id, 'BUSY'); return }
    var action = actions[method]
    if (action) core.navigate(request.params.config_id, action)
    var result = { config_id: request.params.config_id, scheduled: true }
    if (action) result.action = action
    response(id, result)
    pending = { config_id: request.params.config_id, reason: 'command_' + (action || 'snapshot') }
}
