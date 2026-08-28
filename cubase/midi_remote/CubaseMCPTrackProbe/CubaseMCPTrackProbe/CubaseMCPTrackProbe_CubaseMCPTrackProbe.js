// Cubase MCP read-only Track API probe.
// Compatible with Cubase 13 / MIDI Remote API v1.1 (ES5 JavaScript).

var midiremote_api = require('midiremote_api_v1')

var deviceDriver = midiremote_api.makeDeviceDriver(
    'CubaseMCPTrackProbe',
    'CubaseMCPTrackProbe',
    'Cubase MCP contributors'
)

var midiInput = deviceDriver.mPorts.makeMidiInput('Cubase MCP Track Probe Input')
var midiOutput = deviceDriver.mPorts.makeMidiOutput('Cubase MCP Track Probe Output')

deviceDriver.makeDetectionUnit()
    .detectPortPair(midiInput, midiOutput)
    .expectInputNameContains('Cubase MCP Track Probe To Cubase')
    .expectOutputNameContains('Cubase MCP Track Probe From Cubase')

var page = deviceDriver.mMapping.makePage('Cubase MCP Track Probe')
var mixConsole = page.mHostAccess.mMixConsole

var PROBE_PROTOCOL_VERSION = 1
var SYSEX_HEADER = [0xF0, 0x7D, 0x43, 0x4D, 0x54, 0x50, 0x01]
var MAX_INPUT_JSON_BYTES = 4096
var MAX_OUTPUT_JSON_BYTES = 2048
var MAX_STRING_BYTES = 128
var MAX_INLINE_HOST_ID_BYTES = 256
var MAX_HOST_ID_FRAGMENT_BYTES = 256
var MAX_HOST_ID_BYTES = 4096
var MAX_HOST_ID_FRAGMENTS = 16
var MAX_WIRE_ITEMS_PER_SNAPSHOT = 1024
var MAX_ITEMS_PER_CHUNK = 2
// Two 8-slot banks can each emit title, binding-title, selected, mute, and solo
// callbacks during activation (80 records). Leave bounded headroom for an
// initial DirectAccess change burst without making the queue unbounded.
var MAX_PENDING_FEEDBACK = 512
var MAX_FEEDBACK_PER_IDLE = 64
var MAX_DEACTIVATION_FEEDBACK_FLUSH_PASSES =
    Math.ceil(MAX_PENDING_FEEDBACK / MAX_FEEDBACK_PER_IDLE) + 2
var MAX_PENDING_BANK_SNAPSHOTS = 32
var MAX_PENDING_DIRECT_ACCESS_SNAPSHOTS = 16
var MAX_DIRECT_ACCESS_NODES = 256
var MAX_DIRECT_ACCESS_DEPTH = 32
var MAX_DIRECT_ACCESS_CHILDREN = 128
var MAX_OBSERVATION_EPOCH = 2147483647
var BANK_SLOT_COUNT = 8

// Only revision-2 fixture Track titles may cross the probe boundary. This is
// deliberately an exact allowlist rather than a CMCP_ prefix check: an
// unrelated project, Input/Output bus, or VCA can otherwise make its ambient
// title and opaque host ID observable when a host-side filter regresses.
var FIXTURE_TITLE_ALLOWLIST = [
    'CMCP_E1_ONLY_AUDIO',
    'CMCP_E8_01',
    'CMCP_E8_02',
    'CMCP_E8_03',
    'CMCP_E8_04',
    'CMCP_E8_05',
    'CMCP_E8_06',
    'CMCP_E8_07',
    'CMCP_E8_08',
    'CMCP_01_FOLDER_EMPTY',
    'CMCP_DUPLICATE',
    'CMCP_04_MIDI_ASCII',
    'CMCP_05_日本語_é_🎹',
    'CMCP_05_日本語_e\u0301_🎹',
    'CMCP_06_GROUP',
    'CMCP_07_FX',
    'CMCP_08_HIDDEN',
    'CMCP_10_MUTATE_RENAME',
    'CMCP_11_MUTATE_DELETE',
    'CMCP_12_MUTATION_ANCHOR',
    'CMCP_13_STATE_S0_M0_SO0',
    'CMCP_14_STATE_S0_M0_SO1',
    'CMCP_15_STATE_S0_M1_SO0',
    'CMCP_16_STATE_S0_M1_SO1',
    'CMCP_17_STATE_S1_M0_SO0',
    'CMCP_18_STATE_S1_M0_SO1',
    'CMCP_19_STATE_S1_M1_SO0',
    'CMCP_20_STATE_S1_M1_SO1',
    'CMCP_10_RENAMED_変更後',
    'CMCP_21_ADDED'
]
var FIXTURE_P09_TITLE =
    'CMCP_09_LONG_ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNO'
var FIXTURE_P09_PREFIX = 'CMCP_09_LONG_'
var SAFE_DIRECT_ACCESS_TYPE_NAMES = [
    'MixConsole',
    'AudioChannel',
    'MIDIChannel',
    'InstrumentChannel',
    'GroupChannel',
    'FXChannel',
    'FolderTrack'
]

var activeDeviceRef = null
var activeMappingRef = null
var pageIsActive = false
var pageIsReady = false
var pageActivationSeen = false
var postDeactivationCallbackReported = false
var loadMarkerSent = false
var activationInitializationPending = false
var probeIntegrityFailed = false
var sourceSequence = 0
var observationSequence = 0
var observationEpoch = 0
var snapshotSequence = 0
var probeInstanceId = createInstanceId()

var pendingFeedback = []
var pendingDroppedFeedback = makeDroppedFeedbackState()
var pendingBankSnapshots = []
var pendingDirectAccessSnapshots = []

var bankConfigs = [
    makeCoreBankConfig('MB_CORE_ALL', false),
    makeCoreBankConfig('MB_CORE_VISIBLE', true)
]

var directAccess = null
var directAccessActive = false
var directAccessActivationError = null
var directAccessUpdateInProgress = false
var directAccessCapabilities = makeDirectAccess()

deviceDriver.mOnActivate = function (activeDevice) {
    activeDeviceRef = activeDevice
}

deviceDriver.mOnDeactivate = function (activeDevice) {
    deactivatePage(activeDevice, activeMappingRef)
    activeDeviceRef = null
}

page.mOnActivate = function (activeDevice, activeMapping) {
    activeDeviceRef = activeDevice
    activeMappingRef = activeMapping
    pageIsActive = true
    pageActivationSeen = true
    postDeactivationCallbackReported = false
    pageIsReady = false
    activationInitializationPending = true

    if (!loadMarkerSent) {
        sendEvent(activeDevice, 'probe.loaded', {
            probe_session_id: probeInstanceId,
            mapping_active: true,
            read_only: true,
            protocol_version: PROBE_PROTOCOL_VERSION
        })
        loadMarkerSent = true
    }
    sendEvent(activeDevice, 'probe.mapping_active', {
        probe_session_id: probeInstanceId,
        mapping_active: true,
        read_only: true,
        protocol_version: PROBE_PROTOCOL_VERSION
    })

    for (var configIndex = 0; configIndex < bankConfigs.length; ++configIndex) {
        beginBankGeneration(bankConfigs[configIndex])
    }

    activateDirectAccess(activeDevice, activeMapping)
    sendCapabilities(activeDevice)

    for (var snapshotIndex = 0; snapshotIndex < bankConfigs.length; ++snapshotIndex) {
        scheduleBankSnapshot(bankConfigs[snapshotIndex].id, 'page_activate')
    }
    if (directAccessActive) {
        scheduleDirectAccessSnapshot('page_activate')
    }
}

page.mOnDeactivate = function (activeDevice, activeMapping) {
    deactivatePage(activeDevice, activeMapping)
}

page.mOnIdle = function (activeDevice, activeMapping) {
    activeDeviceRef = activeDevice
    activeMappingRef = activeMapping

    flushFeedback(activeDevice)
    if (!feedbackIsPending()) {
        flushOneBankSnapshot(activeDevice, activeMapping)
        // Activation queues both Mixer Bank projections before DirectAccess.
        // Drain that bank batch first so it cannot become
        // ALL, DirectAccess, VISIBLE.
        if (pendingBankSnapshots.length === 0) {
            flushOneDirectAccessSnapshot(activeDevice, activeMapping)
        }
    }
    maybeFinishActivation(activeDevice)
}

midiInput.mOnSysex = function (activeDevice, midiMessage) {
    var jsonText = decodeFrame(midiMessage)
    if (jsonText === null) {
        return
    }

    var envelope = null
    try {
        envelope = JSON.parse(jsonText)
    } catch (error) {
        return
    }

    if (
        envelope === null ||
        typeof envelope !== 'object' ||
        envelope.probe_transport_version !== PROBE_PROTOCOL_VERSION ||
        envelope.message === null ||
        typeof envelope.message !== 'object'
    ) {
        return
    }

    var request = envelope.message
    if (!request || typeof request.id !== 'string' || typeof request.method !== 'string') {
        return
    }

    if (request.method === 'probe.discover') {
        if (envelope.target_instance_id !== null) {
            return
        }
    } else if (envelope.target_instance_id !== probeInstanceId) {
        return
    }

    try {
        handleRequest(activeDevice, request)
    } catch (error) {
        sendError(
            activeDevice,
            request.id,
            'INTERNAL_ERROR',
            'Unhandled probe request failure'
        )
    }
}

