'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')

const scriptPath = path.join(
    __dirname,
    '..',
    'cubase',
    'midi_remote',
    'CubaseMCPTrackProbe',
    'CubaseMCPTrackProbe',
    'CubaseMCPTrackProbe_CubaseMCPTrackProbe.js'
)
const source = fs.readFileSync(scriptPath, 'utf8')
const sysexHeader = [0xF0, 0x7D, 0x43, 0x4D, 0x54, 0x50, 0x01]

function chain() {
    return {
        detectPortPair() { return this },
        expectInputNameContains() { return this },
        expectOutputNameContains() { return this },
        setTypeToggle() { return this }
    }
}

function graphMetadata(graph, objectId) {
    const metadata = graph.metadata || {}
    return metadata[objectId] || {
        uniqueName: `object-${objectId}`,
        uniqueId: `uid-${objectId}`,
        title: `Object ${objectId}`,
        type: `Type ${objectId}`,
        visible: true,
        index: objectId,
        zone: 0
    }
}

function makeDirectAccessMock(options, calls) {
    const graph = options.graph || { base: 0, children: { 0: [] }, metadata: {} }
    const directAccess = {
        mOnObjectChange() {},
        mOnObjectWillBeRemoved() {},
        mOnParameterChange() {},
        activate(activeMapping) {
            calls.directActivate.push(activeMapping)
        },
        update(activeMapping) {
            calls.directUpdate.push(activeMapping)
            if (options.directUpdateThrows) {
                throw new Error('direct update failed')
            }
            if (options.directUpdateObjectChange !== undefined) {
                this.mOnObjectChange(
                    options.activeDeviceForDirectUpdate,
                    activeMapping,
                    options.directUpdateObjectChange
                )
            }
        },
        deactivate(activeMapping) {
            calls.directDeactivate.push(activeMapping)
            if (options.directDeactivateThrows) {
                throw new Error('direct deactivate failed')
            }
        },
        getBaseObjectID(activeMapping) {
            calls.directGetters.push(['base', activeMapping])
            return graph.base
        },
        getNumberOfChildObjects(activeMapping, objectId) {
            calls.directGetters.push(['childCount', objectId])
            if (options.throwChildCountFor === objectId) {
                throw new Error('child count failed')
            }
            if (
                options.childCountOverrides &&
                Object.prototype.hasOwnProperty.call(options.childCountOverrides, objectId)
            ) {
                return options.childCountOverrides[objectId]
            }
            const children = graph.children[objectId] || []
            return children.length
        },
        getChildObjectID(activeMapping, objectId, childIndex) {
            calls.directGetters.push(['child', objectId, childIndex])
            if (
                options.throwChildId &&
                options.throwChildId.objectId === objectId &&
                options.throwChildId.childIndex === childIndex
            ) {
                throw new Error('child ID failed')
            }
            if (
                options.childIdOverrides &&
                options.childIdOverrides[`${objectId}:${childIndex}`] !== undefined
            ) {
                return options.childIdOverrides[`${objectId}:${childIndex}`]
            }
            return graph.children[objectId][childIndex]
        },
        getObjectUniqueName(activeMapping, objectId) {
            calls.directGetters.push(['uniqueName', objectId])
            if (options.throwMetadata) {
                throw new Error(`uniqueName-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).uniqueName
        },
        getObjectUniqueIDString(activeMapping, objectId) {
            calls.directGetters.push(['uniqueId', objectId])
            if (options.throwMetadata) {
                throw new Error(`uniqueId-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).uniqueId
        },
        getObjectTitle(activeMapping, objectId) {
            calls.directGetters.push(['title', objectId])
            if (options.throwMetadata) {
                throw new Error(`title-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).title
        },
        isMixerChannelVisible(activeMapping, objectId) {
            calls.directGetters.push(['visible', objectId])
            if (options.throwMetadata) {
                throw new Error(`visible-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).visible
        },
        getMixerChannelIndex(activeMapping, objectId) {
            calls.directGetters.push(['index', objectId])
            if (options.throwMetadata) {
                throw new Error(`index-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).index
        },
        getMixerChannelZone(activeMapping, objectId) {
            calls.directGetters.push(['zone', objectId])
            if (options.throwMetadata) {
                throw new Error(`zone-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).zone
        }
    }

    if (options.directAccessVersion === '1.3') {
        directAccess.getObjectTypeName = function (activeMapping, objectId) {
            calls.directGetters.push(['type', objectId])
            if (options.throwMetadata) {
                throw new Error(`type-${'x'.repeat(300)}`)
            }
            return graphMetadata(graph, objectId).type
        }
    }
    return directAccess
}

function createHarness(options = {}) {
    const sentMidi = []
    const calls = {
        driverArgs: null,
        midiInputNames: [],
        midiOutputNames: [],
        detectionInputNames: [],
        detectionOutputNames: [],
        bindings: [],
        bankActions: [],
        directFactoryTargets: [],
        directActivate: [],
        directUpdate: [],
        directDeactivate: [],
        directGetters: []
    }
    const surfaceValues = []
    const zones = []
    const activeDevice = { name: 'active-device' }
    const activeMapping = { name: 'active-mapping' }

    function hostValue(name) {
        return Object.freeze({ name })
    }

    function surfaceValue(name) {
        const value = {
            name,
            processValue: 0,
            getProcessValue(device) {
                assert.equal(device, activeDevice)
                return this.processValue
            },
            mOnProcessValueChange() {},
            mOnTitleChange() {}
        }
        surfaceValues.push(value)
        return value
    }

    function bankAction(configId, actionName) {
        return {
            trigger(mapping) {
                calls.bankActions.push({ configId, actionName, mapping })
                if (
                    options.bankActionThrows &&
                    options.bankActionThrows.configId === configId &&
                    options.bankActionThrows.actionName === actionName
                ) {
                    throw new Error('bank action failed')
                }
            }
        }
    }

    function makeBankChannel(configId, slotIndex) {
        const channel = {
            configId,
            slotIndex,
            mValue: {
                mSelected: hostValue(`${configId}-${slotIndex}-selected`),
                mMute: hostValue(`${configId}-${slotIndex}-mute`),
                mSolo: hostValue(`${configId}-${slotIndex}-solo`)
            },
            mOnTitleChange() {}
        }
        if (options.bankUniqueIds) {
            channel.getUniqueIDString = function (mapping) {
                assert.equal(mapping, activeMapping)
                if (options.bankUniqueIdFactory) {
                    return options.bankUniqueIdFactory(configId, slotIndex)
                }
                return `bank-${configId}-${slotIndex}`
            }
        }
        return channel
    }

    function makeZone(configId) {
        const zone = {
            configId,
            filterCalls: [],
            channels: [],
            mAction: {
                mResetBank: bankAction(configId, 'reset'),
                mNextBank: bankAction(configId, 'next'),
                mPrevBank: bankAction(configId, 'prev')
            },
            makeMixerBankChannel() {
                const channel = makeBankChannel(configId, this.channels.length)
                this.channels.push(channel)
                return channel
            }
        }
        const filters = [
            'includeAudioChannels',
            'includeInstrumentChannels',
            'includeMIDIChannels',
            'includeGroupChannels',
            'includeFXChannels',
            'includeVCAChannels',
            'includeInputChannels',
            'includeOutputChannels',
            'excludeAudioChannels',
            'excludeInstrumentChannels',
            'excludeSamplerChannels',
            'excludeMIDIChannels',
            'excludeGroupChannels',
            'excludeFXChannels',
            'excludeVCAChannels',
            'excludeInputChannels',
            'excludeOutputChannels',
            'includeWindowZoneLeftChannels',
            'includeWindowZoneRightChannels',
            'excludeWindowZoneLeftChannels',
            'excludeWindowZoneRightChannels'
        ]
        if (options.explicitMainFilter) {
            filters.push(
                'includeWindowZoneMainChannels',
                'excludeWindowZoneMainChannels'
            )
        }
        for (const filter of filters) {
            zone[filter] = function () {
                this.filterCalls.push([filter])
                return this
            }
        }
        zone.setFollowVisibility = function (followVisibility) {
            this.filterCalls.push(['setFollowVisibility', followVisibility])
            return this
        }
        zones.push(zone)
        return zone
    }

    const mixConsole = {
        makeMixerBankZone(configId) {
            return makeZone(configId)
        }
    }
    const hostAccess = { mMixConsole: mixConsole }
    Object.defineProperty(hostAccess, 'mTransport', {
        get() {
            throw new Error('read-only Track probe accessed Transport')
        }
    })

    let directAccess = null
    if (options.directAccessVersion) {
        options.activeDeviceForDirectUpdate = activeDevice
        directAccess = makeDirectAccessMock(options, calls)
        hostAccess.makeDirectAccess = function (target) {
            calls.directFactoryTargets.push(target)
            return directAccess
        }
    }

    const page = {
        mHostAccess: hostAccess,
        makeValueBinding(surface, host) {
            calls.bindings.push({ surface, host })
            return chain()
        },
        mOnActivate() {},
        mOnDeactivate() {},
        mOnIdle() {}
    }
    const midiInput = { mOnSysex() {} }
    const midiOutput = {
        sendMidi(device, message) {
            assert.equal(device, activeDevice)
            sentMidi.push(message.slice())
        }
    }
    const detectionPair = {
        expectInputNameContains(name) {
            calls.detectionInputNames.push(name)
            return this
        },
        expectOutputNameContains(name) {
            calls.detectionOutputNames.push(name)
            return this
        }
    }
    const driver = {
        mPorts: {
            makeMidiInput(name) {
                calls.midiInputNames.push(name)
                return midiInput
            },
            makeMidiOutput(name) {
                calls.midiOutputNames.push(name)
                return midiOutput
            }
        },
        mMapping: { makePage() { return page } },
        mSurface: { makeCustomValueVariable: surfaceValue },
        makeDetectionUnit() {
            return {
                detectPortPair(input, output) {
                    assert.equal(input, midiInput)
                    assert.equal(output, midiOutput)
                    return detectionPair
                }
            }
        },
        mOnActivate() {},
        mOnDeactivate() {}
    }
    const api = {
        makeDeviceDriver(vendor, device, author) {
            calls.driverArgs = [vendor, device, author]
            return driver
        },
        mDefaults: options.directAccessVersion
            ? { mAppVersion: { getVersionString() { return '15.0.30.287' } } }
            : {}
    }
    const context = vm.createContext({
        require(name) {
            assert.equal(name, 'midiremote_api_v1')
            return api
        }
    })
    vm.runInContext(source, context, { filename: scriptPath })

    function activate() {
        driver.mOnActivate(activeDevice)
        page.mOnActivate(activeDevice, activeMapping)
    }

    function idle(times = 1) {
        for (let index = 0; index < times; index += 1) {
            page.mOnIdle(activeDevice, activeMapping)
        }
    }

    function deactivate() {
        page.mOnDeactivate(activeDevice, activeMapping)
    }

    return {
        context,
        driver,
        page,
        midiInput,
        sentMidi,
        calls,
        zones,
        surfaceValues,
        directAccess,
        activeDevice,
        activeMapping,
        activate,
        idle,
        deactivate
    }
}

function encodeEnvelope(value) {
    const bytes = Buffer.from(JSON.stringify(value), 'utf8')
    const message = sysexHeader.slice()
    for (const byte of bytes) {
        message.push((byte >> 4) & 0x0F, byte & 0x0F)
    }
    message.push(0xF7)
    return message
}

function decodeEnvelope(message) {
    assert.deepEqual(Array.from(message.slice(0, sysexHeader.length)), sysexHeader)
    assert.equal(message.at(-1), 0xF7)
    const bytes = []
    for (let index = sysexHeader.length; index < message.length - 1; index += 2) {
        assert.ok(message[index] <= 0x0F)
        assert.ok(message[index + 1] <= 0x0F)
        bytes.push((message[index] << 4) | message[index + 1])
    }
    assert.ok(bytes.length <= 2048, `outbound JSON frame was ${bytes.length} bytes`)
    return JSON.parse(Buffer.from(bytes).toString('utf8'))
}

function envelopes(harness) {
    return harness.sentMidi.map(decodeEnvelope)
}

function messages(harness) {
    return envelopes(harness).map(envelope => envelope.message)
}

function events(harness, eventName) {
    return messages(harness).filter(message => message.event === eventName)
}

function response(harness, requestId) {
    return messages(harness).find(message => message.id === requestId)
}

function instanceId(harness) {
    return envelopes(harness)[0].source_instance_id
}

function request(harness, id, method, params = {}, target = instanceId(harness)) {
    harness.midiInput.mOnSysex(harness.activeDevice, encodeEnvelope({
        probe_transport_version: 1,
        target_instance_id: target,
        message: {
            version: 1,
            id,
            type: 'request',
            method,
            params
        }
    }))
}

function eventItems(harness, eventName, predicate) {
    const selected = events(harness, eventName).filter(event => predicate(event.data))
    return selected.flatMap(event => event.data.items)
}

function assertSourceSequence(harness) {
    const allEnvelopes = envelopes(harness)
    assert.ok(allEnvelopes.length > 0)
    const source = allEnvelopes[0].source_instance_id
    assert.match(source, /^track-probe-/)
    for (let index = 0; index < allEnvelopes.length; index += 1) {
        assert.equal(allEnvelopes[index].source_instance_id, source)
        assert.equal(allEnvelopes[index].source_seq, index + 1)
    }
}

function resolveHostId(mainItem, allItems) {
    if (mainItem.host_id_raw !== null) {
        return mainItem.host_id_raw
    }
    assert.equal(typeof mainItem.host_id_ref, 'string')
    const fragments = allItems
        .filter(item =>
            item.record_kind === 'host_id_fragment' &&
            item.host_id_ref === mainItem.host_id_ref
        )
        .sort((left, right) => left.fragment_index - right.fragment_index)
    assert.equal(fragments.length, mainItem.host_id_fragment_count)
    assert.ok(fragments.every(fragment =>
        Buffer.byteLength(fragment.fragment, 'utf8') <= 256
    ))
    return fragments.map(fragment => fragment.fragment).join('')
}

function testStaticES5AndIdentity() {
    const withoutComments = source
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/\/\/.*$/gm, '')
    assert.doesNotMatch(withoutComments, /\b(?:const|let|class)\b/)
    assert.doesNotMatch(withoutComments, /=>|\?\.|\?\?/)
    assert.equal((withoutComments.match(/`/g) || []).length, 0)
    assert.doesNotMatch(source, /\.mTransport\b/)
    assert.doesNotMatch(source, /setParameter(?:Process|Display)Value/)
    assert.doesNotThrow(() => new vm.Script(source))

    const harness = createHarness()
    assert.deepEqual(harness.calls.driverArgs, [
        'CubaseMCPTrackProbe',
        'CubaseMCPTrackProbe',
        'Cubase MCP contributors'
    ])
    assert.deepEqual(harness.calls.midiInputNames, ['Cubase MCP Track Probe Input'])
    assert.deepEqual(harness.calls.midiOutputNames, ['Cubase MCP Track Probe Output'])
    assert.deepEqual(harness.calls.detectionInputNames, [
        'Cubase MCP Track Probe To Cubase'
    ])
    assert.deepEqual(harness.calls.detectionOutputNames, [
        'Cubase MCP Track Probe From Cubase'
    ])
    assert.notDeepEqual(sysexHeader, [0xF0, 0x7D, 0x43, 0x4D, 0x43, 0x50, 0x01])
}

function testApi11MixerBanksCommandsAndOverflow() {
    const harness = createHarness()
    assert.equal(harness.zones.length, 5)
    assert.deepEqual(harness.zones.map(zone => zone.configId), [
        'MB_CORE_ALL',
        'MB_CORE_VISIBLE',
        'MB_OPTIONAL_MAIN',
        'MB_OPTIONAL_LEFT',
        'MB_OPTIONAL_RIGHT'
    ])
    assert.deepEqual(harness.zones.map(zone => zone.channels.length), [8, 8, 8, 8, 8])
    assert.equal(harness.calls.bindings.length, 120)

    const expectedFilters = [
        'includeAudioChannels',
        'includeInstrumentChannels',
        'includeMIDIChannels',
        'includeGroupChannels',
        'includeFXChannels',
        'excludeSamplerChannels',
        'excludeVCAChannels',
        'excludeInputChannels',
        'excludeOutputChannels',
        'excludeWindowZoneLeftChannels',
        'excludeWindowZoneRightChannels'
    ]
    assert.deepEqual(
        harness.zones[0].filterCalls.slice(0, -1).map(call => call[0]),
        expectedFilters
    )
    assert.deepEqual(
        harness.zones[1].filterCalls.slice(0, -1).map(call => call[0]),
        expectedFilters
    )
    assert.deepEqual(harness.zones[0].filterCalls.at(-1), ['setFollowVisibility', false])
    assert.deepEqual(harness.zones[1].filterCalls.at(-1), ['setFollowVisibility', true])

    const optionalTypeFilters = [
        'excludeAudioChannels',
        'excludeInstrumentChannels',
        'excludeSamplerChannels',
        'excludeMIDIChannels',
        'excludeGroupChannels',
        'excludeFXChannels',
        'includeVCAChannels',
        'includeInputChannels',
        'includeOutputChannels'
    ]
    assert.deepEqual(
        harness.zones[2].filterCalls.slice(0, -1).map(call => call[0]),
        optionalTypeFilters.concat([
            'excludeWindowZoneLeftChannels',
            'excludeWindowZoneRightChannels'
        ])
    )
    assert.deepEqual(
        harness.zones[3].filterCalls.slice(0, -1).map(call => call[0]),
        optionalTypeFilters.concat([
            'includeWindowZoneLeftChannels',
            'excludeWindowZoneRightChannels'
        ])
    )
    assert.deepEqual(
        harness.zones[4].filterCalls.slice(0, -1).map(call => call[0]),
        optionalTypeFilters.concat([
            'includeWindowZoneRightChannels',
            'excludeWindowZoneLeftChannels'
        ])
    )
    assert.ok(harness.zones.slice(2).every(zone =>
        zone.filterCalls.at(-1)[0] === 'setFollowVisibility' &&
        zone.filterCalls.at(-1)[1] === false
    ))

    harness.activate()
    const loaded = events(harness, 'probe.loaded').at(-1)
    const capability = events(harness, 'probe.capabilities').at(-1)
    assert.equal(loaded.data.probe_session_id, envelopes(harness)[0].source_instance_id)
    assert.equal(events(harness, 'probe.ready').length, 0)
    assert.equal(capability.data.direct_access.supported, false)
    assert.equal(capability.data.mixer_bank.slot_count, 8)
    assert.equal(capability.data.mixer_bank.explicit_main_filter, false)
    assert.deepEqual(Array.from(capability.data.mixer_bank.configs), [
        'MB_CORE_ALL',
        'MB_CORE_VISIBLE',
        'MB_OPTIONAL_MAIN',
        'MB_OPTIONAL_LEFT',
        'MB_OPTIONAL_RIGHT'
    ])

    harness.idle(5)
    const ready = events(harness, 'probe.ready').at(-1)
    assert.equal(ready.data.ready, true)
    assert.equal(ready.data.initial_snapshots_complete, true)
    assert.equal(messages(harness).at(-1).event, 'probe.ready')
    for (const configId of capability.data.mixer_bank.configs) {
        const chunks = events(harness, 'probe.bank.chunk').filter(event =>
            event.data.stream === 'mixer_bank_snapshot' &&
            event.data.config_id === configId &&
            event.data.reason === 'page_activate'
        )
        assert.equal(chunks.length, 4)
        assert.deepEqual(chunks.map(chunk => chunk.data.chunk_index), [0, 1, 2, 3])
        assert.ok(chunks.every(chunk => chunk.data.chunk_count === 4))
        assert.equal(chunks.flatMap(chunk => chunk.data.items).length, 8)
    }

    const slot = harness.context.bankConfigs[0].slots[0]
    slot.channel.mOnTitleChange(
        harness.activeDevice,
        harness.activeMapping,
        'Vocal_日本語_🎛️'
    )
    slot.selected_feedback.mOnTitleChange(
        harness.activeDevice,
        'Vocal_日本語_🎛️',
        'Selected'
    )
    slot.selected_feedback.mOnProcessValueChange(harness.activeDevice, 1, 0)
    slot.mute_feedback.mOnProcessValueChange(harness.activeDevice, 1, 0)
    slot.solo_feedback.mOnProcessValueChange(harness.activeDevice, 0, 1)
    harness.idle()

    const feedback = eventItems(
        harness,
        'probe.bank.chunk',
        data => data.stream === 'mixer_bank_feedback'
    ).slice(-5)
    assert.deepEqual(feedback.map(item => item.changed_field), [
        'title',
        'title',
        'selected',
        'mute',
        'solo'
    ])
    assert.equal(feedback[0].title, 'Vocal_日本語_🎛️')
    assert.deepEqual(feedback.slice(0, 2).map(item => item.callback_source), [
        'mixer_bank_channel',
        'selected_binding'
    ])
    assert.equal(feedback[4].selected, true)
    assert.equal(feedback[4].mute, true)
    assert.equal(feedback[4].solo, false)
    assert.equal(feedback[4].host_id_raw, null)
    assert.deepEqual(feedback.map(item => item.observation_seq), [1, 2, 3, 4, 5])

    const beforeWrongTarget = harness.calls.bankActions.length
    request(
        harness,
        'wrong-target',
        'probe.bank.next',
        { config_id: 'MB_CORE_ALL' },
        'another-instance'
    )
    assert.equal(harness.calls.bankActions.length, beforeWrongTarget)
    assert.equal(response(harness, 'wrong-target'), undefined)

    request(harness, 'reset', 'probe.bank.reset', { config_id: 'MB_CORE_ALL' })
    request(harness, 'next', 'probe.bank.next', { config_id: 'MB_CORE_ALL' })
    request(harness, 'prev', 'probe.bank.prev', { config_id: 'MB_CORE_VISIBLE' })
    assert.deepEqual(
        harness.calls.bankActions.map(call => [call.configId, call.actionName]),
        [
            ['MB_CORE_ALL', 'reset'],
            ['MB_CORE_ALL', 'next'],
            ['MB_CORE_VISIBLE', 'prev']
        ]
    )
    assert.ok(harness.calls.bankActions.every(call => call.mapping === harness.activeMapping))
    assert.equal(response(harness, 'reset').result.action, 'reset')
    assert.equal(response(harness, 'next').result.action, 'next')
    assert.equal(response(harness, 'prev').result.action, 'prev')

    request(harness, 'da-unsupported', 'probe.direct_access.snapshot')
    assert.equal(response(harness, 'da-unsupported').error.code, 'NOT_SUPPORTED')

    for (let index = 0; index < 524; index += 1) {
        slot.channel.mOnTitleChange(
            harness.activeDevice,
            harness.activeMapping,
            `callback-${index}`
        )
    }
    const actionsBeforeOverflowCommand = harness.calls.bankActions.length
    request(harness, 'after-overflow', 'probe.bank.next', {
        config_id: 'MB_CORE_ALL'
    })
    assert.equal(response(harness, 'after-overflow').error.code, 'PROTOCOL_ERROR')
    assert.equal(harness.calls.bankActions.length, actionsBeforeOverflowCommand)
    harness.idle(9)
    const overflow = events(harness, 'probe.overflow').find(event =>
        event.data.stream === 'feedback_queue'
    )
    assert.ok(overflow)
    assert.equal(overflow.data.queue_limit, 512)
    assert.equal(overflow.data.dropped_items, 12)
    assert.equal(overflow.data.dropped_by_stream.mixer_bank_feedback, 12)
    const latestFeedbackChunk = events(harness, 'probe.bank.chunk').filter(event =>
        event.data.stream === 'mixer_bank_feedback'
    ).at(-1)
    assert.equal(latestFeedbackChunk.data.truncated, false)
    assert.equal(latestFeedbackChunk.data.remaining_items, 1)
    assert.ok(latestFeedbackChunk.data.items.length <= 2)

    harness.deactivate()
    assert.equal(events(harness, 'probe.ready').at(-1).data.ready, false)
    assertSourceSequence(harness)
}

function testActivationBurstFitsBoundedFeedbackQueue() {
    const harness = createHarness()
    harness.activate()

    for (const config of harness.context.bankConfigs) {
        for (const slot of config.slots) {
            slot.channel.mOnTitleChange(
                harness.activeDevice,
                harness.activeMapping,
                `${config.id}-${slot.index}`
            )
            slot.selected_feedback.mOnTitleChange(
                harness.activeDevice,
                `${config.id}-${slot.index}`,
                'Selected'
            )
            slot.selected_feedback.mOnProcessValueChange(harness.activeDevice, 0, 0)
            slot.mute_feedback.mOnProcessValueChange(harness.activeDevice, 0, 0)
            slot.solo_feedback.mOnProcessValueChange(harness.activeDevice, 0, 0)
        }
    }

    assert.equal(harness.context.pendingFeedback.length, 200)
    assert.equal(events(harness, 'probe.overflow').length, 0)
    harness.idle(8)

    const feedback = eventItems(
        harness,
        'probe.bank.chunk',
        data => data.stream === 'mixer_bank_feedback'
    )
    assert.equal(feedback.length, 200)
    assert.equal(events(harness, 'probe.overflow').length, 0)
    assert.equal(events(harness, 'probe.ready').at(-1).data.ready, true)
    assert.equal(
        events(harness, 'probe.capabilities').at(-1).data.limits.feedback_queue,
        512
    )
    assertSourceSequence(harness)
}

function testMappingReactivationKeepsSessionAndRequiresFreshReady() {
    const harness = createHarness()
    harness.activate()
    harness.idle(5)
    const sourceId = envelopes(harness)[0].source_instance_id

    harness.deactivate()
    const beforeReactivation = messages(harness).length
    harness.activate()
    assert.equal(events(harness, 'probe.loaded').length, 1)
    assert.equal(events(harness, 'probe.mapping_active').length, 2)
    assert.equal(messages(harness).slice(beforeReactivation)[0].event, 'probe.mapping_active')
    assert.equal(events(harness, 'probe.ready').at(-1).data.ready, false)

    harness.idle(5)
    assert.equal(events(harness, 'probe.ready').at(-1).data.ready, true)
    assert.ok(envelopes(harness).every(envelope =>
        envelope.source_instance_id === sourceId
    ))
    assertSourceSequence(harness)
}

function testBankGenerationDoesNotReusePriorSlotState() {
    const harness = createHarness()
    harness.activate()
    harness.idle(5)

    const slot = harness.context.bankConfigs[0].slots[0]
    slot.channel.mOnTitleChange(
        harness.activeDevice,
        harness.activeMapping,
        'Prior Bank Title'
    )
    slot.selected_feedback.mOnProcessValueChange(harness.activeDevice, 1, 0)
    harness.idle()

    request(harness, 'next-generation', 'probe.bank.next', {
        config_id: 'MB_CORE_ALL'
    })
    harness.idle()

    const snapshotEvents = events(harness, 'probe.bank.chunk').filter(event =>
        event.data.stream === 'mixer_bank_snapshot' &&
        event.data.config_id === 'MB_CORE_ALL' &&
        event.data.reason === 'command_next'
    )
    const snapshotItems = snapshotEvents
        .flatMap(event => event.data.items)
        .filter(item => item.record_kind === 'observation')
    assert.equal(snapshotItems.length, 8)
    assert.equal(snapshotItems[0].bank_generation, 2)
    assert.equal(snapshotItems[0].title, null)
    assert.equal(snapshotItems[0].selected, null)
    assert.equal(snapshotItems[0].field_observation_generation.title, 1)
    assert.equal(snapshotItems[0].field_observation_generation.selected, 1)
    assert.equal(snapshotItems[0].field_last_observation_seq.title, 1)
    assert.equal(snapshotItems[0].field_last_observation_seq.selected, 2)
    assertSourceSequence(harness)
}

function testSnapshotQueuesAreBoundedAndDeactivationIsFailClosed() {
    const bounded = createHarness()
    bounded.activate()
    bounded.idle(5)

    for (let index = 0; index < 33; index += 1) {
        request(bounded, `snapshot-${index}`, 'probe.bank.snapshot', {
            config_id: 'MB_CORE_ALL'
        })
    }
    for (let index = 0; index < 32; index += 1) {
        assert.equal(response(bounded, `snapshot-${index}`).type, 'response')
    }
    assert.equal(response(bounded, 'snapshot-32').type, 'error')
    assert.equal(response(bounded, 'snapshot-32').error.code, 'BUSY')
    assert.equal(bounded.context.pendingBankSnapshots.length, 32)
    bounded.idle(32)
    assert.equal(bounded.context.pendingBankSnapshots.length, 0)
    assertSourceSequence(bounded)

    const deactivation = createHarness()
    deactivation.activate()
    deactivation.idle(5)
    const slot = deactivation.context.bankConfigs[0].slots[0]
    slot.channel.mOnTitleChange(
        deactivation.activeDevice,
        deactivation.activeMapping,
        'pending-before-deactivate'
    )
    deactivation.deactivate()

    const discard = events(deactivation, 'probe.overflow').find(event =>
        event.data.stream === 'deactivation_discard'
    )
    assert.ok(discard)
    assert.equal(discard.data.pending_feedback_items, 1)
    assert.equal(discard.data.discarded_items, 1)
    assert.equal(events(deactivation, 'probe.ready').at(-1).data.ready, false)
    assertSourceSequence(deactivation)

    const lateCallback = createHarness()
    lateCallback.activate()
    lateCallback.idle(5)
    const lateSlot = lateCallback.context.bankConfigs[0].slots[0]
    lateCallback.deactivate()
    for (let index = 0; index < 1000; index += 1) {
        lateSlot.selected_feedback.mOnProcessValueChange(
            lateCallback.activeDevice,
            index % 2,
            0
        )
    }
    const lateOverflows = events(lateCallback, 'probe.overflow').filter(event =>
        event.data.stream === 'post_deactivation_callback'
    )
    assert.equal(lateOverflows.length, 1)
    assert.equal(lateOverflows[0].data.callback_source, 'selected_binding')
    assertSourceSequence(lateCallback)
}

function testBankActionFailureIsFatal() {
    const harness = createHarness({
        bankActionThrows: {
            configId: 'MB_CORE_ALL',
            actionName: 'next'
        }
    })
    harness.activate()
    harness.idle(5)

    request(harness, 'failing-bank-action', 'probe.bank.next', {
        config_id: 'MB_CORE_ALL'
    })
    assert.equal(response(harness, 'failing-bank-action').error.code, 'INTERNAL_ERROR')
    const overflow = events(harness, 'probe.overflow').find(event =>
        event.data.stream === 'bank_action'
    )
    assert.ok(overflow)
    assert.equal(harness.context.pageIsReady, false)
    assert.equal(harness.context.bankConfigs[0].generation, 2)
    assert.equal(harness.context.pendingBankSnapshots.length, 0)

    request(harness, 'after-bank-action-failure', 'probe.bank.reset', {
        config_id: 'MB_CORE_ALL'
    })
    assert.equal(response(harness, 'after-bank-action-failure').error.code, 'PROTOCOL_ERROR')
    assert.equal(harness.calls.bankActions.length, 1)
    assertSourceSequence(harness)
}

function testFeedbackStreamsPreserveCallbackArrivalOrder() {
    const harness = createHarness({
        directAccessVersion: '1.2',
        graph: { base: 0, children: { 0: [] }, metadata: {} }
    })
    harness.activate()
    harness.idle(5)

    const slot = harness.context.bankConfigs[0].slots[0]
    const start = harness.sentMidi.length
    harness.directAccess.mOnObjectChange(
        harness.activeDevice,
        harness.activeMapping,
        0
    )
    slot.selected_feedback.mOnProcessValueChange(harness.activeDevice, 1, 0)
    harness.directAccess.mOnParameterChange(
        harness.activeDevice,
        harness.activeMapping,
        0,
        17
    )
    slot.mute_feedback.mOnProcessValueChange(harness.activeDevice, 1, 0)
    harness.idle()

    const callbackFrames = harness.sentMidi
        .slice(start)
        .map(decodeEnvelope)
        .map(envelope => envelope.message)
        .filter(message =>
            message.type === 'event' &&
            message.data &&
            (
                message.data.stream === 'mixer_bank_feedback' ||
                message.data.stream === 'direct_access_feedback'
            )
        )
    assert.deepEqual(
        callbackFrames.map(frame => frame.data.stream),
        [
            'direct_access_feedback',
            'mixer_bank_feedback',
            'direct_access_feedback',
            'mixer_bank_feedback'
        ]
    )
    assert.deepEqual(
        callbackFrames.flatMap(frame => frame.data.items).map(item => item.observation_seq),
        [1, 2, 3, 4]
    )
    assertSourceSequence(harness)
}

function testApi12DirectAccessCommonMetadataAndLifecycle() {
    const graph = {
        base: 0,
        children: { 0: [1], 1: [] },
        metadata: {
            0: {
                uniqueName: 'root',
                uniqueId: 'root-id',
                title: 'MixConsole',
                type: 'must-not-be-read',
                visible: true,
                index: 0,
                zone: 0
            },
            1: {
                uniqueName: 'track-one',
                uniqueId: 'track-id-1',
                title: 'Audio 01',
                type: 'must-not-be-read',
                visible: false,
                index: 1,
                zone: 0
            }
        }
    }
    const harness = createHarness({
        directAccessVersion: '1.2',
        graph,
        bankUniqueIds: true
    })
    harness.activate()
    harness.idle(5)

    assert.equal(harness.calls.directFactoryTargets.length, 1)
    assert.equal(harness.calls.directFactoryTargets[0], harness.page.mHostAccess.mMixConsole)
    assert.deepEqual(harness.calls.directActivate, [harness.activeMapping])
    assert.deepEqual(harness.calls.directUpdate, [harness.activeMapping])

    const capability = events(harness, 'probe.capabilities').at(-1).data
    assert.equal(capability.direct_access.supported, true)
    assert.equal(capability.direct_access.get_object_unique_name_v1_2, true)
    assert.equal(capability.direct_access.get_object_unique_id_string_v1_2, true)
    assert.equal(capability.direct_access.get_object_title_v1_2, true)
    assert.equal(capability.direct_access.get_object_type_name_v1_3, false)
    assert.equal(capability.mixer_bank.unique_id, true)

    const items = eventItems(
        harness,
        'probe.direct_access.chunk',
        data => data.stream === 'direct_access_snapshot' && data.reason === 'page_activate'
    )
    assert.equal(items.length, 2)
    assert.deepEqual(
        items.map(item => [
            item.unique_name,
            item.host_id_raw,
            item.title,
            item.type_name,
            item.mixer_visible,
            item.mixer_index
        ]),
        [
            ['root', 'root-id', 'MixConsole', null, true, 0],
            ['track-one', 'track-id-1', 'Audio 01', null, false, 1]
        ]
    )
    assert.equal(harness.calls.directGetters.some(call => call[0] === 'type'), false)

    harness.directAccess.mOnObjectChange(
        harness.activeDevice,
        harness.activeMapping,
        1
    )
    harness.idle()
    assert.equal(harness.calls.directUpdate.length, 2)
    const changeItems = eventItems(
        harness,
        'probe.direct_access.chunk',
        data => data.stream === 'direct_access_feedback'
    )
    assert.equal(changeItems.at(-1).change, 'object_change')
    assert.equal(changeItems.at(-1).object_id, 1)

    request(harness, 'da-snapshot', 'probe.direct_access.snapshot')
    assert.deepEqual(response(harness, 'da-snapshot').result, {})
    harness.idle()
    assert.equal(harness.calls.directUpdate.length, 3)

    harness.deactivate()
    assert.deepEqual(harness.calls.directDeactivate, [harness.activeMapping])
    assertSourceSequence(harness)
}

function testDirectAccessEnumerationFailuresAreIncomplete() {
    const countFailure = createHarness({
        directAccessVersion: '1.2',
        throwChildCountFor: 0,
        graph: { base: 0, children: { 0: [1], 1: [] }, metadata: {} }
    })
    countFailure.activate()
    countFailure.idle()
    const countChunks = events(countFailure, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot'
    )
    assert.ok(countChunks.every(event => event.data.truncated === true))
    assert.ok(countChunks[0].data.truncation_reasons.includes('child_count_failed'))
    assert.equal(countChunks[0].data.observation_items, 1)

    const invalidChild = createHarness({
        directAccessVersion: '1.2',
        childIdOverrides: { '0:0': -1 },
        graph: { base: 0, children: { 0: [1], 1: [] }, metadata: {} }
    })
    invalidChild.activate()
    invalidChild.idle()
    const childChunks = events(invalidChild, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot'
    )
    assert.ok(childChunks.every(event => event.data.truncated === true))
    assert.ok(childChunks[0].data.truncation_reasons.includes(
        'invalid_child_object_id'
    ))
    assert.equal(childChunks[0].data.observation_items, 1)

    const invalidCount = createHarness({
        directAccessVersion: '1.2',
        childCountOverrides: { 0: 1.5 },
        graph: { base: 0, children: { 0: [1], 1: [] }, metadata: {} }
    })
    invalidCount.activate()
    invalidCount.idle()
    const invalidCountChunks = events(invalidCount, 'probe.direct_access.chunk').filter(
        event => event.data.stream === 'direct_access_snapshot'
    )
    assert.ok(invalidCountChunks[0].data.truncation_reasons.includes(
        'invalid_child_count'
    ))
    assert.equal(invalidCountChunks[0].data.truncated, true)
}

function testOpaqueHostIdsAreLosslessAndFragmented() {
    const commonPrefix = `opaque-${'同じprefix'.repeat(60)}-`
    const directRootId = commonPrefix + 'direct-root-A'
    const directTrackId = commonPrefix + 'direct-track-B'
    const harness = createHarness({
        directAccessVersion: '1.2',
        bankUniqueIds: true,
        bankUniqueIdFactory(configId, slotIndex) {
            return commonPrefix + configId + '-' + slotIndex
        },
        graph: {
            base: 0,
            children: { 0: [1], 1: [] },
            metadata: {
                0: {
                    uniqueName: 'root',
                    uniqueId: directRootId,
                    title: 'Root',
                    type: 'unused',
                    visible: true,
                    index: 0,
                    zone: 0
                },
                1: {
                    uniqueName: 'track',
                    uniqueId: directTrackId,
                    title: 'Track',
                    type: 'unused',
                    visible: true,
                    index: 1,
                    zone: 0
                }
            }
        }
    })
    harness.activate()
    harness.idle(5)

    const directWireItems = eventItems(
        harness,
        'probe.direct_access.chunk',
        data => data.stream === 'direct_access_snapshot'
    )
    const directMainItems = directWireItems.filter(item =>
        item.record_kind === 'observation'
    )
    assert.equal(directMainItems.length, 2)
    assert.deepEqual(
        directMainItems.map(item => resolveHostId(item, directWireItems)),
        [directRootId, directTrackId]
    )
    assert.deepEqual(
        directMainItems.map(item => item.host_id_byte_length),
        [Buffer.byteLength(directRootId), Buffer.byteLength(directTrackId)]
    )

    const bankWireItems = eventItems(
        harness,
        'probe.bank.chunk',
        data =>
            data.stream === 'mixer_bank_snapshot' &&
            data.config_id === 'MB_CORE_ALL'
    )
    const bankMainItems = bankWireItems.filter(item =>
        item.record_kind === 'observation'
    )
    assert.equal(bankMainItems.length, 8)
    const expectedBankId = commonPrefix + 'MB_CORE_ALL-0'
    assert.equal(resolveHostId(bankMainItems[0], bankWireItems), expectedBankId)
    assert.equal(
        bankMainItems[0].host_id_byte_length,
        Buffer.byteLength(expectedBankId)
    )
    assert.notEqual(
        resolveHostId(directMainItems[0], directWireItems),
        resolveHostId(directMainItems[1], directWireItems)
    )
    assert.equal(events(harness, 'probe.overflow').length, 0)
    assertSourceSequence(harness)
}

function testControlCharacterHostIdsUseFrameSafeFragments() {
    const directRootId = '\u0001'.repeat(200)
    const directTrackId = '\u0001'.repeat(257)
    const harness = createHarness({
        directAccessVersion: '1.2',
        graph: {
            base: 0,
            children: { 0: [1], 1: [] },
            metadata: {
                0: {
                    uniqueName: 'root',
                    uniqueId: directRootId,
                    title: 'Root',
                    type: 'unused',
                    visible: true,
                    index: 0,
                    zone: 0
                },
                1: {
                    uniqueName: 'track',
                    uniqueId: directTrackId,
                    title: 'Track',
                    type: 'unused',
                    visible: true,
                    index: 1,
                    zone: 0
                }
            }
        }
    })
    harness.activate()
    harness.idle(5)

    const wireItems = eventItems(
        harness,
        'probe.direct_access.chunk',
        data => data.stream === 'direct_access_snapshot'
    )
    const mainItems = wireItems.filter(item => item.record_kind === 'observation')
    assert.equal(mainItems.length, 2)
    assert.equal(mainItems[0].host_id_raw, null)
    assert.equal(mainItems[1].host_id_raw, null)
    assert.deepEqual(
        mainItems.map(item => resolveHostId(item, wireItems)),
        [directRootId, directTrackId]
    )
    assert.equal(events(harness, 'probe.overflow').length, 0)
    assertSourceSequence(harness)
}

function testOversizedHostIdsFailClosed() {
    const oversizedId = 'x'.repeat(4097)
    const harness = createHarness({
        directAccessVersion: '1.2',
        graph: {
            base: 0,
            children: { 0: [] },
            metadata: {
                0: {
                    uniqueName: 'root',
                    uniqueId: oversizedId,
                    title: 'Root',
                    type: 'unused',
                    visible: true,
                    index: 0,
                    zone: 0
                }
            }
        }
    })
    harness.activate()
    harness.idle(5)

    const overflow = events(harness, 'probe.overflow').find(event =>
        event.data.stream === 'host_id'
    )
    assert.ok(overflow)
    assert.equal(overflow.data.attempted_bytes, 4097)
    assert.equal(overflow.data.max_bytes, 4096)
    assert.equal(events(harness, 'probe.direct_access.chunk').length, 0)
    assertSourceSequence(harness)
}

function testDirectAccessUpdateCallbackDoesNotStarveSnapshot() {
    const harness = createHarness({
        directAccessVersion: '1.2',
        directUpdateObjectChange: 0,
        graph: { base: 0, children: { 0: [] }, metadata: {} }
    })
    harness.activate()
    harness.idle(8)

    const snapshots = events(harness, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot'
    )
    assert.ok(snapshots.some(event => event.data.reason === 'page_activate'))
    assert.ok(snapshots.some(event => event.data.reason === 'object_change'))
    assert.equal(harness.calls.directUpdate.length, 2)
    assert.equal(events(harness, 'probe.overflow').length, 0)
    assertSourceSequence(harness)
}

function testDirectAccessUpdateFailurePreventsReady() {
    const harness = createHarness({
        directAccessVersion: '1.2',
        directUpdateThrows: true,
        graph: { base: 0, children: { 0: [] }, metadata: {} }
    })
    harness.activate()
    harness.idle(5)

    assert.equal(events(harness, 'probe.direct_access.chunk').length, 0)
    const overflow = events(harness, 'probe.overflow').find(event =>
        event.data.stream === 'direct_access_update'
    )
    assert.ok(overflow)
    const ready = events(harness, 'probe.ready').at(-1)
    assert.equal(ready.data.ready, false)
    assert.equal(ready.data.initialization_failed, true)

    request(harness, 'after-update-failure', 'probe.bank.next', {
        config_id: 'MB_CORE_ALL'
    })
    assert.equal(response(harness, 'after-update-failure').error.code, 'PROTOCOL_ERROR')
    assert.equal(harness.calls.bankActions.length, 0)
    assertSourceSequence(harness)
}

function testDirectAccessDeactivateFailureIsFatal() {
    const harness = createHarness({
        directAccessVersion: '1.2',
        directDeactivateThrows: true,
        graph: { base: 0, children: { 0: [] }, metadata: {} }
    })
    harness.activate()
    harness.idle(5)
    harness.deactivate()

    const overflow = events(harness, 'probe.overflow').find(event =>
        event.data.stream === 'direct_access_deactivate'
    )
    assert.ok(overflow)
    assert.equal(events(harness, 'probe.ready').at(-1).data.ready, false)
    assertSourceSequence(harness)
}

function testApi13TypeCycleAndTraversalBounds() {
    const cyclicHarness = createHarness({
        directAccessVersion: '1.3',
        explicitMainFilter: true,
        graph: {
            base: 0,
            children: { 0: [1], 1: [0] },
            metadata: {
                0: {
                    uniqueName: 'root',
                    uniqueId: 'root-id',
                    title: 'Root',
                    type: 'MixConsole',
                    visible: true,
                    index: 0,
                    zone: 0
                },
                1: {
                    uniqueName: 'track',
                    uniqueId: 'track-id',
                    title: 'Track',
                    type: 'AudioChannel',
                    visible: true,
                    index: 1,
                    zone: 0
                }
            }
        }
    })
    cyclicHarness.activate()
    cyclicHarness.idle()

    const capability = events(cyclicHarness, 'probe.capabilities').at(-1).data
    assert.equal(capability.direct_access.get_object_type_name_v1_3, true)
    assert.equal(capability.mixer_bank.explicit_main_filter, true)
    assert.ok(cyclicHarness.zones.slice(0, 3).every(zone =>
        zone.filterCalls.some(call => call[0] === 'includeWindowZoneMainChannels')
    ))
    assert.ok(cyclicHarness.zones.slice(3).every(zone =>
        zone.filterCalls.some(call => call[0] === 'excludeWindowZoneMainChannels')
    ))

    const cycleEvents = events(cyclicHarness, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot' &&
        event.data.reason === 'page_activate'
    )
    assert.equal(cycleEvents.length, 1)
    assert.equal(cycleEvents[0].data.cycle_count, 1)
    assert.equal(cycleEvents[0].data.items.length, 2)
    assert.deepEqual(
        cycleEvents[0].data.items.map(item => item.type_name),
        ['MixConsole', 'AudioChannel']
    )
    cyclicHarness.deactivate()
    assertSourceSequence(cyclicHarness)

    const children = []
    const childMap = { 0: children }
    for (let index = 1; index <= 200; index += 1) {
        children.push(index)
        childMap[index] = []
    }
    const boundedHarness = createHarness({
        directAccessVersion: '1.3',
        graph: { base: 0, children: childMap, metadata: {} }
    })
    boundedHarness.activate()
    boundedHarness.idle()

    const boundedEvents = events(boundedHarness, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot' &&
        event.data.reason === 'page_activate'
    )
    assert.equal(boundedEvents[0].data.total_items, 129)
    assert.ok(boundedEvents.every(event => event.data.truncated === true))
    assert.ok(boundedEvents[0].data.truncation_reasons.includes('child_count_limit'))
    assert.ok(boundedEvents.every(event => event.data.items.length <= 2))
    assert.equal(boundedEvents.at(-1).data.snapshot_complete, true)
    assertSourceSequence(boundedHarness)

    const largeErrorHarness = createHarness({
        directAccessVersion: '1.3',
        throwMetadata: true,
        graph: { base: 0, children: { 0: [1], 1: [] }, metadata: {} }
    })
    largeErrorHarness.activate()
    largeErrorHarness.idle()
    const errorChunks = events(largeErrorHarness, 'probe.direct_access.chunk').filter(event =>
        event.data.stream === 'direct_access_snapshot' &&
        event.data.reason === 'page_activate'
    )
    assert.equal(errorChunks.length, 2)
    assert.equal(errorChunks[0].data.total_items, 2)
    assert.equal(errorChunks.flatMap(event => event.data.items).length, 2)
    assert.equal(events(largeErrorHarness, 'probe.overflow').length, 0)
    assertSourceSequence(largeErrorHarness)
}

testStaticES5AndIdentity()
testApi11MixerBanksCommandsAndOverflow()
testActivationBurstFitsBoundedFeedbackQueue()
testMappingReactivationKeepsSessionAndRequiresFreshReady()
testBankGenerationDoesNotReusePriorSlotState()
testSnapshotQueuesAreBoundedAndDeactivationIsFailClosed()
testBankActionFailureIsFatal()
testFeedbackStreamsPreserveCallbackArrivalOrder()
testApi12DirectAccessCommonMetadataAndLifecycle()
testDirectAccessEnumerationFailuresAreIncomplete()
testOpaqueHostIdsAreLosslessAndFragmented()
testControlCharacterHostIdsUseFrameSafeFragments()
testOversizedHostIdsFailClosed()
testDirectAccessUpdateCallbackDoesNotStarveSnapshot()
testDirectAccessUpdateFailurePreventsReady()
testDirectAccessDeactivateFailureIsFatal()
testApi13TypeCycleAndTraversalBounds()

console.log('Cubase Track probe script tests passed')
