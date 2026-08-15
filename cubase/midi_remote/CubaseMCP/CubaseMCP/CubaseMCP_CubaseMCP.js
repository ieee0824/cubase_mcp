// Cubase MCP MIDI Remote Bridge
// Compatible with Cubase 13 / MIDI Remote API v1.1 (ES5 JavaScript).

var midiremote_api = require('midiremote_api_v1')

var deviceDriver = midiremote_api.makeDeviceDriver(
    'CubaseMCP',
    'CubaseMCP',
    'Cubase MCP contributors'
)

var midiInput = deviceDriver.mPorts.makeMidiInput('Cubase MCP Input')
var midiOutput = deviceDriver.mPorts.makeMidiOutput('Cubase MCP Output')

deviceDriver.makeDetectionUnit()
    .detectPortPair(midiInput, midiOutput)
    .expectInputNameContains('Cubase MCP To Cubase')
    .expectOutputNameContains('Cubase MCP From Cubase')

var page = deviceDriver.mMapping.makePage('Cubase MCP')
var transport = page.mHostAccess.mTransport

// Value bindings give us authoritative transport feedback from Cubase.
var playFeedback = deviceDriver.mSurface.makeCustomValueVariable('Playback State')
var stopFeedback = deviceDriver.mSurface.makeCustomValueVariable('Stop State')
var recordFeedback = deviceDriver.mSurface.makeCustomValueVariable('Record State')

page.makeValueBinding(playFeedback, transport.mValue.mStart).setTypeToggle()
page.makeValueBinding(stopFeedback, transport.mValue.mStop).setTypeToggle()
page.makeValueBinding(recordFeedback, transport.mValue.mRecord).setTypeToggle()

var activeDeviceRef = null
var activeMappingRef = null
var pendingTransportRequest = null
var bridgeState = {
    playing: null,
    recording: null,
    tempo: null,
    position: null
}

var SYSEX_HEADER = [0xF0, 0x7D, 0x43, 0x4D, 0x43, 0x50, 0x01]
var MAX_JSON_BYTES = 65536

deviceDriver.mOnActivate = function (activeDevice) {
    activeDeviceRef = activeDevice
}

deviceDriver.mOnDeactivate = function (activeDevice) {
    sendEvent(activeDevice, 'connection.changed', { connected: false })
    activeDeviceRef = null
    activeMappingRef = null
    pendingTransportRequest = null
}

page.mOnActivate = function (activeDevice, activeMapping) {
    activeDeviceRef = activeDevice
    activeMappingRef = activeMapping
    bridgeState.playing = playFeedback.getProcessValue(activeDevice) >= 0.5
    bridgeState.recording = recordFeedback.getProcessValue(activeDevice) >= 0.5
    sendEvent(activeDevice, 'connection.changed', { connected: true })
    sendTransportEvent(activeDevice)
}

page.mOnDeactivate = function (activeDevice) {
    sendEvent(activeDevice, 'connection.changed', { connected: false })
    activeMappingRef = null
    pendingTransportRequest = null
}

playFeedback.mOnProcessValueChange = function (activeDevice, value) {
    var playing = value >= 0.5
    if (bridgeState.playing !== playing) {
        bridgeState.playing = playing
        sendTransportEvent(activeDevice)
    }
    completePendingTransportRequest(activeDevice)
}

stopFeedback.mOnProcessValueChange = function (activeDevice, value) {
    if (value >= 0.5 && bridgeState.playing !== false) {
        bridgeState.playing = false
        sendTransportEvent(activeDevice)
    }
    completePendingTransportRequest(activeDevice)
}

recordFeedback.mOnProcessValueChange = function (activeDevice, value) {
    var recording = value >= 0.5
    if (bridgeState.recording !== recording) {
        bridgeState.recording = recording
        sendTransportEvent(activeDevice)
    }
    completePendingTransportRequest(activeDevice)
}

transport.mTimeDisplay.mOnChangeTempoBPM = function (activeDevice, activeMapping, tempoBPM) {
    if (typeof tempoBPM === 'number' && isFinite(tempoBPM) && tempoBPM > 0) {
        if (bridgeState.tempo !== tempoBPM) {
            bridgeState.tempo = tempoBPM
            sendEvent(activeDevice, 'tempo.changed', { tempo: tempoBPM })
        }
    }
}

transport.mTimeDisplay.mPrimary.mTransportLocator.mOnChange = function (
    activeDevice,
    activeMapping,
    time,
    format
) {
    bridgeState.position = parseMusicalPosition(time, format)
}

midiInput.mOnSysex = function (activeDevice, midiMessage) {
    var jsonText = decodeFrame(midiMessage)
    if (jsonText === null) {
        return
    }

    var request = null
    try {
        request = JSON.parse(jsonText)
    } catch (error) {
        return
    }

    var requestId = request && typeof request.id === 'string' ? request.id : null
    if (requestId === null) {
        return
    }

    try {
        handleRequest(activeDevice, request)
    } catch (error) {
        sendError(
            activeDevice,
            requestId,
            'INTERNAL_ERROR',
            error && error.message ? String(error.message) : String(error)
        )
    }
}