function makeCoreBankConfig(configId, followVisibility) {
    var zone = mixConsole.makeMixerBankZone(configId)

    zone.includeAudioChannels()
    zone.includeInstrumentChannels()
    zone.includeMIDIChannels()
    zone.includeGroupChannels()
    zone.includeFXChannels()
    zone.excludeSamplerChannels()
    zone.excludeVCAChannels()
    zone.excludeInputChannels()
    zone.excludeOutputChannels()
    zone.excludeWindowZoneLeftChannels()
    zone.excludeWindowZoneRightChannels()
    var explicitMainFilter = typeof zone.includeWindowZoneMainChannels === 'function'
    if (explicitMainFilter) {
        zone.includeWindowZoneMainChannels()
    }
    zone.setFollowVisibility(followVisibility)

    return finishBankConfig(configId, followVisibility, explicitMainFilter, zone)
}

function finishBankConfig(configId, followVisibility, explicitMainFilter, zone) {

    var config = {
        id: configId,
        follow_visibility: followVisibility,
        explicit_main_filter: explicitMainFilter,
        generation: 0,
        zone: zone,
        slots: []
    }

    for (var slotIndex = 0; slotIndex < BANK_SLOT_COUNT; ++slotIndex) {
        config.slots.push(makeBankSlot(config, slotIndex))
    }
    return config
}

function makeBankSlot(config, slotIndex) {
    var channel = config.zone.makeMixerBankChannel()
    var prefix = config.id + ' Slot ' + slotIndex + ' '
    var selectedFeedback = deviceDriver.mSurface.makeCustomValueVariable(prefix + 'Selected')
    var muteFeedback = deviceDriver.mSurface.makeCustomValueVariable(prefix + 'Mute')
    var soloFeedback = deviceDriver.mSurface.makeCustomValueVariable(prefix + 'Solo')

    page.makeValueBinding(selectedFeedback, channel.mValue.mSelected).setTypeToggle()
    page.makeValueBinding(muteFeedback, channel.mValue.mMute).setTypeToggle()
    page.makeValueBinding(soloFeedback, channel.mValue.mSolo).setTypeToggle()

    var slot = {
        index: slotIndex,
        channel: channel,
        selected_feedback: selectedFeedback,
        mute_feedback: muteFeedback,
        solo_feedback: soloFeedback,
        state: {
            title: null,
            title_redacted: false,
            title_authorized: false,
            selected: null,
            mute: null,
            solo: null,
            unique_id: null,
            unique_id_observation_status: 'not_observed',
            observed_generation: {
                title: -1,
                selected: -1,
                mute: -1,
                solo: -1,
                unique_id: -1
            },
            last_observation_seq: {
                title: null,
                selected: null,
                mute: null,
                solo: null,
                unique_id: null
            },
            last_observation_epoch: {
                title: null,
                selected: null,
                mute: null,
                solo: null,
                unique_id: null
            }
        }
    }

    channel.mOnTitleChange = function (activeDevice, activeMapping, title) {
        applyBankTitlePolicy(slot, title)
        slot.state.observed_generation.title = config.generation
        refreshBankUniqueId(config, slot, activeMapping)
        recordBankFeedback(
            config,
            slot,
            'title',
            slot.state.title,
            'mixer_bank_channel'
        )
    }

    // A bound surface value supplies an independent object-title callback.
    // Keeping both paths visible is intentional: the host spike records which
    // callback path survives bank moves and empty slots on each Cubase build.
    selectedFeedback.mOnTitleChange = function (activeDevice, objectTitle, valueTitle) {
        applyBankTitlePolicy(slot, objectTitle)
        slot.state.observed_generation.title = config.generation
        refreshBankUniqueId(config, slot, activeMappingRef)
        recordBankFeedback(
            config,
            slot,
            'title',
            slot.state.title,
            'selected_binding'
        )
    }

    selectedFeedback.mOnProcessValueChange = function (activeDevice, value) {
        slot.state.selected = feedbackBoolean(value)
        slot.state.observed_generation.selected = config.generation
        recordBankFeedback(
            config,
            slot,
            'selected',
            slot.state.selected,
            'selected_binding'
        )
    }

    muteFeedback.mOnProcessValueChange = function (activeDevice, value) {
        slot.state.mute = feedbackBoolean(value)
        slot.state.observed_generation.mute = config.generation
        recordBankFeedback(config, slot, 'mute', slot.state.mute, 'mute_binding')
    }

    soloFeedback.mOnProcessValueChange = function (activeDevice, value) {
        slot.state.solo = feedbackBoolean(value)
        slot.state.observed_generation.solo = config.generation
        recordBankFeedback(config, slot, 'solo', slot.state.solo, 'solo_binding')
    }

    return slot
}

function applyBankTitlePolicy(slot, value) {
    slot.state.title = null
    slot.state.title_redacted = false
    slot.state.title_authorized = false
    if (typeof value !== 'string' || value.length === 0) {
        return
    }
    if (fixtureTitleIsAllowed(value)) {
        slot.state.title = value
        slot.state.title_authorized = true
        return
    }
    slot.state.title_redacted = true
}

function feedbackBoolean(value) {
    if (typeof value !== 'number' || !isFinite(value)) {
        return null
    }
    return value >= 0.5
}

function refreshBankUniqueId(config, slot, activeMapping) {
    if (!slot.state.title_authorized) {
        slot.state.unique_id = null
        slot.state.unique_id_observation_status = 'title_not_authorized'
        slot.state.observed_generation.unique_id = -1
        return
    }
    if (activeMapping === null) {
        slot.state.unique_id = null
        slot.state.unique_id_observation_status = 'mapping_unavailable'
        slot.state.observed_generation.unique_id = -1
        return
    }
    if (
        !slot.channel ||
        typeof slot.channel.getUniqueIDString !== 'function'
    ) {
        slot.state.unique_id = null
        slot.state.unique_id_observation_status = 'getter_unavailable'
        slot.state.observed_generation.unique_id = -1
        return
    }
    try {
        var value = slot.channel.getUniqueIDString(activeMapping)
        if (typeof value === 'string') {
            slot.state.unique_id = value
            slot.state.unique_id_observation_status =
                'observed_with_title_callback'
            slot.state.observed_generation.unique_id = config.generation
        } else {
            slot.state.unique_id = null
            slot.state.unique_id_observation_status = 'invalid_type'
            slot.state.observed_generation.unique_id = -1
        }
    } catch (error) {
        slot.state.unique_id = null
        slot.state.unique_id_observation_status = 'getter_failed'
        slot.state.observed_generation.unique_id = -1
    }
}

function recordBankFeedback(config, slot, field, value, callbackSource) {
    if (!pageIsActive) {
        if (pageActivationSeen) {
            reportPostDeactivationCallback({
                stream: 'post_deactivation_callback',
                callback_source: sanitizeString(callbackSource),
                callback_field: sanitizeString(field),
                config_id: config.id,
                slot_index: slot.index
            })
        }
        return
    }

    var callbackEpoch = observationEpoch
    ++observationSequence
    slot.state.last_observation_seq[field] = observationSequence
    slot.state.last_observation_epoch[field] = callbackEpoch
    if (field === 'title') {
        slot.state.last_observation_seq.unique_id =
            slot.state.unique_id_observation_status ===
            'observed_with_title_callback'
                ? observationSequence
                : null
        slot.state.last_observation_epoch.unique_id =
            slot.state.unique_id_observation_status ===
            'observed_with_title_callback'
                ? callbackEpoch
                : null
    }
    var item = bankSlotItem(config, slot)
    item.observation_seq = observationSequence
    item.observation_epoch = callbackEpoch
    item.observation_epoch_status = 'callback_observed'
    item.changed_field = field
    item.changed_value = field === 'title' ? item.title : value
    item.changed_value_redacted =
        field === 'title' && item.title_redacted
    item.callback_source = callbackSource

    enqueueFeedback('mixer_bank_feedback', 'probe.bank.chunk', item)
}

function bankSlotItem(config, slot) {
    var observed = slot.state.observed_generation
    var generation = config.generation
    var titleWasObserved = observed.title === generation
    var titleRedacted = titleWasObserved && slot.state.title_redacted
    var hostIdRedacted =
        titleRedacted &&
        slot.channel &&
        typeof slot.channel.getUniqueIDString === 'function'
    return {
        record_kind: 'observation',
        config_id: config.id,
        bank_generation: generation,
        slot_index: slot.index,
        title: titleWasObserved ? slot.state.title : null,
        title_redacted: titleRedacted,
        selected: observed.selected === generation ? slot.state.selected : null,
        mute: observed.mute === generation ? slot.state.mute : null,
        solo: observed.solo === generation ? slot.state.solo : null,
        host_id_raw: observed.unique_id === generation ? slot.state.unique_id : null,
        host_id_redacted: hostIdRedacted,
        host_id_observed_with_title_callback:
            observed.unique_id === generation &&
            slot.state.unique_id_observation_status ===
                'observed_with_title_callback',
        host_id_observation_status:
            titleWasObserved
                ? slot.state.unique_id_observation_status
                : 'not_observed',
        redacted_string_count:
            (titleRedacted ? 1 : 0) + (hostIdRedacted ? 1 : 0),
        field_observation_generation: {
            title: observed.title,
            selected: observed.selected,
            mute: observed.mute,
            solo: observed.solo,
            host_id_raw: observed.unique_id
        },
        field_last_observation_seq: {
            title: slot.state.last_observation_seq.title,
            selected: slot.state.last_observation_seq.selected,
            mute: slot.state.last_observation_seq.mute,
            solo: slot.state.last_observation_seq.solo,
            host_id_raw: slot.state.last_observation_seq.unique_id
        },
        field_last_observation_epoch: {
            title: slot.state.last_observation_epoch.title,
            selected: slot.state.last_observation_epoch.selected,
            mute: slot.state.last_observation_epoch.mute,
            solo: slot.state.last_observation_epoch.solo,
            host_id_raw: slot.state.last_observation_epoch.unique_id
        }
    }
}

