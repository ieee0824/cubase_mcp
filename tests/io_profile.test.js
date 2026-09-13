'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')
const fs = require('node:fs')
const path = require('node:path')
const modulePath = '../cubase/midi_remote/CubaseMCPIOProbe/CubaseMCPIOProbe/io-profile.js'
const { createProfile } = require(modulePath)
const INPUT = 'IO_INPUT_ALL'
const OUTPUT = 'IO_OUTPUT_ALL'
const CANARY = 'credential=PRIVATE_日本語_🎹'

function harness(options = {}) {
    const zones = [], events = [], calls = []
    const device = {}, mapping = {}
    let onEmit = options.onEmit
    const mixConsole = {
        makeMixerBankZone(id) {
            const zone = { id, channels: [], mAction: {} }
            for (const kind of ['Audio', 'Instrument', 'Sampler', 'MIDI', 'FX', 'Group', 'VCA', 'Input', 'Output']) {
                for (const verb of ['include', 'exclude']) {
                    zone[`${verb}${kind}Channels`] = () => { calls.push([id, `${verb}${kind}Channels`]); return zone }
                }
            }
            for (const area of ['Left', 'Right', ...(options.old ? [] : ['Main'])]) {
                zone[`includeWindowZone${area}Channels`] = () => { calls.push([id, `includeWindowZone${area}Channels`]); return zone }
            }
            zone.setFollowVisibility = value => { calls.push([id, 'setFollowVisibility', value]); return zone }
            zone.makeMixerBankChannel = () => {
                const channel = { idValue: `${id}-${zone.channels.length}` }
                Object.defineProperty(channel, 'mValue', { get() { throw new Error('Forbidden host value access') } })
                if (!options.old) channel.getUniqueIDString = actualMapping => {
                    assert.equal(actualMapping, mapping)
                    return channel.idValue
                }
                zone.channels.push(channel)
                return channel
            }
            for (const name of ['Reset', 'Next', 'Prev']) {
                zone.mAction[`m${name}Bank`] = { trigger(actualMapping) {
                    calls.push([id, name, actualMapping])
                    if (zone.trigger) zone.trigger(name, actualMapping)
                } }
            }
            zones.push(zone)
            return zone
        }
    }
    const profile = createProfile(mixConsole, event => {
        events.push(event)
        return onEmit ? onEmit(event) : true
    })
    return { profile, zones, events, calls, device, mapping,
        activate() { return profile.activate(device, mapping) },
        title(slot, title, zone = 0) { zones[zone].channels[slot].mOnTitleChange(device, mapping, title) },
        setEmitter(value) { onEmit = value } }
}

function expectCode(fn, code) {
    assert.throws(fn, error => error.code === code && error.message === code && !String(error).includes(CANARY))
}

test('two explicit 8-slot read-only filters, all zones requested, no completeness claims', () => {
    const h = harness()
    const caps = h.profile.capabilities()
    assert.equal(caps.profile, 'io-existing-v1')
    assert.equal(caps.complete, false)
    assert.equal(caps.metadata_only, true)
    assert.equal(h.zones.length, 2)
    for (const [index, kind] of ['Input', 'Output'].entries()) {
        const zone = h.zones[index], calls = h.calls.filter(call => call[0] === zone.id)
        assert.equal(zone.channels.length, 8)
        assert.deepEqual(calls.filter(call => call[1].startsWith('include') && !call[1].includes('Window')), [[zone.id, `include${kind}Channels`]])
        assert.equal(calls.filter(call => call[1].startsWith('exclude')).length, 8)
        assert.deepEqual(calls.slice(-4), ['Left', 'Right', 'Main'].map(area => [zone.id, `includeWindowZone${area}Channels`]).concat([[zone.id, 'setFollowVisibility', false]]))
        assert.equal(caps.configs[index].window_zones, 'all_requested')
        assert.equal(caps.configs[index].explicit_main_filter, true)
    }
    assert.ok(!h.calls.some(call => ['Next', 'Prev', 'Reset'].includes(call[1])))
})

test('inactive reads rejected; unknown config/action and missing action fail safely', () => {
    const h = harness()
    expectCode(() => h.profile.snapshot(INPUT), 'NOT_CONNECTED')
    expectCode(() => h.profile.activate(null, {}), 'INVALID_ARGUMENT')
    h.activate()
    expectCode(() => h.profile.snapshot(CANARY), 'INVALID_ARGUMENT')
    expectCode(() => h.profile.navigate(INPUT, CANARY), 'INVALID_ARGUMENT')
    delete h.zones[0].mAction.mNextBank
    expectCode(() => h.profile.navigate(INPUT, 'next'), 'NOT_SUPPORTED')
})

