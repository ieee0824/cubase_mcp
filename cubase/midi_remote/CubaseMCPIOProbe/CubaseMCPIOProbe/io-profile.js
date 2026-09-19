'use strict'

// Metadata only. A requested filter, a slot title, or equal pages is not proof
// of channel type, channel identity, window zone, or complete enumeration.
var PROFILE = 'io-existing-v1'
var SLOT_COUNT = 8
var MAX_ALIASES = 256
var MAX_STRING_LENGTH = 4096
var MAX_PENDING_FEEDBACK = 128
var MAX_SEQUENCE = 2147483647

function fixedError(code) {
    var error = new Error(code)
    error.code = code
    return error
}

function immutable(value) {
    if (value && typeof value === 'object') {
        Object.keys(value).forEach(function (key) { immutable(value[key]) })
        Object.freeze(value)
    }
    return value
}

function createProfile(mixConsole, emit) {
    if (!mixConsole || typeof emit !== 'function') throw fixedError('INVALID_ARGUMENT')
    var activeDevice = null
    var activeMapping = null
    var active = false
    var activationEpoch = 0
    var observationId = 0
    var fatalError = null
    var titleAliases = []
    var hostAliases = []
    var pendingFeedback = []
    var delivering = false
    var configs = []

    function fail(code) {
        fatalError = code
        pendingFeedback = []
        throw fixedError(code)
    }

    function healthy(requireActive) {
        if (fatalError) throw fixedError(fatalError)
        if (requireActive && !active) throw fixedError('NOT_CONNECTED')
    }

    function increment(value) {
        if (value >= MAX_SEQUENCE) fail('OVERFLOW')
        return value + 1
    }

    // Raw strings stay only in these bounded, activation-local tables. No hash
    // or prefix of a user's title/ID leaves the module; alias equality is not ID.
    function alias(table, value, prefix) {
        if (value.length > MAX_STRING_LENGTH) fail('OVERFLOW')
        var index = table.indexOf(value)
        if (index < 0) {
            if (table.length >= MAX_ALIASES) fail('OVERFLOW')
            index = table.length
            table.push(value)
        }
        return prefix + (index + 1)
    }

    function publish(event, data) {
        if (fatalError) return
        if (pendingFeedback.length >= MAX_PENDING_FEEDBACK) fail('OVERFLOW')
        pendingFeedback.push(immutable({ event: event, data: data }))
        if (delivering) return
        delivering = true
        try {
            // Bound a reentrant producer even if it adds one item per emit.
            var count = 0
            while (pendingFeedback.length) {
                if (++count > MAX_PENDING_FEEDBACK) fail('OVERFLOW')
                var accepted
                try { accepted = emit(pendingFeedback.shift()) }
                catch (error) { fail('INTERNAL_ERROR') }
                if (accepted === false) fail('OVERFLOW')
                healthy(false)
            }
        } finally { delivering = false }
    }

    function call(object, method, argument) {
        if (!object || typeof object[method] !== 'function') throw fixedError('NOT_SUPPORTED')
        try { return object[method](argument) }
        catch (error) { throw fixedError('INTERNAL_ERROR') }
    }

    function blank(slot) {
        return {
            slot_index: slot.index,
            title_observed: false,
            title_state: 'unobserved',
            title_alias: null,
            host_id_supported: slot.idSupported,
            host_id_status: slot.idSupported ? 'unobserved' : 'unsupported',
            host_id_alias: null
        }
    }

    function copy(value) {
        var result = {}
        Object.keys(value).forEach(function (key) { result[key] = value[key] })
        return result
    }

    function installCallback(config, slot) {
        // Old callback references can be rejected. The host supplies no bank
        // generation identifier. A late delivery through the new handler cannot be
        // distinguished from new feedback and is not proof of callback origin.
        var generation = config.generation
        var epoch = activationEpoch
        slot.channel.mOnTitleChange = function (device, mapping, title) {
            if (fatalError) return
            try {
                var reason = !active ? 'inactive' :
                    device !== activeDevice || mapping !== activeMapping || epoch !== activationEpoch ?
                        'lifecycle' : generation !== config.generation ? 'generation' : null
                if (reason) {
                    publish('probe.io.stale_callback', { reason: reason })
                    return
                }
                if (typeof title !== 'string') fail('INTERNAL_ERROR')
                var titleAlias = title === '' ? null : alias(titleAliases, title, 'title-')
                observationId = increment(observationId)
                slot.state = blank(slot)
                slot.state.title_observed = true
                slot.state.title_state = title === '' ? 'empty' : 'nonempty'
                slot.state.title_alias = titleAlias
                publish('probe.io.feedback', {
                    config_id: config.id,
                    activation_epoch: activationEpoch,
                    generation: generation,
                    observation_id: observationId,
                    item: copy(slot.state)
                })
            } catch (error) {
                // Host callbacks cannot propagate host/emitter exception text.
                if (!fatalError) fatalError = 'INTERNAL_ERROR'
            }
        }
    }

    function reset(config) {
        config.generation = increment(config.generation)
        config.slots.forEach(function (slot) {
            slot.state = blank(slot)
            installCallback(config, slot)
        })
    }

    function makeConfig(configId, kind) {
        var zone = call(mixConsole, 'makeMixerBankZone', configId)
        var kinds = ['Audio', 'Instrument', 'Sampler', 'MIDI', 'FX', 'Group', 'VCA', 'Input', 'Output']
        kinds.forEach(function (candidate) {
            call(zone, (candidate === kind ? 'include' : 'exclude') + candidate + 'Channels')
        })
        call(zone, 'includeWindowZoneLeftChannels')
        call(zone, 'includeWindowZoneRightChannels')
        var main = typeof zone.includeWindowZoneMainChannels === 'function'
        if (main) call(zone, 'includeWindowZoneMainChannels')
        call(zone, 'setFollowVisibility', false)
        var config = { id: configId, kind: kind.toLowerCase(), zone: zone,
            explicitMain: main, generation: 0, slots: [] }
        for (var index = 0; index < SLOT_COUNT; ++index) {
            var channel = call(zone, 'makeMixerBankChannel')
            if (!channel) throw fixedError('INTERNAL_ERROR')
            var slot = { index: index, channel: channel,
                idSupported: typeof channel.getUniqueIDString === 'function' }
            slot.state = blank(slot)
            installCallback(config, slot)
            config.slots.push(slot)
        }
        return config
    }

    configs.push(makeConfig('IO_INPUT_ALL', 'Input'))
    configs.push(makeConfig('IO_OUTPUT_ALL', 'Output'))

    function lookup(id) {
        for (var i = 0; i < configs.length; ++i) if (configs[i].id === id) return configs[i]
        throw fixedError('INVALID_ARGUMENT')
    }

    function hasAction(config, action) {
        var actions = config.zone.mAction
        return !!(actions && actions[action] && typeof actions[action].trigger === 'function')
    }

    function capabilities() {
        return immutable({
            profile: PROFILE,
            active: active,
            activation_epoch: activationEpoch,
            fatal_error: fatalError,
            slot_count: SLOT_COUNT,
            metadata_only: true,
            complete: false,
            limits: { aliases: MAX_ALIASES, raw_string_length: MAX_STRING_LENGTH,
                pending_feedback: MAX_PENDING_FEEDBACK },
            configs: configs.map(function (config) {
                return { config_id: config.id, requested_channel_kind: config.kind,
                    follow_visibility: false, window_zones: 'all_requested',
                    explicit_main_filter: config.explicitMain,
                    host_id_supported: config.slots.map(function (slot) { return slot.idSupported }),
                    actions: { reset: hasAction(config, 'mResetBank'),
                        next: hasAction(config, 'mNextBank'), prev: hasAction(config, 'mPrevBank') } }
            })
        })
    }

    function activate(device, mapping) {
        healthy(false)
        if (!device || !mapping) throw fixedError('INVALID_ARGUMENT')
        activationEpoch = increment(activationEpoch)
        activeDevice = device
        activeMapping = mapping
        active = true
        titleAliases = []
        hostAliases = []
        pendingFeedback = []
        configs.forEach(reset)
        return immutable({ activation_epoch: activationEpoch })
    }

    function deactivate() {
        active = false
        activeDevice = null
        activeMapping = null
        titleAliases = []
        hostAliases = []
        pendingFeedback = []
        configs.forEach(function (config) {
            config.slots.forEach(function (slot) { slot.state = blank(slot) })
        })
    }

    function snapshot(id) {
        healthy(true)
        var config = lookup(id)
        var epoch = activationEpoch
        var generation = config.generation
        var observation = observationId
        var mapping = activeMapping
        var rawIds = []
        function stable() {
            healthy(true)
            if (epoch !== activationEpoch || generation !== config.generation || observation !== observationId) {
                throw fixedError('BUSY')
            }
        }
        var items = config.slots.map(function (slot) {
            var item = copy(slot.state)
            if (!slot.idSupported) return item
            var value
            try { value = slot.channel.getUniqueIDString(mapping) }
            catch (error) { value = null }
            // Stop before touching another slot if a getter reentered lifecycle
            // or feedback; don't mix observations from different contexts.
            stable()
            if (typeof value !== 'string') item.host_id_status = 'error'
            else if (value === '') item.host_id_status = 'empty'
            else {
                if (value.length > MAX_STRING_LENGTH) fail('OVERFLOW')
                rawIds.push({ item: item, value: value })
                item.host_id_status = 'supported'
            }
            return item
        })
        stable()
        rawIds.forEach(function (entry) {
            entry.item.host_id_alias = alias(hostAliases, entry.value, 'host-')
        })
        return immutable({ profile: PROFILE, config_id: config.id, activation_epoch: epoch,
            generation: generation, observation_id: observation, items: items,
            metadata_only: true, complete: false })
    }

    function navigate(id, action) {
        healthy(true)
        var config = lookup(id)
        var name = action === 'reset' ? 'mResetBank' : action === 'next' ? 'mNextBank' :
            action === 'prev' ? 'mPrevBank' : null
        if (!name) throw fixedError('INVALID_ARGUMENT')
        if (!hasAction(config, name)) throw fixedError('NOT_SUPPORTED')
        reset(config)
        var epoch = activationEpoch
        var generation = config.generation
        try { config.zone.mAction[name].trigger(activeMapping) }
        catch (error) { fail('INTERNAL_ERROR') }
        healthy(true)
        if (epoch !== activationEpoch || generation !== config.generation) throw fixedError('BUSY')
        return immutable({ config_id: config.id, action: action,
            activation_epoch: activationEpoch, generation: config.generation })
    }

    return { activate: activate, deactivate: deactivate, capabilities: capabilities,
        snapshot: snapshot, navigate: navigate }
}

module.exports = { createProfile: createProfile }