function beginBankGeneration(config) {
    ++config.generation
    for (var index = 0; index < config.slots.length; ++index) {
        var state = config.slots[index].state
        state.title = null
        state.title_redacted = false
        state.title_authorized = false
        state.selected = null
        state.mute = null
        state.solo = null
        state.unique_id = null
        state.unique_id_observation_status = 'not_observed'
    }
}

function makeDroppedFeedbackState() {
    return {
        total: 0,
        first_observation_seq: null,
        last_observation_seq: null,
        mixer_bank_feedback: 0,
        direct_access_feedback: 0
    }
}

function enqueueFeedback(stream, eventName, item) {
    if (pendingFeedback.length >= MAX_PENDING_FEEDBACK) {
        probeIntegrityFailed = true
        pageIsReady = false
        ++pendingDroppedFeedback.total
        ++pendingDroppedFeedback[stream]
        if (pendingDroppedFeedback.first_observation_seq === null) {
            pendingDroppedFeedback.first_observation_seq = item.observation_seq
        }
        pendingDroppedFeedback.last_observation_seq = item.observation_seq
        return
    }
    pendingFeedback.push({
        kind: 'item',
        stream: stream,
        event_name: eventName,
        observation_seq: item.observation_seq,
        item: item
    })
}

function enqueueDroppedFeedbackMarker() {
    if (
        pendingDroppedFeedback.total === 0 ||
        pendingFeedback.length >= MAX_PENDING_FEEDBACK
    ) {
        return
    }

    pendingFeedback.push({
        kind: 'overflow',
        observation_seq: pendingDroppedFeedback.first_observation_seq,
        data: {
            stream: 'feedback_queue',
            dropped_items: pendingDroppedFeedback.total,
            dropped_by_stream: {
                mixer_bank_feedback: pendingDroppedFeedback.mixer_bank_feedback,
                direct_access_feedback: pendingDroppedFeedback.direct_access_feedback
            },
            first_dropped_observation_seq:
                pendingDroppedFeedback.first_observation_seq,
            last_dropped_observation_seq:
                pendingDroppedFeedback.last_observation_seq,
            queue_limit: MAX_PENDING_FEEDBACK
        }
    })
    pendingDroppedFeedback = makeDroppedFeedbackState()
}

function flushFeedback(activeDevice) {
    enqueueDroppedFeedbackMarker()
    var processed = 0
    while (processed < MAX_FEEDBACK_PER_IDLE && pendingFeedback.length > 0) {
        var first = pendingFeedback.shift()
        ++processed
        if (first.kind === 'overflow') {
            sendOverflow(activeDevice, first.data)
            continue
        }

        var records = [first]
        while (
            processed < MAX_FEEDBACK_PER_IDLE &&
            pendingFeedback.length > 0 &&
            pendingFeedback[0].kind === 'item' &&
            pendingFeedback[0].stream === first.stream
        ) {
            records.push(pendingFeedback.shift())
            ++processed
        }

        var items = []
        for (var index = 0; index < records.length; ++index) {
            items.push(records[index].item)
        }
        sendChunkedEvent(
            activeDevice,
            first.event_name,
            first.stream,
            'feedback',
            items,
            false,
            {
                first_observation_seq: records[0].observation_seq,
                last_observation_seq: records[records.length - 1].observation_seq,
                remaining_items: pendingFeedback.length
            }
        )
    }
    enqueueDroppedFeedbackMarker()
}

function flushFeedbackBeforeDeactivation(activeDevice) {
    var passes = 0
    while (
        feedbackIsPending() &&
        passes < MAX_DEACTIVATION_FEEDBACK_FLUSH_PASSES
    ) {
        flushFeedback(activeDevice)
        ++passes
    }
}

function isInvalidatedDirectAccessSnapshot(pending) {
    return pending.reason === 'object_change' ||
        pending.reason === 'object_will_be_removed' ||
        pending.reason === 'parameter_change'
}

function cancelInvalidatedDirectAccessSnapshots() {
    var retained = []
    for (var index = 0; index < pendingDirectAccessSnapshots.length; ++index) {
        if (!isInvalidatedDirectAccessSnapshot(pendingDirectAccessSnapshots[index])) {
            retained.push(pendingDirectAccessSnapshots[index])
        }
    }
    pendingDirectAccessSnapshots = retained
}

function feedbackIsPending() {
    return pendingFeedback.length > 0 || pendingDroppedFeedback.total > 0
}

function scheduleBankSnapshot(configId, reason) {
    var config = findBankConfig(configId)
    if (pendingBankSnapshots.length >= MAX_PENDING_BANK_SNAPSHOTS) {
        return false
    }
    pendingBankSnapshots.push({
        config_id: configId,
        reason: reason,
        generation: config === null ? null : config.generation
    })
    return true
}

function flushOneBankSnapshot(activeDevice, activeMapping) {
    if (pendingBankSnapshots.length === 0) {
        return
    }

    var pending = pendingBankSnapshots.shift()
    var config = findBankConfig(pending.config_id)
    if (config === null) {
        return
    }

    if (pending.generation !== config.generation) {
        sendChunkedEvent(
            activeDevice,
            'probe.bank.chunk',
            'mixer_bank_snapshot',
            pending.reason,
            [],
            true,
            {
                config_id: config.id,
                follow_visibility: config.follow_visibility,
                requested_bank_generation: pending.generation,
                bank_generation: config.generation,
                superseded: true
            }
        )
        return
    }

    var items = []
    for (var index = 0; index < config.slots.length; ++index) {
        var slot = config.slots[index]
        items.push(bankSlotItem(config, slot))
    }

    sendChunkedEvent(
        activeDevice,
        'probe.bank.chunk',
        'mixer_bank_snapshot',
        pending.reason,
        items,
        false,
        {
            config_id: config.id,
            follow_visibility: config.follow_visibility,
            requested_bank_generation: pending.generation,
            bank_generation: config.generation,
            superseded: false
        }
    )
}

function findBankConfig(configId) {
    for (var index = 0; index < bankConfigs.length; ++index) {
        if (bankConfigs[index].id === configId) {
            return bankConfigs[index]
        }
    }
    return null
}

function makeDirectAccess() {
    var capabilities = {
        supported: false,
        active: false,
        get_object_unique_name_v1_2: false,
        get_object_unique_id_string_v1_2: false,
        get_object_title_v1_2: false,
        get_object_type_name_v1_3: false,
        mixer_visibility_v1_2: false,
        mixer_index_v1_2: false,
        mixer_zone_v1_2: false,
        reason: 'make_direct_access_unavailable'
    }

    if (typeof page.mHostAccess.makeDirectAccess !== 'function') {
        return capabilities
    }

    var candidate = null
    try {
        candidate = page.mHostAccess.makeDirectAccess(mixConsole)
    } catch (error) {
        capabilities.reason = 'make_direct_access_failed'
        return capabilities
    }

    if (
        !candidate ||
        typeof candidate.activate !== 'function' ||
        typeof candidate.update !== 'function' ||
        typeof candidate.deactivate !== 'function' ||
        typeof candidate.getBaseObjectID !== 'function' ||
        typeof candidate.getNumberOfChildObjects !== 'function' ||
        typeof candidate.getChildObjectID !== 'function'
    ) {
        capabilities.reason = 'core_methods_incomplete'
        return capabilities
    }

    directAccess = candidate
    capabilities.supported = true
    capabilities.get_object_unique_name_v1_2 =
        typeof candidate.getObjectUniqueName === 'function'
    capabilities.get_object_unique_id_string_v1_2 =
        typeof candidate.getObjectUniqueIDString === 'function'
    capabilities.get_object_title_v1_2 =
        typeof candidate.getObjectTitle === 'function'
    capabilities.get_object_type_name_v1_3 =
        typeof candidate.getObjectTypeName === 'function'
    capabilities.mixer_visibility_v1_2 =
        typeof candidate.isMixerChannelVisible === 'function'
    capabilities.mixer_index_v1_2 =
        typeof candidate.getMixerChannelIndex === 'function'
    capabilities.mixer_zone_v1_2 =
        typeof candidate.getMixerChannelZone === 'function'
    capabilities.reason = null

    candidate.mOnObjectChange = function (activeDevice, activeMapping, objectId) {
        recordDirectAccessChange('object_change', objectId, null)
        if (pageIsReady && !directAccessUpdateInProgress) {
            scheduleDirectAccessSnapshotOnce('object_change')
        }
    }
    candidate.mOnObjectWillBeRemoved = function (activeDevice, activeMapping, objectId) {
        recordDirectAccessChange('object_will_be_removed', objectId, null)
        if (pageIsReady && !directAccessUpdateInProgress) {
            scheduleDirectAccessSnapshotOnce('object_will_be_removed')
        }
    }
    candidate.mOnParameterChange = function (
        activeDevice,
        activeMapping,
        objectId,
        parameterTag
    ) {
        recordDirectAccessChange('parameter_change', objectId, parameterTag)
        if (pageIsReady && !directAccessUpdateInProgress) {
            scheduleDirectAccessSnapshotOnce('parameter_change')
        }
    }

    return capabilities
}

