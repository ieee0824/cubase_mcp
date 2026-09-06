'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const { harness, wire, canary } = require('./helpers/io_probe_harness')

test('dedicated profile becomes request-ready without implying complete metadata', () => {
    const h = harness()
    h.idle()
    assert.deepEqual(h.ports, ['Cubase MCP IO Probe To Cubase', 'Cubase MCP IO Probe From Cubase'])
    assert.deepEqual(h.messages.map(e => e.message.event),
        ['probe.loaded', 'probe.mapping_active', 'probe.capabilities', 'probe.ready'])
    for (const envelope of h.messages.filter(e => e.message.event !== 'probe.capabilities')) {
        const data = envelope.message.data
        assert.equal(data.probe_session_id, h.instance())
        assert.equal(data.protocol_version, 1)
        assert.equal(data.read_only, true)
    }
    const caps = h.messages[2].message.data
    assert.equal(caps.bus_mutation, false)
    assert.equal(caps.complete, false)
    assert.equal(caps.profile, 'io-existing-v1')
    assert.equal(h.messages[3].message.data.initial_snapshots_complete, true)
    h.call('probe.discover')
    assert.equal(h.messages.at(-1).message.result.ready, true)
})

test('response precedes 4 complete chunks; no host title, ID or exception is serialized', () => {
    const h = harness()
    h.zones.forEach(zone => zone.slots.forEach(slot => slot.mOnTitleChange(h.device, h.mapping, canary)))
    h.idle()
    for (const config_id of ['IO_INPUT_ALL', 'IO_OUTPUT_ALL']) {
        const id = h.call('probe.bank.snapshot', { config_id })
        assert.equal(h.messages.at(-1).message.id, id)
        assert.equal(h.messages.at(-1).message.result.scheduled, true)
        h.idle()
        const chunks = h.messages.slice(-4).map(e => e.message.data)
        assert.deepEqual(chunks.map(c => c.chunk_index), [0, 1, 2, 3])
        assert.deepEqual(chunks.map(c => c.snapshot_complete), [false, false, false, true])
        assert.deepEqual(chunks.flatMap(c => c.items.map(i => i.slot_index)), [0, 1, 2, 3, 4, 5, 6, 7])
        assert.ok(chunks.every(c => c.complete === false && c.total_items === 8 && c.items.length === 2))
        assert.ok(chunks.flatMap(c => c.items).every(i => i.config_id === config_id && /^host-\d+$/.test(i.host_id_alias)))
    }
    assert.deepEqual(h.messages.map(e => e.source_seq), h.messages.map((_, i) => i + 1))
    assert.ok(!JSON.stringify(h.messages).includes('PRIVATE_BUS_SECRET'))
    assert.ok(!JSON.stringify(h.messages).includes('日本語'))
})

test('old API does not invent IDs, title callbacks or empty channels', () => {
    const h = harness({ oldApi: true })
    h.idle()
    h.call('probe.bank.snapshot', { config_id: 'IO_INPUT_ALL' })
    h.idle()
    const item = h.messages.at(-1).message.data.items[0]
    assert.equal(item.title_state, 'unobserved')
    assert.equal(item.title_observed, false)
    assert.equal(item.host_id_status, 'unsupported')
    assert.equal(item.host_id_alias, null)
})

test('only dedicated bank navigation is exposed; duplicates and concurrent calls do not execute twice', () => {
    const h = harness()
    h.idle()
    for (const method of ['transport.play', 'probe.observation.cut', 'bus.create', 'probe.bank.rename']) {
        h.call(method)
        assert.equal(h.messages.at(-1).message.error.code, 'NOT_SUPPORTED')
    }
    h.call('probe.bank.next', { config_id: 'IO_INPUT_ALL', value: 1 })
    assert.equal(h.messages.at(-1).message.error.code, 'INVALID_ARGUMENT')
    h.call('probe.bank.next', { config_id: 'IO_INPUT_ALL' }, { id: 'same' })
    h.call('probe.bank.next', { config_id: 'IO_INPUT_ALL' }, { id: 'same' })
    assert.equal(h.messages.at(-1).message.error.code, 'INVALID_ARGUMENT')
    h.call('probe.bank.next', { config_id: 'IO_INPUT_ALL' })
    assert.equal(h.messages.at(-1).message.error.code, 'BUSY')
    h.call('probe.bank.next', { config_id: 'IO_OUTPUT_ALL' }, { target: 'different-instance' })
    assert.deepEqual(h.navigations, [['IO_INPUT_ALL', 'Next']])
    h.idle()
    assert.equal(h.messages.at(-1).message.data.reason, 'command_next')
})

test('callback overflow and pending deactivation fail closed without host error text', () => {
    const h = harness()
    h.zones[0].slots[0].mOnTitleChange(h.device, h.mapping, canary.repeat(300))
    h.idle()
    assert.equal(h.messages.at(-1).message.event, 'probe.overflow')
    assert.ok(!JSON.stringify(h.messages).includes('PRIVATE_BUS_SECRET'))
    const active = harness()
    active.idle()
    active.call('probe.bank.snapshot', { config_id: 'IO_INPUT_ALL' })
    active.page.mOnDeactivate(active.device, active.mapping)
    assert.ok(active.messages.some(e => e.message.event === 'probe.overflow'))
    assert.ok(!active.messages.some(e => e.message.event === 'probe.bank.chunk'))
})

test('unsupported filters are not advertised as a ready working profile', () => {
    const h = harness({ missingFilter: true })
    h.idle()
    h.call('probe.discover')
    assert.equal(h.messages.at(-1).message.result.ready, false)
    h.call('probe.capabilities.get')
    assert.equal(h.messages.at(-1).message.result.fatal_error, 'NOT_SUPPORTED')
})

test('wire rejects foreign, oversized, non-ASCII, fractional and malformed frames', () => {
    for (const frame of [[], [0xF0, 0x7E, 0x7F, 6, 1, 0xF7]]) assert.equal(wire.decode(frame, 4096), null)
    const frame = wire.encode({ test: 1 }, 4096)
    assert.deepEqual(wire.decode(frame, 4096), { test: 1 })
    assert.equal(wire.decode(frame, 1), null)
    for (const value of [-1, 8, 1.5, '1', NaN, Infinity]) {
        const bad = frame.slice(); bad[7] = value
        assert.equal(wire.decode(bad, 4096), null)
    }
    // 'p' is 0x70: coercing an invalid low nibble to zero would otherwise
    // reconstruct an unchanged valid JSON key and accidentally admit the frame.
    const lowNibble = wire.encode({ p: 1 }, 4096)
    for (const value of [-1, 16, 1.5, '0', NaN, Infinity]) {
        const bad = lowNibble.slice(); bad[12] = value
        assert.equal(wire.decode(bad, 4096), null)
    }
    assert.throws(() => wire.encode({ private: canary }, 4096))
})
