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
    'CubaseMCP',
    'CubaseMCP',
    'CubaseMCP_CubaseMCP.js'
)

const sentMidi = []
const increments = { play: 0, stop: 0, record: 0 }
const surfaceState = {
    'Playback State': 0,
    'Stop State': 1,
    'Record State': 0
}

function chain() {
    return {
        detectPortPair() { return this },
        expectInputNameContains() { return this },
        expectOutputNameContains() { return this },
        setTypeToggle() { return this }
    }
}

function surfaceValue(name) {
    return {
        getProcessValue() { return surfaceState[name] },
        mOnProcessValueChange() {}
    }
}

const transport = {
    mValue: {
        mStart: { increment() { increments.play += 1 } },
        mStop: { increment() { increments.stop += 1 } },
        mRecord: { increment() { increments.record += 1 } }
    },
    mTimeDisplay: {
        mPrimary: { mTransportLocator: { mOnChange() {} } },
        mOnChangeTempoBPM() {}
    }
}

const midiInput = { mOnSysex() {} }
const midiOutput = { sendMidi(_device, message) { sentMidi.push(message.slice()) } }
const page = {
    mHostAccess: { mTransport: transport },
    makeValueBinding() { return chain() },
    mOnActivate() {},
    mOnDeactivate() {}
}
const driver = {
    mPorts: {
        makeMidiInput() { return midiInput },
        makeMidiOutput() { return midiOutput }
    },
    mMapping: { makePage() { return page } },
    mSurface: { makeCustomValueVariable: surfaceValue },
    makeDetectionUnit: chain,
    mOnActivate() {},
    mOnDeactivate() {}
}
const api = { makeDeviceDriver() { return driver } }

const context = vm.createContext({
    require(name) {
        assert.equal(name, 'midiremote_api_v1')
        return api
    }
})
vm.runInContext(fs.readFileSync(scriptPath, 'utf8'), context, { filename: scriptPath })

const header = [0xF0, 0x7D, 0x43, 0x4D, 0x43, 0x50, 0x01]

function encode(value) {
    const bytes = Buffer.from(JSON.stringify(value), 'utf8')
    const message = header.slice()
    for (const byte of bytes) {
        message.push((byte >> 4) & 0x0F, byte & 0x0F)
    }
    message.push(0xF7)
    return message
}

function decode(message) {
    assert.deepEqual(Array.from(message.slice(0, header.length)), header)
    assert.equal(message.at(-1), 0xF7)
    const bytes = []
    for (let index = header.length; index < message.length - 1; index += 2) {
        bytes.push((message[index] << 4) | message[index + 1])
    }
    return JSON.parse(Buffer.from(bytes).toString('utf8'))
}

function request(id, method) {
    context.midiInput.mOnSysex({}, encode({
        version: 1,
        id,
        type: 'request',
        method,
        params: {}
    }))
}

function messages() {
    return sentMidi.map(decode)
}

function response(id) {
    return messages().find(message => message.id === id)
}

const activeDevice = {}
const activeMapping = {}
context.deviceDriver.mOnActivate(activeDevice)
context.page.mOnActivate(activeDevice, activeMapping)
assert(messages().some(message => message.event === 'connection.changed'))
sentMidi.length = 0

request('status-日本語', 'system.get_status')
assert.equal(response('status-日本語').result.tempo, null)
sentMidi.length = 0

request('play-1', 'transport.play')
assert.equal(increments.play, 1)
assert.equal(response('play-1'), undefined)
surfaceState['Playback State'] = 1
context.playFeedback.mOnProcessValueChange(activeDevice, 1)
assert.deepEqual(response('play-1').result, {})
sentMidi.length = 0

request('play-idempotent', 'transport.play')
assert.equal(increments.play, 1)
assert.deepEqual(response('play-idempotent').result, {})
sentMidi.length = 0

request('stop-1', 'transport.stop')
assert.equal(increments.stop, 1)
assert.equal(response('stop-1'), undefined)
surfaceState['Playback State'] = 0
context.stopFeedback.mOnProcessValueChange(activeDevice, 1)
assert.deepEqual(response('stop-1').result, {})
sentMidi.length = 0

context.transport.mTimeDisplay.mPrimary.mTransportLocator.mOnChange(
    activeDevice,
    activeMapping,
    '32. 2. 1. 0',
    '小節 + 拍'
)
request('transport-1', 'transport.get')
assert.deepEqual(JSON.parse(JSON.stringify(response('transport-1').result.position)), {
    bars: 32,
    beats: 2
})

console.log('Cubase MIDI Remote script tests passed')