function activateDirectAccess(activeDevice, activeMapping) {
    if (!directAccessCapabilities.supported || directAccess === null) {
        return
    }

    try {
        directAccess.activate(activeMapping)
        directAccessActive = true
        directAccessCapabilities.active = true
        directAccessActivationError = null
    } catch (error) {
        directAccessActive = false
        directAccessCapabilities.active = false
        directAccessActivationError = 'activate_failed'
        sendEvent(activeDevice, 'probe.direct_access.error', {
            operation: 'activate',
            error_code: directAccessActivationError
        })
    }
}

function deactivateDirectAccess(activeDevice, activeMapping) {
    if (!directAccessActive || directAccess === null || activeMapping === null) {
        return
    }

    try {
        directAccess.deactivate(activeMapping)
    } catch (error) {
        sendOverflow(activeDevice, {
            stream: 'direct_access_deactivate',
            error_code: 'deactivate_failed'
        })
    }
    directAccessActive = false
    directAccessCapabilities.active = false
}

function recordDirectAccessChange(kind, objectId, parameterTag) {
    if (!pageIsActive) {
        if (pageActivationSeen) {
            reportPostDeactivationCallback({
                stream: 'post_deactivation_callback',
                callback_source: 'direct_access',
                callback_kind: sanitizeString(kind),
                object_id: validNumberOrNull(objectId),
                parameter_tag: validNumberOrNull(parameterTag)
            })
        }
        return
    }

    var callbackEpoch = observationEpoch
    ++observationSequence
    var item = {
        observation_seq: observationSequence,
        observation_epoch: callbackEpoch,
        observation_epoch_status: 'callback_observed',
        change: kind,
        object_id: validNumberOrNull(objectId),
        parameter_tag: validNumberOrNull(parameterTag)
    }

    enqueueFeedback('direct_access_feedback', 'probe.direct_access.chunk', item)
}

function reportPostDeactivationCallback(data) {
    probeIntegrityFailed = true
    pageIsReady = false
    if (postDeactivationCallbackReported) {
        return
    }
    postDeactivationCallbackReported = true
    sendOverflow(activeDeviceRef, data)
}

function scheduleDirectAccessSnapshot(reason) {
    if (pendingDirectAccessSnapshots.length >= MAX_PENDING_DIRECT_ACCESS_SNAPSHOTS) {
        return false
    }
    pendingDirectAccessSnapshots.push({
        reason: reason,
        updated: false
    })
    return true
}

function scheduleDirectAccessSnapshotOnce(reason) {
    for (var index = 0; index < pendingDirectAccessSnapshots.length; ++index) {
        if (pendingDirectAccessSnapshots[index].reason === reason) {
            return
        }
    }
    if (!scheduleDirectAccessSnapshot(reason)) {
        sendOverflow(activeDeviceRef, {
            stream: 'direct_access_snapshot_queue',
            pending_items: pendingDirectAccessSnapshots.length,
            queue_limit: MAX_PENDING_DIRECT_ACCESS_SNAPSHOTS,
            attempted_reason: sanitizeString(reason)
        })
    }
}

function hasActivationSnapshotPending() {
    var index = 0
    for (index = 0; index < pendingBankSnapshots.length; ++index) {
        if (pendingBankSnapshots[index].reason === 'page_activate') {
            return true
        }
    }
    for (index = 0; index < pendingDirectAccessSnapshots.length; ++index) {
        if (pendingDirectAccessSnapshots[index].reason === 'page_activate') {
            return true
        }
    }
    return false
}

function maybeFinishActivation(activeDevice) {
    if (
        !activationInitializationPending ||
        feedbackIsPending() ||
        hasActivationSnapshotPending()
    ) {
        return
    }

    activationInitializationPending = false
    if (probeIntegrityFailed) {
        pageIsReady = false
        sendEvent(activeDevice, 'probe.ready', {
            ready: false,
            probe_session_id: probeInstanceId,
            read_only: true,
            protocol_version: PROBE_PROTOCOL_VERSION,
            initial_snapshots_complete: false,
            initialization_failed: true
        })
        return
    }
    pageIsReady = true
    sendEvent(activeDevice, 'probe.ready', {
        ready: true,
        probe_session_id: probeInstanceId,
        read_only: true,
        protocol_version: PROBE_PROTOCOL_VERSION,
        initial_snapshots_complete: true
    })
}

function flushOneDirectAccessSnapshot(activeDevice, activeMapping) {
    if (pendingDirectAccessSnapshots.length === 0 || !directAccessActive) {
        return
    }

    // Keep the entry in the queue while update runs. A synchronous callback
    // can then de-duplicate its follow-up reason against this in-flight entry.
    // More importantly, draining feedback on a later idle must not run update
    // for the same snapshot a second time.
    var pending = pendingDirectAccessSnapshots[0]
    if (!pending.updated) {
        pending.updated = true
        directAccessUpdateInProgress = true
        try {
            directAccess.update(activeMapping)
        } catch (error) {
            directAccessUpdateInProgress = false
            pendingDirectAccessSnapshots.shift()
            sendOverflow(activeDevice, {
                stream: 'direct_access_update',
                attempted_reason: pending.reason,
                error_code: 'update_failed'
            })
            return
        }
        directAccessUpdateInProgress = false
    }

    if (feedbackIsPending()) {
        return
    }

    pendingDirectAccessSnapshots.shift()
    var result = collectDirectAccess(activeMapping)
    sendChunkedEvent(
        activeDevice,
        'probe.direct_access.chunk',
        'direct_access_snapshot',
        pending.reason,
        result.items,
        result.truncated,
        {
            base_object_id: result.base_object_id,
            observation_epoch: result.observation_epoch,
            observation_epoch_status: result.observation_epoch_status,
            observation_items: result.observation_items,
            reference_items: result.reference_items,
            cycle_count: result.cycle_count,
            shared_reference_count: result.shared_reference_count,
            error_count: result.error_count,
            truncation_reasons: result.truncation_reasons
        }
    )
}

function collectDirectAccess(activeMapping) {
    var snapshotEpoch = observationEpoch
    var result = {
        base_object_id: null,
        items: [],
        observation_epoch: snapshotEpoch,
        observation_epoch_status: 'snapshot_observed',
        observation_items: 0,
        reference_items: 0,
        cycle_count: 0,
        shared_reference_count: 0,
        error_count: 0,
        truncated: false,
        truncation_reasons: []
    }

    var baseObjectId = null
    try {
        baseObjectId = directAccess.getBaseObjectID(activeMapping)
    } catch (error) {
        result.error_count = 1
        result.truncated = true
        result.truncation_reasons.push('get_base_object_id_failed')
        return result
    }

    if (!validObjectId(baseObjectId)) {
        result.error_count = 1
        result.truncated = true
        result.truncation_reasons.push('invalid_base_object_id')
        return result
    }

    result.base_object_id = baseObjectId
    var stack = [{
        frame_kind: 'enter',
        object_id: baseObjectId,
        parent_id: null,
        depth: 0,
        child_index: null
    }]
    var visited = {}
    var activeAncestors = {}

    while (stack.length > 0) {
        var frame = stack.pop()
        if (frame.frame_kind === 'leave') {
            delete activeAncestors[frame.object_key]
            continue
        }

        if (result.items.length >= MAX_DIRECT_ACCESS_NODES) {
            result.truncated = true
            addUniqueReason(result.truncation_reasons, 'node_limit')
            break
        }

        var entry = frame
        var key = '$' + String(entry.object_id)
        if (Object.prototype.hasOwnProperty.call(visited, key)) {
            var referenceKind = 'shared_reference'
            if (activeAncestors[key] === true) {
                referenceKind = 'ancestor_cycle'
                ++result.cycle_count
            } else {
                ++result.shared_reference_count
            }
            ++result.reference_items
            result.items.push({
                record_kind: 'object_reference',
                observation_epoch: snapshotEpoch,
                observation_epoch_status: 'snapshot_observed',
                reference_kind: referenceKind,
                object_id: entry.object_id,
                parent_id: entry.parent_id,
                depth: entry.depth,
                child_index: entry.child_index,
                target_observation_index: visited[key]
            })
            continue
        }

        visited[key] = result.observation_items
        activeAncestors[key] = true
        ++result.observation_items

        var item = readDirectAccessObject(activeMapping, entry, snapshotEpoch)
        result.error_count += item.metadata_error_count
        result.items.push(item)
        stack.push({ frame_kind: 'leave', object_key: key })

        var childCount = 0
        try {
            childCount = directAccess.getNumberOfChildObjects(
                activeMapping,
                entry.object_id
            )
        } catch (error) {
            ++result.error_count
            item.child_count = null
            item.child_enumeration_error = 'get_number_of_child_objects_failed'
            result.truncated = true
            addUniqueReason(result.truncation_reasons, 'child_count_failed')
            continue
        }

        if (!validNonnegativeInteger(childCount)) {
            ++result.error_count
            item.child_count = null
            item.child_enumeration_error = 'invalid_child_count'
            result.truncated = true
            addUniqueReason(result.truncation_reasons, 'invalid_child_count')
            continue
        }

        item.child_count = childCount
        if (entry.depth >= MAX_DIRECT_ACCESS_DEPTH) {
            if (childCount > 0) {
                result.truncated = true
                addUniqueReason(result.truncation_reasons, 'depth_limit')
            }
            continue
        }

        var visitedChildCount = childCount
        if (visitedChildCount > MAX_DIRECT_ACCESS_CHILDREN) {
            visitedChildCount = MAX_DIRECT_ACCESS_CHILDREN
            result.truncated = true
            addUniqueReason(result.truncation_reasons, 'child_count_limit')
        }

        for (var childIndex = visitedChildCount - 1; childIndex >= 0; --childIndex) {
            var childObjectId = null
            try {
                childObjectId = directAccess.getChildObjectID(
                    activeMapping,
                    entry.object_id,
                    childIndex
                )
            } catch (error) {
                ++result.error_count
                result.truncated = true
                addUniqueReason(result.truncation_reasons, 'child_object_id_failed')
                continue
            }

            if (!validObjectId(childObjectId)) {
                ++result.error_count
                result.truncated = true
                addUniqueReason(result.truncation_reasons, 'invalid_child_object_id')
                continue
            }
            stack.push({
                frame_kind: 'enter',
                object_id: childObjectId,
                parent_id: entry.object_id,
                depth: entry.depth + 1,
                child_index: childIndex
            })
        }
    }

    return result
}