test('old API distinguishes unobserved and empty title from unsupported host ID', () => {
    const h = harness({ old: true })
    h.activate()
    assert.equal(h.profile.capabilities().configs[0].explicit_main_filter, false)
    const first = h.profile.snapshot(INPUT).items[0]
    assert.deepEqual([first.title_observed, first.title_state, first.host_id_status], [false, 'unobserved', 'unsupported'])
    h.title(0, '')
    const empty = h.profile.snapshot(INPUT).items[0]
    assert.deepEqual([empty.title_observed, empty.title_state, empty.title_alias], [true, 'empty', null])
    assert.equal(h.profile.snapshot(INPUT).complete, false)
})

test('duplicate/unicode/private titles and IDs leave only aliases; snapshots are immutable', () => {
    const h = harness()
    h.activate()
    h.title(0, CANARY); h.title(1, CANARY); h.title(2, '__proto__')
    h.zones[0].channels[0].idValue = CANARY
    h.zones[0].channels[1].idValue = CANARY
    const snap = h.profile.snapshot(INPUT)
    assert.equal(snap.items[0].title_alias, snap.items[1].title_alias)
    assert.notEqual(snap.items[0].title_alias, snap.items[2].title_alias)
    assert.equal(snap.items[0].host_id_alias, snap.items[1].host_id_alias)
    assert.equal(h.events[0].data.item.host_id_status, 'unobserved')
    assert.deepEqual(h.events.map(event => event.data.observation_id), [1, 2, 3])
    assert.ok(!JSON.stringify([snap, h.events, h.profile.capabilities()]).includes(CANARY))
    assert.ok(Object.isFrozen(snap.items[0]))
    assert.throws(() => { snap.items[0].title_alias = 'raw' }, TypeError)
    h.title(0, 'later')
    assert.equal(snap.items[0].title_alias, 'title-1')
})

test('ID empty, exception, invalid value and absence stay distinct from title feedback', () => {
    const h = harness(); h.activate()
    const channels = h.zones[0].channels
    channels[0].idValue = ''
    channels[1].getUniqueIDString = () => { throw new Error(CANARY) }
    channels[2].idValue = {}
    const snap = h.profile.snapshot(INPUT)
    assert.deepEqual(snap.items.slice(0, 4).map(item => item.host_id_status), ['empty', 'error', 'error', 'supported'])
    assert.ok(snap.items.every(item => !item.title_observed))
    assert.ok(!JSON.stringify(snap).includes(CANARY))
})

test('navigation clears only selected bank; old generation callbacks do not contaminate it', () => {
    const h = harness(); h.activate(); h.title(0, 'input'); h.title(0, 'output', 1)
    const old = h.zones[0].channels[0].mOnTitleChange
    const first = h.profile.snapshot(INPUT)
    for (const action of ['next', 'prev', 'reset']) h.profile.navigate(INPUT, action)
    old(h.device, h.mapping, CANARY)
    const next = h.profile.snapshot(INPUT)
    assert.equal(next.generation, first.generation + 3)
    assert.equal(next.items[0].title_state, 'unobserved')
    assert.equal(h.profile.snapshot(OUTPUT).items[0].title_state, 'nonempty')
    assert.equal(h.events.at(-1).data.reason, 'generation')
    assert.ok(!JSON.stringify(h.events).includes(CANARY))
})

test('navigation preserves synchronous callbacks from the newly installed handler', () => {
    const h = harness(); h.activate()
    h.zones[0].trigger = () => h.title(0, 'new bank')
    const action = h.profile.navigate(INPUT, 'next')
    assert.equal(h.profile.snapshot(INPUT).items[0].title_state, 'nonempty')
    assert.equal(h.events.at(-1).data.generation, action.generation)
})

