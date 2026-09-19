// CMTP v1 framing for the ASCII-only, privacy-projected I/O probe messages.
// ES5 / MIDI Remote v1.1. No host string is encoded by this module's caller.
var HEADER = [0xF0, 0x7D, 0x43, 0x4D, 0x54, 0x50, 1]

function encode(value, maximum) {
    var text = JSON.stringify(value)
    if (typeof text !== 'string' || text.length > maximum || /[^\x00-\x7F]/.test(text)) {
        throw new Error('INVALID_FRAME')
    }
    var result = HEADER.slice(0)
    for (var i = 0; i < text.length; i++) {
        result.push(text.charCodeAt(i) >> 4, text.charCodeAt(i) & 15)
    }
    result.push(0xF7)
    return result
}

function decode(frame, maximum) {
    if (!frame || frame.length < 8 || frame.length > 8 + maximum * 2 ||
        frame[frame.length - 1] !== 0xF7 || (frame.length - 8) % 2 !== 0) return null
    var i
    for (i = 0; i < HEADER.length; i++) if (frame[i] !== HEADER[i]) return null
    var text = ''
    for (i = HEADER.length; i < frame.length - 1; i += 2) {
        var hi = frame[i], lo = frame[i + 1]
        if (typeof hi !== 'number' || typeof lo !== 'number' ||
            hi < 0 || hi > 7 || lo < 0 || lo > 15 || hi % 1 !== 0 || lo % 1 !== 0) return null
        text += String.fromCharCode(hi * 16 + lo)
    }
    try { return JSON.parse(text) } catch (error) { return null }
}

exports.encode = encode
exports.decode = decode