function readDirectAccessObject(activeMapping, entry, snapshotEpoch) {
    var errors = []
    var titleResult = readDirectAccessStringResult(
        'getObjectTitle',
        activeMapping,
        entry.object_id,
        errors
    )
    var titleAllowed =
        titleResult.value !== null && fixtureTitleIsAllowed(titleResult.value)
    var hostId = null
    if (titleAllowed) {
        hostId = readDirectAccessOpaqueId(
            'getObjectUniqueIDString',
            activeMapping,
            entry.object_id,
            errors
        )
    }
    var uniqueNameAvailable =
        typeof directAccess.getObjectUniqueName === 'function'
    var uniqueNameRedacted = uniqueNameAvailable
    var titleRedacted = titleResult.value !== null && !titleAllowed
    var hostIdRedacted =
        titleRedacted && typeof directAccess.getObjectUniqueIDString === 'function'
    var item = {
        record_kind: 'observation',
        observation_epoch: snapshotEpoch,
        observation_epoch_status: 'snapshot_observed',
        object_id: entry.object_id,
        parent_id: entry.parent_id,
        depth: entry.depth,
        child_index: entry.child_index,
        unique_name: null,
        unique_name_redacted: uniqueNameRedacted,
        unique_name_status:
            uniqueNameAvailable ? 'not_invoked_by_policy' : 'not_available',
        host_id_raw: hostId,
        host_id_redacted: hostIdRedacted,
        title: titleAllowed ? titleResult.value : null,
        title_redacted: titleRedacted,
        type_name: null,
        type_name_redacted: false,
        mixer_visible: readDirectAccessBoolean(
            'isMixerChannelVisible',
            activeMapping,
            entry.object_id,
            errors
        ),
        mixer_index: readDirectAccessNumber(
            'getMixerChannelIndex',
            activeMapping,
            entry.object_id,
            errors
        ),
        mixer_zone: readDirectAccessNumber(
            'getMixerChannelZone',
            activeMapping,
            entry.object_id,
            errors
        ),
        child_count: null,
        child_enumeration_error: null,
        metadata_error_count: 0,
        metadata_errors: [],
        redacted_string_count: 0
    }

    // getObjectTypeName is API v1.3. Do not treat it as part of the v1.2
    // DirectAccess metadata baseline.
    if (directAccessCapabilities.get_object_type_name_v1_3) {
        var typeResult = readDirectAccessStringResult(
            'getObjectTypeName',
            activeMapping,
            entry.object_id,
            errors
        )
        if (
            typeResult.value !== null &&
            safeDirectAccessTypeName(typeResult.value)
        ) {
            item.type_name = typeResult.value
        } else if (typeResult.value !== null) {
            item.type_name_redacted = true
        }
    }

    item.redacted_string_count =
        (item.unique_name_redacted ? 1 : 0) +
        (item.host_id_redacted ? 1 : 0) +
        (item.title_redacted ? 1 : 0) +
        (item.type_name_redacted ? 1 : 0)
    item.metadata_error_count = errors.length
    item.metadata_errors = errors
    return item
}

function readDirectAccessStringResult(methodName, activeMapping, objectId, errors) {
    var result = { value: null }
    if (typeof directAccess[methodName] !== 'function') {
        return result
    }
    try {
        var value = directAccess[methodName](activeMapping, objectId)
        if (typeof value !== 'string') {
            errors.push(methodName + '_invalid_string')
            return result
        }
        result.value = value
        return result
    } catch (error) {
        errors.push(methodName + '_failed')
        return result
    }
}

function readDirectAccessOpaqueId(methodName, activeMapping, objectId, errors) {
    if (typeof directAccess[methodName] !== 'function') {
        return null
    }
    try {
        var value = directAccess[methodName](activeMapping, objectId)
        if (typeof value !== 'string') {
            errors.push(methodName + '_invalid_string')
            return null
        }
        return value
    } catch (error) {
        errors.push(methodName + '_failed')
        return null
    }
}

function readDirectAccessBoolean(methodName, activeMapping, objectId, errors) {
    if (typeof directAccess[methodName] !== 'function') {
        return null
    }
    try {
        var value = directAccess[methodName](activeMapping, objectId)
        if (typeof value !== 'boolean') {
            errors.push(methodName + '_invalid_boolean')
            return null
        }
        return value
    } catch (error) {
        errors.push(methodName + '_failed')
        return null
    }
}

function readDirectAccessNumber(methodName, activeMapping, objectId, errors) {
    if (typeof directAccess[methodName] !== 'function') {
        return null
    }
    try {
        var value = directAccess[methodName](activeMapping, objectId)
        if (typeof value !== 'number' || !isFinite(value)) {
            errors.push(methodName + '_invalid_number')
            return null
        }
        return value
    } catch (error) {
        errors.push(methodName + '_failed')
        return null
    }
}

function fixtureTitleIsAllowed(value) {
    if (typeof value !== 'string' || value.length === 0) {
        return false
    }
    for (var index = 0; index < FIXTURE_TITLE_ALLOWLIST.length; ++index) {
        if (FIXTURE_TITLE_ALLOWLIST[index] === value) {
            return true
        }
    }
    return (
        value.length >= FIXTURE_P09_PREFIX.length &&
        value.indexOf(FIXTURE_P09_PREFIX) === 0 &&
        FIXTURE_P09_TITLE.indexOf(value) === 0
    )
}

function safeDirectAccessTypeName(value) {
    for (var index = 0; index < SAFE_DIRECT_ACCESS_TYPE_NAMES.length; ++index) {
        if (SAFE_DIRECT_ACCESS_TYPE_NAMES[index] === value) {
            return true
        }
    }
    return false
}

function addUniqueReason(reasons, reason) {
    for (var index = 0; index < reasons.length; ++index) {
        if (reasons[index] === reason) {
            return
        }
    }
    reasons.push(reason)
}