test('deactivation clears aliases; stale device/mapping and old activation callbacks are ignored', () => {
    const h = harness(); h.activate(); h.title(0, 'before')
    const old = h.zones[0].channels[0].mOnTitleChange
    h.profile.deactivate(); old(h.device, h.mapping, CANARY)
    assert.equal(h.events.at(-1).data.reason, 'inactive')
    const activation = h.activate()
    old(h.device, h.mapping, CANARY)
    h.zones[0].channels[0].mOnTitleChange({}, h.mapping, CANARY)
    h.zones[0].channels[0].mOnTitleChange(h.device, {}, CANARY)
    assert.equal(h.profile.snapshot(INPUT).items[0].title_state, 'unobserved')
    h.title(0, 'after')
    assert.equal(h.profile.snapshot(INPUT).items[0].title_alias, 'title-1')
    assert.equal(activation.activation_epoch, 2)
    assert.ok(!JSON.stringify(h.events).includes(CANARY))
})

test('snapshot rejects callbacks and lifecycle changes during ID reads', () => {
    for (const change of ['callback', 'navigation', 'activation', 'deactivation']) {
        const h = harness(); h.activate()
        let laterReads = 0
        h.zones[0].channels[1].getUniqueIDString = () => { laterReads++; return CANARY }
        h.zones[0].channels[0].getUniqueIDString = () => {
            if (change === 'callback') h.title(1, CANARY)
            if (change === 'navigation') h.profile.navigate(INPUT, 'next')
            if (change === 'activation') h.activate()
            if (change === 'deactivation') h.profile.deactivate()
            return CANARY
        }
        expectCode(() => h.profile.snapshot(INPUT), change === 'deactivation' ? 'NOT_CONNECTED' : 'BUSY')
        assert.equal(laterReads, 0)
    }
})

test('navigation does not acknowledge an action across a lifecycle transition', () => {
    const h = harness(); h.activate()
    h.zones[0].trigger = () => h.activate()
    expectCode(() => h.profile.navigate(INPUT, 'next'), 'BUSY')
})

test('host action and emitter exceptions never expose host strings', () => {
    const h = harness(); h.activate()
    h.zones[0].trigger = () => { throw new Error(CANARY) }
    expectCode(() => h.profile.navigate(INPUT, 'reset'), 'INTERNAL_ERROR')
    const emitter = harness({ onEmit() { throw new Error(CANARY) } }); emitter.activate(); emitter.title(0, CANARY)
    expectCode(() => emitter.profile.snapshot(INPUT), 'INTERNAL_ERROR')
})

test('title and host alias cardinality/length are bounded and overflow remains fatal', () => {
    for (const mode of ['title-count', 'title-length', 'id-count', 'id-length']) {
        const h = harness(); h.activate()
        if (mode === 'title-count') for (let i = 0; i < 257; i++) h.title(0, `title-${i}`)
        if (mode === 'title-length') h.title(0, 'x'.repeat(4097))
        if (mode === 'id-length') {
            h.zones[0].channels[0].idValue = 'x'.repeat(4097)
            expectCode(() => h.profile.snapshot(INPUT), 'OVERFLOW')
        }
        if (mode === 'id-count') {
            for (let i = 0; i < 32; i++) {
                h.zones[0].channels.forEach((channel, slot) => { channel.idValue = `${i}-${slot}` })
                h.profile.snapshot(INPUT)
            }
            h.zones[0].channels[0].idValue = 'overflow'
            expectCode(() => h.profile.snapshot(INPUT), 'OVERFLOW')
        }
        expectCode(() => h.profile.snapshot(INPUT), 'OVERFLOW')
        h.profile.deactivate()
        expectCode(() => h.activate(), 'OVERFLOW')
    }
})

test('reentrant or rejected feedback queue fails closed without unbounded recursion', () => {
    for (const mode of ['reentry', 'reject', 'flood']) {
        const h = harness(); h.activate()
        let once = true
        h.setEmitter(() => {
            if (mode === 'reject') return false
            if (mode === 'reentry') h.title(0, 'same')
            if (mode === 'flood' && once) { once = false; for (let i = 0; i < 129; i++) h.title(0, 'same') }
            return true
        })
        h.title(0, 'same')
        expectCode(() => h.profile.snapshot(INPUT), 'OVERFLOW')
        assert.ok(h.events.length <= 128)
    }
})

test('module stays ES5 and never binds host values or exposes project/transport/bus commands', () => {
    const source = fs.readFileSync(path.join(__dirname, modulePath), 'utf8')
    assert.doesNotMatch(source, /\b(?:const|let|class)\b|=>|\bconsole\s*\.|\.mValue\b|makeValueBinding|mTransport|mProject|setProcessValue/)
    assert.deepEqual(Object.keys(harness().profile).sort(), ['activate', 'capabilities', 'deactivate', 'navigate', 'snapshot'])
})
