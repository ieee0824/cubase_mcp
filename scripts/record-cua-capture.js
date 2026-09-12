#!/usr/bin/env node
'use strict'

// Persist a single capture that was already obtained through Computer Use.
// This utility deliberately has no UI or automation API dependency: callers
// provide the exact AX text and screenshot bytes over stdin.  It writes the
// canonical capture files consumed by the local calibration/evidence checkers
// and emits their relative paths and digests for an operator trace.

const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')

const MAX_APP_BYTES = 1024
const MAX_STATE_BYTES = 4 * 1024 * 1024
const MAX_SCREENSHOT_BYTES = 64 * 1024 * 1024
// Includes base64 expansion, JSON escaping of AX text, and framing overhead.
// Bound input before JSON.parse, including streams which never send EOF.
const MAX_INPUT_BYTES = 128 * 1024 * 1024

function readCaptureInput() {
    const chunks = []
    const buffer = Buffer.alloc(64 * 1024)
    let total = 0
    while (true) {
        const count = fs.readSync(0, buffer, 0, Math.min(buffer.length, MAX_INPUT_BYTES - total + 1), null)
        if (count === 0) break
        total += count
        if (total > MAX_INPUT_BYTES) fail('stdin exceeds the size limit')
        chunks.push(Buffer.from(buffer.subarray(0, count)))
    }
    return Buffer.concat(chunks, total).toString('utf8')
}

function fail(message) {
    throw new Error(`record-cua-capture: ${message}`)
}

function usage() {
    return 'usage: record-cua-capture --output-directory DIRECTORY --capture-id ID'
}

function parseArguments(argv) {
    if (argv.length !== 4 || argv[0] !== '--output-directory' || argv[2] !== '--capture-id') {
        fail(usage())
    }
    const outputDirectory = argv[1]
    const captureId = argv[3]
    if (!path.isAbsolute(outputDirectory)) fail('output directory must be absolute')
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(captureId)) fail('capture id is invalid')
    return { outputDirectory, captureId }
}

function lstatDirectory(directory, label) {
    let stat
    try {
        stat = fs.lstatSync(directory)
    } catch (error) {
        if (error && error.code === 'ENOENT') fail(`${label} does not exist`)
        throw error
    }
    if (stat.isSymbolicLink() || !stat.isDirectory()) fail(`${label} must be a non-symbolic-link directory`)
}

function ensureCaptureDirectory(directory) {
    lstatDirectory(directory, 'output directory')
    for (const name of ['states', 'screenshots']) {
        const child = path.join(directory, name)
        try {
            fs.mkdirSync(child, { mode: 0o700 })
        } catch (error) {
            if (!error || error.code !== 'EEXIST') throw error
        }
        lstatDirectory(child, `${name} directory`)
    }
}

function parseCapture(input) {
    let capture
    try {
        capture = JSON.parse(input)
    } catch {
        fail('stdin must contain one JSON object')
    }
    if (!capture || Array.isArray(capture) || typeof capture !== 'object') fail('capture must be an object')
    const expected = ['app', 'captured_at', 'screenshot_base64', 'text']
    if (Object.keys(capture).sort().join('\u0000') !== expected.join('\u0000')) fail('capture keys are invalid')
    if (typeof capture.app !== 'string' || Buffer.byteLength(capture.app, 'utf8') === 0 ||
        Buffer.byteLength(capture.app, 'utf8') > MAX_APP_BYTES) fail('app must be a bounded non-empty string')
    if (typeof capture.text !== 'string' || Buffer.byteLength(capture.text, 'utf8') === 0 ||
        Buffer.byteLength(capture.text, 'utf8') > MAX_STATE_BYTES) fail('text must be a bounded non-empty string')
    if (!isRfc3339Timestamp(capture.captured_at)) {
        fail('captured_at must be an RFC 3339 timestamp with milliseconds')
    }
    if (typeof capture.screenshot_base64 !== 'string' ||
        capture.screenshot_base64.length === 0 ||
        capture.screenshot_base64.length > 4 * Math.ceil(MAX_SCREENSHOT_BYTES / 3) ||
        capture.screenshot_base64.length % 4 !== 0) {
        fail('screenshot_base64 must be canonical base64')
    }
    const screenshot = Buffer.from(capture.screenshot_base64, 'base64')
    // Decode/re-encode equality rejects invalid alphabet, whitespace, misplaced
    // padding and nonzero pad bits without a repeating-group regex whose stack
    // usage grows with the size of an otherwise valid screenshot.
    if (screenshot.toString('base64') !== capture.screenshot_base64) fail('screenshot_base64 is not canonical')
    if (screenshot.length > MAX_SCREENSHOT_BYTES) fail('screenshot exceeds the size limit')
    return { ...capture, screenshot }
}