function handleRequest(activeDevice, request) {
    if (request.version !== 1 || request.type !== 'request') {
        sendError(activeDevice, request.id, 'PROTOCOL_ERROR', 'Invalid probe request')
        return
    }
    if (
        request.params === null ||
        typeof request.params !== 'object' ||
        objectIsArray(request.params)
    ) {
        sendError(activeDevice, request.id, 'INVALID_ARGUMENT', 'params must be an object')
        return
    }

    if (request.method === 'probe.discover') {
        sendResponse(activeDevice, request.id, {
            instance_id: probeInstanceId,
            ready: pageIsReady,
            read_only: true
        })
        return
    }

    if (request.method === 'probe.capabilities.get') {
        sendResponse(activeDevice, request.id, capabilityData())
        return
    }

    if (!pageIsActive || activeMappingRef === null) {
        sendError(activeDevice, request.id, 'NOT_CONNECTED', 'Probe mapping is inactive')
        return
    }
    if (probeIntegrityFailed) {
        sendError(activeDevice, request.id, 'PROTOCOL_ERROR', 'Probe integrity failed; reload required')
        return
    }
    if (!pageIsReady) {
        sendError(activeDevice, request.id, 'BUSY', 'Initial probe snapshots are pending')
        return
    }

    if (request.method === 'probe.observation.cut') {
        handleObservationCut(activeDevice, request)
        return
    }

    if (
        request.method === 'probe.bank.reset' ||
        request.method === 'probe.bank.next' ||
        request.method === 'probe.bank.prev'
    ) {
        handleBankAction(activeDevice, request)
        return
    }

    if (request.method === 'probe.bank.snapshot') {
        var snapshotConfig = requestConfig(request)
        if (snapshotConfig === null) {
            sendError(activeDevice, request.id, 'INVALID_ARGUMENT', 'Unknown config_id')
            return
        }
        if (!scheduleBankSnapshot(snapshotConfig.id, 'command_snapshot')) {
            sendError(activeDevice, request.id, 'BUSY', 'Bank snapshot queue is full')
            return
        }
        sendResponse(activeDevice, request.id, { config_id: snapshotConfig.id })
        return
    }

    if (request.method === 'probe.direct_access.snapshot') {
        if (!directAccessCapabilities.supported || !directAccessActive) {
            sendError(activeDevice, request.id, 'NOT_SUPPORTED', 'DirectAccess is not available')
            return
        }
        if (!scheduleDirectAccessSnapshot('command_snapshot')) {
            sendError(activeDevice, request.id, 'BUSY', 'DirectAccess snapshot queue is full')
            return
        }
        sendResponse(activeDevice, request.id, {})
        return
    }

    sendError(activeDevice, request.id, 'NOT_SUPPORTED', 'Unknown probe method')
}

function handleObservationCut(activeDevice, request) {
    if (!objectHasNoOwnProperties(request.params)) {
        sendError(
            activeDevice,
            request.id,
            'INVALID_ARGUMENT',
            'observation cut params must be empty'
        )
        return
    }
    if (
        !validNonnegativeInteger(observationEpoch) ||
        observationEpoch >= MAX_OBSERVATION_EPOCH
    ) {
        sendOverflow(activeDevice, {
            stream: 'observation_epoch',
            error_code: 'epoch_exhausted',
            observation_epoch: validNumberOrNull(observationEpoch),
            max_observation_epoch: MAX_OBSERVATION_EPOCH,
            rollover_policy: 'reload_required'
        })
        sendError(
            activeDevice,
            request.id,
            'PROTOCOL_ERROR',
            'Observation epoch exhausted; reload required'
        )
        return
    }
    ++observationEpoch
    sendResponse(activeDevice, request.id, {
        observation_epoch: observationEpoch
    })
}

function handleBankAction(activeDevice, request) {
    var config = requestConfig(request)
    if (config === null) {
        sendError(activeDevice, request.id, 'INVALID_ARGUMENT', 'Unknown config_id')
        return
    }

    var action = null
    var actionName = null
    if (request.method === 'probe.bank.reset') {
        action = config.zone.mAction.mResetBank
        actionName = 'reset'
    } else if (request.method === 'probe.bank.next') {
        action = config.zone.mAction.mNextBank
        actionName = 'next'
    } else {
        action = config.zone.mAction.mPrevBank
        actionName = 'prev'
    }

    if (!action || typeof action.trigger !== 'function') {
        sendError(activeDevice, request.id, 'NOT_SUPPORTED', 'Bank action is not available')
        return
    }
    if (pendingBankSnapshots.length >= MAX_PENDING_BANK_SNAPSHOTS) {
        sendError(activeDevice, request.id, 'BUSY', 'Bank snapshot queue is full')
        return
    }

    beginBankGeneration(config)
    try {
        action.trigger(activeMappingRef)
    } catch (error) {
        sendOverflow(activeDevice, {
            stream: 'bank_action',
            config_id: config.id,
            action: actionName,
            error_code: 'bank_action_failed'
        })
        sendError(activeDevice, request.id, 'INTERNAL_ERROR', 'Bank action failed; reload required')
        return
    }
    if (!scheduleBankSnapshot(config.id, 'command_' + actionName)) {
        sendOverflow(activeDevice, {
            stream: 'bank_snapshot_queue',
            config_id: config.id,
            action: actionName,
            queue_limit: MAX_PENDING_BANK_SNAPSHOTS
        })
        sendError(activeDevice, request.id, 'BUSY', 'Bank snapshot queue is full')
        return
    }
    sendResponse(activeDevice, request.id, {
        config_id: config.id,
        action: actionName
    })
}

function requestConfig(request) {
    if (!request.params || typeof request.params.config_id !== 'string') {
        return null
    }
    return findBankConfig(request.params.config_id)
}

function capabilityData() {
    var bankUniqueId = false
    var configIds = []
    if (bankConfigs.length > 0 && bankConfigs[0].slots.length > 0) {
        bankUniqueId =
            typeof bankConfigs[0].slots[0].channel.getUniqueIDString === 'function'
    }
    for (var configIndex = 0; configIndex < bankConfigs.length; ++configIndex) {
        configIds.push(bankConfigs[configIndex].id)
    }

    return {
        read_only: true,
        integrity_failed: probeIntegrityFailed,
        host_version: getHostVersion(),
        data_minimization: {
            source_redaction: true,
            fixture_revision: 2,
            unknown_titles: 'redacted',
            unknown_host_ids: 'omitted',
            unique_name_policy: 'not_invoked',
            exception_text: 'fixed_codes'
        },
        observation_epoch: {
            supported: true,
            version: 1,
            current: observationEpoch,
            max: MAX_OBSERVATION_EPOCH,
            rollover_policy: 'reload_required'
        },
        mixer_bank: {
            supported: true,
            slot_count: BANK_SLOT_COUNT,
            configs: configIds,
            title: true,
            selected: true,
            mute: true,
            solo: true,
            unique_id: bankUniqueId,
            explicit_main_filter: bankConfigs[0].explicit_main_filter
        },
        direct_access: {
            supported: directAccessCapabilities.supported,
            active: directAccessCapabilities.active,
            activation_error: directAccessActivationError,
            get_object_unique_name_v1_2:
                directAccessCapabilities.get_object_unique_name_v1_2,
            get_object_unique_id_string_v1_2:
                directAccessCapabilities.get_object_unique_id_string_v1_2,
            get_object_title_v1_2: directAccessCapabilities.get_object_title_v1_2,
            get_object_type_name_v1_3:
                directAccessCapabilities.get_object_type_name_v1_3,
            mixer_visibility_v1_2: directAccessCapabilities.mixer_visibility_v1_2,
            mixer_index_v1_2: directAccessCapabilities.mixer_index_v1_2,
            mixer_zone_v1_2: directAccessCapabilities.mixer_zone_v1_2,
            reason: directAccessCapabilities.reason
        },
        limits: {
            output_json_bytes: MAX_OUTPUT_JSON_BYTES,
            chunk_items: MAX_ITEMS_PER_CHUNK,
            feedback_queue: MAX_PENDING_FEEDBACK,
            bank_snapshot_queue: MAX_PENDING_BANK_SNAPSHOTS,
            direct_access_snapshot_queue: MAX_PENDING_DIRECT_ACCESS_SNAPSHOTS,
            host_id_bytes: MAX_HOST_ID_BYTES,
            host_id_fragments: MAX_HOST_ID_FRAGMENTS,
            wire_items_per_snapshot: MAX_WIRE_ITEMS_PER_SNAPSHOT,
            direct_access_nodes: MAX_DIRECT_ACCESS_NODES,
            direct_access_depth: MAX_DIRECT_ACCESS_DEPTH,
            direct_access_children: MAX_DIRECT_ACCESS_CHILDREN
        }
    }
}

function getHostVersion() {
    if (
        !midiremote_api.mDefaults ||
        !midiremote_api.mDefaults.mAppVersion ||
        typeof midiremote_api.mDefaults.mAppVersion.getVersionString !== 'function'
    ) {
        return null
    }
    try {
        return sanitizeNullableString(
            midiremote_api.mDefaults.mAppVersion.getVersionString()
        )
    } catch (error) {
        return null
    }
}

function sendCapabilities(activeDevice) {
    sendEvent(activeDevice, 'probe.capabilities', capabilityData())
}

function deactivatePage(activeDevice, activeMapping) {
    if (!pageIsActive) {
        activeMappingRef = null
        return
    }

    deactivateDirectAccess(activeDevice, activeMapping)
    // Cubase can synchronously enqueue more than one idle batch while a
    // project-activation dialog blocks mOnIdle. Preserve that evidence before
    // ready(false); callbacks emitted after the inactive boundary are invalid.
    flushFeedbackBeforeDeactivation(activeDevice)
    // Coalesced change snapshots describe the mapping that is now being torn
    // down. Their underlying callbacks were drained above and the next mapping
    // emits a complete page_activate snapshot. Command and activation work is
    // deliberately retained so the fail-closed discard check still catches it.
    cancelInvalidatedDirectAccessSnapshots()
    var initialSnapshotsComplete = pageIsReady
    var discardedItems =
        pendingFeedback.length +
        pendingDroppedFeedback.total +
        pendingBankSnapshots.length +
        pendingDirectAccessSnapshots.length
    if (discardedItems > 0) {
        sendOverflow(activeDevice, {
            stream: 'deactivation_discard',
            discarded_items: discardedItems,
            pending_feedback_items: pendingFeedback.length,
            already_dropped_feedback_items: pendingDroppedFeedback.total,
            pending_bank_snapshots: pendingBankSnapshots.length,
            pending_direct_access_snapshots: pendingDirectAccessSnapshots.length
        })
    }
    sendEvent(activeDevice, 'probe.ready', {
        ready: false,
        probe_session_id: probeInstanceId,
        read_only: true,
        protocol_version: PROBE_PROTOCOL_VERSION,
        initial_snapshots_complete: initialSnapshotsComplete
    })

    pageIsActive = false
    pageIsReady = false
    activationInitializationPending = false
    activeMappingRef = null
    pendingFeedback = []
    pendingDroppedFeedback = makeDroppedFeedbackState()
    pendingBankSnapshots = []
    pendingDirectAccessSnapshots = []
    directAccessUpdateInProgress = false
}

