'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const directory = path.join(__dirname, '..', '..', 'cubase', 'midi_remote', 'CubaseMCPIOProbe', 'CubaseMCPIOProbe')
const wire = require(path.join(directory, 'wire'))
const source = fs.readFileSync(path.join(directory, 'CubaseMCPIOProbe_CubaseMCPIOProbe.js'), 'utf8')
const canary = 'PRIVATE_BUS_SECRET_日本語'

function harness({ oldApi = false, getterThrows = false, missingFilter = false,
    autoActivate = true, onFrame = () => {} } = {}) {
    const messages = [], zones = [], navigations = [], ports = []
    const input = {}, output = { sendMidi(device, frame) {
        const decoded = wire.decode(frame, 2048)
        assert.ok(decoded, 'bounded ASCII CMTP output')
        messages.push(decoded)
        onFrame(Array.from(frame))
    } }
    const mapping = {}, device = {}, page = { mHostAccess: { mMixConsole: {
        makeMixerBankZone(name) {
            const zone = { name, slots: [], mAction: {} }
            for (const kind of ['Audio', 'Instrument', 'Sampler', 'MIDI', 'FX', 'Group', 'VCA', 'Input', 'Output']) {
                zone[`include${kind}Channels`] = zone[`exclude${kind}Channels`] = () => zone
            }
            zone.includeWindowZoneLeftChannels = zone.includeWindowZoneRightChannels = () => zone
            if (!oldApi) zone.includeWindowZoneMainChannels = () => zone
            if (missingFilter) delete zone.includeInputChannels
            zone.setFollowVisibility = value => { assert.equal(value, false); return zone }
            zone.makeMixerBankChannel = () => {
                const channel = {}
                Object.defineProperty(channel, 'mValue', { get() { throw new Error('host values are forbidden') } })
                if (!oldApi) channel.getUniqueIDString = () => {
                    if (getterThrows) throw new Error(canary)
                    return `${canary}-id-${name}-${zone.slots.indexOf(channel)}`
                }
                zone.slots.push(channel)
                return channel
            }
            for (const action of ['Reset', 'Next', 'Prev']) zone.mAction[`m${action}Bank`] = {
                trigger(actualMapping) {
                    assert.equal(actualMapping, mapping)
                    navigations.push([name, action])
                    zone.slots.forEach(slot => slot.mOnTitleChange(device, mapping, canary))
                }
            }
            zones.push(zone)
            return zone
        }
    } } }
    const detection = { detectPortPair() { return this },
        expectInputNameContains(value) { ports.push(value); return this },
        expectOutputNameContains(value) { ports.push(value); return this } }
    const driver = { mPorts: { makeMidiInput: () => input, makeMidiOutput: () => output },
        makeDetectionUnit: () => detection, mMapping: { makePage: () => page } }
    const context = vm.createContext({ require(name) {
        if (name === 'midiremote_api_v1') return { makeDeviceDriver: () => driver }
        if (name === './io-profile' || name === './wire') return require(path.join(directory, name))
        throw new Error('unexpected dependency')
    } })
    vm.runInContext(source, context)
    const activate = () => { driver.mOnActivate(device); page.mOnActivate(device, mapping) }
    if (autoActivate) activate()
    const idle = () => page.mOnIdle(device, mapping)
    const instance = () => messages[0].source_instance_id
    let request = 0
    function call(method, params = {}, options = {}) {
        const id = options.id || `request-${++request}`
        input.mOnSysex(device, wire.encode({ probe_transport_version: 1,
            target_instance_id: options.target === undefined ?
                (method === 'probe.discover' ? null : instance()) : options.target,
            message: { version: 1, type: 'request', id, method, params } }, 4096))
        return id
    }
    return { messages, zones, navigations, ports, page, device, mapping, idle, instance, call, input, context, activate }
}

module.exports = { harness, wire, canary, directory }