function isRfc3339Timestamp(value) {
    if (typeof value !== 'string') return false
    const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.\d{3}(Z|([+-])(\d{2}):(\d{2}))$/.exec(value)
    if (!match) return false
    const [year, month, day, hour, minute, second] = match.slice(1, 7).map(Number)
    const timezoneHour = match[9] === undefined ? 0 : Number(match[9])
    const timezoneMinute = match[10] === undefined ? 0 : Number(match[10])
    if (year < 1970 || month < 1 || month > 12 || hour > 23 || minute > 59 || second > 59 ||
        timezoneHour > 23 || timezoneMinute > 59) return false
    const daysInMonth = month === 2
        ? ((year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0)) ? 29 : 28)
        : ([4, 6, 9, 11].includes(month) ? 30 : 31)
    return day >= 1 && day <= daysInMonth
}

function screenshotExtension(screenshot) {
    if (screenshot.length >= 8 && screenshot.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) return 'png'
    if (screenshot.length >= 3 && screenshot[0] === 0xff && screenshot[1] === 0xd8 && screenshot[2] === 0xff) return 'jpg'
    fail('screenshot is neither PNG nor JPEG')
}

function writeExclusive(file, contents) {
    const descriptor = fs.openSync(file, 'wx', 0o600)
    try {
        fs.writeFileSync(descriptor, contents)
        fs.fsyncSync(descriptor)
    } finally {
        fs.closeSync(descriptor)
    }
}

function sha256(contents) {
    return crypto.createHash('sha256').update(contents).digest('hex')
}

function main() {
    const { outputDirectory, captureId } = parseArguments(process.argv.slice(2))
    const capture = parseCapture(readCaptureInput())
    const extension = screenshotExtension(capture.screenshot)
    ensureCaptureDirectory(outputDirectory)

    const screenshotRelativePath = `screenshots/${captureId}.${extension}`
    const stateRelativePath = `states/${captureId}.json`
    const screenshotPath = path.join(outputDirectory, screenshotRelativePath)
    const statePath = path.join(outputDirectory, stateRelativePath)
    const state = {
        app: capture.app,
        capture_format_version: 1,
        captured_at: capture.captured_at,
        text: capture.text
    }
    const stateBytes = Buffer.from(`${JSON.stringify(state)}\n`, 'utf8')

    // Never overwrite an earlier capture. Checking both paths first avoids
    // adding one half of a capture beside an existing other half; exclusive
    // creation below remains the race-safe enforcement.
    if (fs.existsSync(screenshotPath) || fs.existsSync(statePath)) {
        fail('capture id already exists')
    }
    writeExclusive(screenshotPath, capture.screenshot)
    try {
        writeExclusive(statePath, stateBytes)
    } catch (error) {
        // The screenshot remains visible as an incomplete capture instead of
        // being removed or overwritten. A caller must use a fresh directory.
        throw error
    }

    process.stdout.write(`${JSON.stringify({
        app: capture.app,
        captured_at: capture.captured_at,
        screenshot_path: screenshotRelativePath,
        screenshot_sha256: sha256(capture.screenshot),
        state_path: stateRelativePath,
        state_sha256: sha256(stateBytes)
    })}\n`)
}

try {
    main()
} catch (error) {
    process.stderr.write(`${error && error.message ? error.message : String(error)}\n`)
    process.exitCode = 1
}