function sendChunkedEvent(
    activeDevice,
    eventName,
    stream,
    reason,
    items,
    truncated,
    extra
) {
    var snapshotId = nextSnapshotId()
    var chunkExtra = {}
    copyOwnProperties(chunkExtra, extra)
    if (!Object.prototype.hasOwnProperty.call(chunkExtra, 'observation_items')) {
        chunkExtra.observation_items = items.length
    }
    var expansion = expandHostIdsForWire(
        snapshotId,
        eventName,
        stream,
        reason,
        items,
        truncated,
        chunkExtra
    )
    if (expansion.overflow !== null) {
        sendOverflow(activeDevice, expansion.overflow)
        return
    }
    var wireItems = expansion.items
    if (wireItems.length > MAX_WIRE_ITEMS_PER_SNAPSHOT) {
        sendOverflow(activeDevice, {
            stream: 'snapshot_wire_items',
            attempted_event: sanitizeString(eventName),
            snapshot_id: snapshotId,
            attempted_items: wireItems.length,
            max_items: MAX_WIRE_ITEMS_PER_SNAPSHOT
        })
        return
    }
    var itemChunks = splitChunkItemsForFrame(
        eventName,
        snapshotId,
        stream,
        reason,
        wireItems,
        truncated,
        chunkExtra
    )
    if (itemChunks === null) {
        sendOverflow(activeDevice, {
            stream: 'outbound_item',
            attempted_event: sanitizeString(eventName),
            snapshot_id: snapshotId,
            max_json_bytes: MAX_OUTPUT_JSON_BYTES
        })
        return
    }
    var chunkCount = itemChunks.length

    for (var chunkIndex = 0; chunkIndex < chunkCount; ++chunkIndex) {
        var data = makeChunkData(
            snapshotId,
            stream,
            reason,
            chunkIndex,
            chunkCount,
            wireItems.length,
            itemChunks[chunkIndex],
            truncated,
            chunkExtra
        )
        sendEvent(activeDevice, eventName, data)
    }
}

function expandHostIdsForWire(
    snapshotId,
    eventName,
    stream,
    reason,
    items,
    truncated,
    extra
) {
    var expanded = []
    for (var index = 0; index < items.length; ++index) {
        var item = {}
        copyOwnProperties(item, items[index])
        if (!Object.prototype.hasOwnProperty.call(item, 'host_id_raw')) {
            expanded.push(item)
            continue
        }

        var rawId = item.host_id_raw
        if (typeof rawId !== 'string') {
            item.host_id_raw = null
            item.host_id_byte_length = null
            item.host_id_ref = null
            item.host_id_fragment_count = 0
            expanded.push(item)
            continue
        }

        var byteLength = utf8Encode(rawId).length
        if (byteLength > MAX_HOST_ID_BYTES) {
            return {
                items: [],
                overflow: {
                    stream: 'host_id',
                    attempted_event: sanitizeString(eventName),
                    snapshot_id: snapshotId,
                    item_index: index,
                    attempted_bytes: byteLength,
                    max_bytes: MAX_HOST_ID_BYTES
                }
            }
        }
        item.host_id_byte_length = byteLength
        item.host_id_ref = null
        item.host_id_fragment_count = 0
        if (
            byteLength <= MAX_INLINE_HOST_ID_BYTES &&
            wireItemFitsOutputFrame(
                eventName,
                snapshotId,
                stream,
                reason,
                item,
                truncated,
                extra
            )
        ) {
            expanded.push(item)
            continue
        }

        var idRef = snapshotId + '-item-' + index + '-host-id'
        var fragments = splitHostIdForFrames(
            rawId,
            byteLength,
            idRef,
            eventName,
            snapshotId,
            stream,
            reason,
            truncated,
            extra
        )
        if (fragments.length > MAX_HOST_ID_FRAGMENTS) {
            return {
                items: [],
                overflow: {
                    stream: 'host_id_fragments',
                    attempted_event: sanitizeString(eventName),
                    snapshot_id: snapshotId,
                    item_index: index,
                    attempted_fragments: fragments.length,
                    max_fragments: MAX_HOST_ID_FRAGMENTS
                }
            }
        }
        item.host_id_raw = null
        item.host_id_ref = idRef
        item.host_id_fragment_count = fragments.length
        expanded.push(item)

        for (var fragmentIndex = 0; fragmentIndex < fragments.length; ++fragmentIndex) {
            expanded.push(makeHostIdFragmentItem(
                idRef,
                byteLength,
                fragmentIndex,
                fragments.length,
                fragments[fragmentIndex]
            ))
        }
    }
    return { items: expanded, overflow: null }
}

function splitHostIdForFrames(
    value,
    byteLength,
    idRef,
    eventName,
    snapshotId,
    stream,
    reason,
    truncated,
    extra
) {
    var fragments = []
    var current = ''
    var currentBytes = 0
    for (var index = 0; index < value.length; ++index) {
        var character = value.charAt(index)
        var code = value.charCodeAt(index)
        if (code >= 0xD800 && code <= 0xDBFF && index + 1 < value.length) {
            var low = value.charCodeAt(index + 1)
            if (low >= 0xDC00 && low <= 0xDFFF) {
                character += value.charAt(index + 1)
                ++index
            }
        }

        var characterBytes = utf8Encode(character).length
        var candidate = current + character
        var candidateItem = makeHostIdFragmentItem(
            idRef,
            byteLength,
            999999,
            999999,
            candidate
        )
        var candidateFits =
            currentBytes + characterBytes <= MAX_HOST_ID_FRAGMENT_BYTES &&
            wireItemFitsOutputFrame(
                eventName,
                snapshotId,
                stream,
                reason,
                candidateItem,
                truncated,
                extra
            )
        if (current.length > 0 && !candidateFits) {
            fragments.push(current)
            current = character
            currentBytes = characterBytes
        } else {
            current = candidate
            currentBytes += characterBytes
        }
    }
    if (current.length > 0 || value.length === 0) {
        fragments.push(current)
    }
    return fragments
}

function makeHostIdFragmentItem(
    idRef,
    byteLength,
    fragmentIndex,
    fragmentCount,
    fragment
) {
    return {
        record_kind: 'host_id_fragment',
        host_id_ref: idRef,
        host_id_byte_length: byteLength,
        fragment_index: fragmentIndex,
        fragment_count: fragmentCount,
        fragment: fragment
    }
}

function wireItemFitsOutputFrame(
    eventName,
    snapshotId,
    stream,
    reason,
    item,
    truncated,
    extra
) {
    return chunkFitsOutputFrame(
        eventName,
        snapshotId,
        stream,
        reason,
        999999,
        [item],
        truncated,
        extra
    )
}

function splitChunkItemsForFrame(
    eventName,
    snapshotId,
    stream,
    reason,
    items,
    truncated,
    extra
) {
    if (items.length === 0) {
        return [[]]
    }

    var chunks = []
    var current = []
    for (var index = 0; index < items.length; ++index) {
        var candidate = current.slice(0)
        candidate.push(items[index])
        var candidateFits =
            candidate.length <= MAX_ITEMS_PER_CHUNK &&
            chunkFitsOutputFrame(
                eventName,
                snapshotId,
                stream,
                reason,
                items.length,
                candidate,
                truncated,
                extra
            )
        if (current.length > 0 && !candidateFits) {
            chunks.push(current)
            if (!wireItemFitsOutputFrame(
                eventName,
                snapshotId,
                stream,
                reason,
                items[index],
                truncated,
                extra
            )) {
                return null
            }
            current = [items[index]]
        } else if (current.length === 0 && !candidateFits) {
            return null
        } else {
            current = candidate
        }
    }
    if (current.length > 0) {
        chunks.push(current)
    }
    return chunks
}

function chunkFitsOutputFrame(
    eventName,
    snapshotId,
    stream,
    reason,
    totalItems,
    chunkItems,
    truncated,
    extra
) {
    // Six-digit chunk indexes and a long sequence value reserve more JSON
    // overhead than any bounded probe snapshot needs in practice.
    var data = makeChunkData(
        snapshotId,
        stream,
        reason,
        999999,
        999999,
        totalItems,
        chunkItems,
        truncated,
        extra
    )
    var envelope = {
        probe_transport_version: PROBE_PROTOCOL_VERSION,
        source_instance_id: probeInstanceId,
        source_seq: 999999999999999,
        message: {
            version: 1,
            type: 'event',
            event: eventName,
            data: data
        }
    }
    return utf8Encode(JSON.stringify(envelope)).length <= MAX_OUTPUT_JSON_BYTES
}