function handleRequest(activeDevice, request) {
    if (request.version !== 1) {
        sendError(activeDevice, request.id, 'PROTOCOL_ERROR', 'Unsupported bridge protocol version')
        return
    }
    if (request.type !== 'request' || typeof request.method !== 'string') {
        sendError(activeDevice, request.id, 'PROTOCOL_ERROR', 'Invalid bridge request envelope')
        return
    }
    if (request.params === null || typeof request.params !== 'object') {
        sendError(activeDevice, request.id, 'INVALID_ARGUMENT', 'Request params must be an object')
        return
    }
    if (activeMappingRef === null) {
        sendError(activeDevice, request.id, 'NOT_CONNECTED', 'Cubase MIDI Remote mapping is inactive')
        return
    }

    // The daemon serializes requests. A different request therefore means any
    // earlier operation already timed out and its late acknowledgement can be discarded.
    if (pendingTransportRequest !== null && pendingTransportRequest.id !== request.id) {
        pendingTransportRequest = null
    }

    switch (request.method) {
        case 'system.get_status':
            sendResponse(activeDevice, request.id, {
                connected: true,
                project_open: null,
                playing: bridgeState.playing,
                recording: bridgeState.recording,
                tempo: bridgeState.tempo
            })
            return

        case 'transport.get':
            if (bridgeState.playing === null || bridgeState.recording === null) {
                sendError(activeDevice, request.id, 'BUSY', 'Cubase transport state is initializing')
                return
            }
            sendResponse(activeDevice, request.id, {
                playing: bridgeState.playing,
                recording: bridgeState.recording,
                tempo: bridgeState.tempo,
                position: bridgeState.position
            })
            return

        case 'transport.play':
            beginTransportRequest(activeDevice, request.id, 'play')
            return

        case 'transport.stop':
            beginTransportRequest(activeDevice, request.id, 'stop')
            return

        case 'transport.record':
            beginTransportRequest(activeDevice, request.id, 'record')
            return

        case 'capabilities.get':
            sendResponse(activeDevice, request.id, {
                transport: { read: true, write: true },
                tracks: {
                    list: false,
                    select: false,
                    mute: false,
                    solo: false,
                    volume: false,
                    pan: false
                },
                markers: false,
                commands: false,
                audio_analysis: false,
                plugin_parameters: false
            })
            return

        default:
            sendError(
                activeDevice,
                request.id,
                'NOT_SUPPORTED',
                "Bridge method '" + request.method + "' is not supported"
            )
    }
}

function beginTransportRequest(activeDevice, requestId, operation) {
    if (transportRequestIsComplete(operation)) {
        sendResponse(activeDevice, requestId, {})
        return
    }

    pendingTransportRequest = { id: requestId, operation: operation }
    if (operation === 'play') {
        transport.mValue.mStart.increment(activeMappingRef)
    } else if (operation === 'stop') {
        transport.mValue.mStop.increment(activeMappingRef)
    } else {
        transport.mValue.mRecord.increment(activeMappingRef)
    }
}

function completePendingTransportRequest(activeDevice) {
    if (
        pendingTransportRequest === null ||
        !transportRequestIsComplete(pendingTransportRequest.operation)
    ) {
        return
    }

    var requestId = pendingTransportRequest.id
    pendingTransportRequest = null
    sendResponse(activeDevice, requestId, {})
}

function transportRequestIsComplete(operation) {
    if (operation === 'play') {
        return bridgeState.playing === true
    }
    if (operation === 'stop') {
        return bridgeState.playing === false && bridgeState.recording === false
    }
    return bridgeState.recording === true
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
            message: message
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

function sendTransportEvent(activeDevice) {
    sendEvent(activeDevice, 'transport.changed', {
        playing: bridgeState.playing,
        recording: bridgeState.recording
    })
}

function sendMessage(activeDevice, value) {
    var text = JSON.stringify(value)
    var bytes = utf8Encode(text)
    if (bytes.length > MAX_JSON_BYTES) {
        return
    }

    var message = SYSEX_HEADER.slice(0)
    for (var index = 0; index < bytes.length; ++index) {
        message.push((bytes[index] >> 4) & 0x0F)
        message.push(bytes[index] & 0x0F)
    }
    message.push(0xF7)
    midiOutput.sendMidi(activeDevice, message)
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
    if (payloadLength % 2 !== 0 || payloadLength / 2 > MAX_JSON_BYTES) {
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

function parseMusicalPosition(time, format) {
    if (typeof time !== 'string' || typeof format !== 'string') {
        return null
    }

    // Cubase localizes the format label. Bars+Beats itself remains the only
    // built-in display made of three or four dot-separated integer groups.
    var parts = time.match(/^\s*(-?\d+)\s*\.\s*(\d+)\s*\.\s*(\d+)(?:\s*\.\s*(\d+))?\s*$/)
    if (parts === null) {
        return null
    }
    return {
        bars: parseInt(parts[1], 10),
        beats: parseInt(parts[2], 10)
    }
}