function makeChunkData(
    snapshotId,
    stream,
    reason,
    chunkIndex,
    chunkCount,
    totalItems,
    chunkItems,
    truncated,
    extra
) {
    var data = {
        snapshot_id: snapshotId,
        stream: stream,
        reason: reason,
        chunk_index: chunkIndex,
        chunk_count: chunkCount,
        total_items: totalItems,
        items: chunkItems,
        snapshot_complete: chunkIndex === chunkCount - 1,
        truncated: truncated === true,
        overflow_safe: true
    }
    copyOwnProperties(data, extra)
    return data
}

function nextSnapshotId() {
    ++snapshotSequence
    return probeInstanceId + '-snapshot-' + snapshotSequence
}

function sendResponse(activeDevice, requestId, result) {
    sendMessage(activeDevice, {
        version: 1,
        id: requestId,
        type: 'response',
        result: result
    })
}

function sendError(activeDevice, requestId, code, message) {
    sendMessage(activeDevice, {
        version: 1,
        id: requestId,
        type: 'error',
        error: {
            code: code,
            message: sanitizeString(message)
        }
    })
}

function sendEvent(activeDevice, eventName, data) {
    if (activeDevice === null) {
        return
    }
    sendMessage(activeDevice, {
        version: 1,
        type: 'event',
        event: eventName,
        data: data
    })
}

function sendOverflow(activeDevice, data) {
    probeIntegrityFailed = true
    pageIsReady = false
    sendEvent(activeDevice, 'probe.overflow', data)
}

function sendMessage(activeDevice, messageValue) {
    var nextSequence = sourceSequence + 1
    var envelope = {
        probe_transport_version: PROBE_PROTOCOL_VERSION,
        source_instance_id: probeInstanceId,
        source_seq: nextSequence,
        message: messageValue
    }
    var text = JSON.stringify(envelope)
    var bytes = utf8Encode(text)

    if (bytes.length > MAX_OUTPUT_JSON_BYTES) {
        sendOversizeNotice(activeDevice, messageValue, bytes.length)
        return false
    }

    sourceSequence = nextSequence
    sendBytes(activeDevice, bytes)
    return true
}

function sendOversizeNotice(activeDevice, attemptedMessage, attemptedBytes) {
    probeIntegrityFailed = true
    pageIsReady = false
    var nextSequence = sourceSequence + 1
    var attemptedEvent = null
    if (attemptedMessage && typeof attemptedMessage.event === 'string') {
        attemptedEvent = sanitizeString(attemptedMessage.event)
    }
    var envelope = {
        probe_transport_version: PROBE_PROTOCOL_VERSION,
        source_instance_id: probeInstanceId,
        source_seq: nextSequence,
        message: {
            version: 1,
            type: 'event',
            event: 'probe.overflow',
            data: {
                stream: 'outbound_frame',
                attempted_event: attemptedEvent,
                attempted_json_bytes: attemptedBytes,
                max_json_bytes: MAX_OUTPUT_JSON_BYTES
            }
        }
    }
    var bytes = utf8Encode(JSON.stringify(envelope))
    if (bytes.length > MAX_OUTPUT_JSON_BYTES) {
        return
    }
    sourceSequence = nextSequence
    sendBytes(activeDevice, bytes)
}

function sendBytes(activeDevice, bytes) {
    var midiMessage = SYSEX_HEADER.slice(0)
    for (var index = 0; index < bytes.length; ++index) {
        midiMessage.push((bytes[index] >> 4) & 0x0F)
        midiMessage.push(bytes[index] & 0x0F)
    }
    midiMessage.push(0xF7)
    midiOutput.sendMidi(activeDevice, midiMessage)
}

function createInstanceId() {
    var timestamp = new Date().getTime().toString(36)
    var random = Math.floor(Math.random() * 0x7FFFFFFF).toString(36)
    return 'track-probe-' + timestamp + '-' + random
}

function decodeFrame(message) {
    if (!message || message.length < SYSEX_HEADER.length + 1) {
        return null
    }
    for (var headerIndex = 0; headerIndex < SYSEX_HEADER.length; ++headerIndex) {
        if (message[headerIndex] !== SYSEX_HEADER[headerIndex]) {
            return null
        }
    }
    if (message[message.length - 1] !== 0xF7) {
        return null
    }

    var payloadLength = message.length - SYSEX_HEADER.length - 1
    if (payloadLength % 2 !== 0 || payloadLength / 2 > MAX_INPUT_JSON_BYTES) {
        return null
    }

    var bytes = []
    for (var index = SYSEX_HEADER.length; index < message.length - 1; index += 2) {
        var high = message[index]
        var low = message[index + 1]
        if (high > 0x0F || low > 0x0F) {
            return null
        }
        bytes.push((high << 4) | low)
    }
    return utf8Decode(bytes)
}

function sanitizeNullableString(value) {
    if (typeof value !== 'string') {
        return null
    }
    return sanitizeString(value)
}

function sanitizeString(value) {
    var text = typeof value === 'string' ? value : String(value)
    var bytes = 0
    var result = ''
    for (var index = 0; index < text.length; ++index) {
        var character = text.charAt(index)
        var code = text.charCodeAt(index)
        if (code >= 0xD800 && code <= 0xDBFF && index + 1 < text.length) {
            var low = text.charCodeAt(index + 1)
            if (low >= 0xDC00 && low <= 0xDFFF) {
                character += text.charAt(index + 1)
                ++index
            }
        }

        var characterBytes = utf8Encode(character).length
        if (bytes + characterBytes > MAX_STRING_BYTES - 3) {
            return result + '...'
        }
        result += character
        bytes += characterBytes
    }
    return result
}

function validNumberOrNull(value) {
    if (typeof value === 'number' && isFinite(value)) {
        return value
    }
    return null
}

function validNonnegativeInteger(value) {
    return (
        typeof value === 'number' &&
        isFinite(value) &&
        value >= 0 &&
        value <= 9007199254740991 &&
        Math.floor(value) === value
    )
}

function validObjectId(value) {
    return validNonnegativeInteger(value)
}

function objectIsArray(value) {
    return Object.prototype.toString.call(value) === '[object Array]'
}

function objectHasNoOwnProperties(value) {
    for (var key in value) {
        if (Object.prototype.hasOwnProperty.call(value, key)) {
            return false
        }
    }
    return true
}

function copyOwnProperties(target, source) {
    if (!source) {
        return
    }
    for (var key in source) {
        if (Object.prototype.hasOwnProperty.call(source, key)) {
            target[key] = source[key]
        }
    }
}

function utf8Encode(text) {
    var bytes = []
    for (var index = 0; index < text.length; ++index) {
        var codePoint = text.charCodeAt(index)
        if (codePoint >= 0xD800 && codePoint <= 0xDBFF && index + 1 < text.length) {
            var lowSurrogate = text.charCodeAt(index + 1)
            if (lowSurrogate >= 0xDC00 && lowSurrogate <= 0xDFFF) {
                codePoint =
                    0x10000 + ((codePoint - 0xD800) << 10) + (lowSurrogate - 0xDC00)
                ++index
            }
        }

        if (codePoint < 0x80) {
            bytes.push(codePoint)
        } else if (codePoint < 0x800) {
            bytes.push(0xC0 | (codePoint >> 6))
            bytes.push(0x80 | (codePoint & 0x3F))
        } else if (codePoint < 0x10000) {
            bytes.push(0xE0 | (codePoint >> 12))
            bytes.push(0x80 | ((codePoint >> 6) & 0x3F))
            bytes.push(0x80 | (codePoint & 0x3F))
        } else {
            bytes.push(0xF0 | (codePoint >> 18))
            bytes.push(0x80 | ((codePoint >> 12) & 0x3F))
            bytes.push(0x80 | ((codePoint >> 6) & 0x3F))
            bytes.push(0x80 | (codePoint & 0x3F))
        }
    }
    return bytes
}

function utf8Decode(bytes) {
    var text = ''
    var index = 0
    while (index < bytes.length) {
        var first = bytes[index++]
        var codePoint = 0
        if (first < 0x80) {
            codePoint = first
        } else if ((first & 0xE0) === 0xC0 && index < bytes.length) {
            codePoint = ((first & 0x1F) << 6) | (bytes[index++] & 0x3F)
        } else if ((first & 0xF0) === 0xE0 && index + 1 < bytes.length) {
            codePoint =
                ((first & 0x0F) << 12) |
                ((bytes[index++] & 0x3F) << 6) |
                (bytes[index++] & 0x3F)
        } else if ((first & 0xF8) === 0xF0 && index + 2 < bytes.length) {
            codePoint =
                ((first & 0x07) << 18) |
                ((bytes[index++] & 0x3F) << 12) |
                ((bytes[index++] & 0x3F) << 6) |
                (bytes[index++] & 0x3F)
        } else {
            return null
        }

        if (codePoint <= 0xFFFF) {
            text += String.fromCharCode(codePoint)
        } else {
            codePoint -= 0x10000
            text += String.fromCharCode(0xD800 + (codePoint >> 10))
            text += String.fromCharCode(0xDC00 + (codePoint & 0x3FF))
        }
    }
    return text
}
