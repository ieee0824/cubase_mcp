'use strict'

const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { execFileSync, spawnSync } = require('node:child_process')
const test = require('node:test')

const root = path.resolve(__dirname, '..')
const recorder = path.join(root, 'scripts', 'record-cua-capture.js')
const onePixelPng = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFgAI/ScL6qQAAAABJRU5ErkJggg==',
    'base64'
)

function temporaryDirectory() {
    return fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-cua-capture-test-'))
}

function capture(overrides = {}) {
    return {
        app: 'Finder',
        captured_at: '2026-09-12T20:48:00.123+09:00',
        text: 'Window: "scratch", App: Finder.\n0 window',
        screenshot_base64: onePixelPng.toString('base64'),
        ...overrides
    }
}

function run(directory, id, payload) {
    return execFileSync(process.execPath, [recorder, '--output-directory', directory, '--capture-id', id], {
        encoding: 'utf8',
        input: JSON.stringify(payload)
    })
}

test('records canonical state and screenshot files without a UI dependency', () => {
    const directory = temporaryDirectory()
    const payload = capture()
    const result = JSON.parse(run(directory, 'finder-pre', payload))
    const screenshotPath = path.join(directory, 'screenshots', 'finder-pre.png')
    const statePath = path.join(directory, 'states', 'finder-pre.json')

    assert.equal(result.screenshot_path, 'screenshots/finder-pre.png')
    assert.equal(result.state_path, 'states/finder-pre.json')
    assert.equal(result.screenshot_sha256, crypto.createHash('sha256').update(onePixelPng).digest('hex'))
    const stateBytes = fs.readFileSync(statePath)
    assert.equal(result.state_sha256, crypto.createHash('sha256').update(stateBytes).digest('hex'))
    assert.deepEqual(JSON.parse(stateBytes), {
        app: payload.app,
        capture_format_version: 1,
        captured_at: payload.captured_at,
        text: payload.text
    })
    assert.deepEqual(fs.readFileSync(screenshotPath), onePixelPng)
})

test('rejects malformed images before creating capture artifacts', () => {
    const directory = temporaryDirectory()
    const result = spawnSync(process.execPath, [recorder, '--output-directory', directory, '--capture-id', 'bad-image'], {
        encoding: 'utf8',
        input: JSON.stringify(capture({ screenshot_base64: Buffer.from('not an image').toString('base64') }))
    })

    assert.equal(result.status, 1)
    assert.match(result.stderr, /neither PNG nor JPEG/)
    assert.equal(fs.existsSync(path.join(directory, 'states')), false)
    assert.equal(fs.existsSync(path.join(directory, 'screenshots')), false)
})

test('rejects invalid timestamps before creating capture artifacts', () => {
    const directory = temporaryDirectory()
    const result = spawnSync(process.execPath, [recorder, '--output-directory', directory, '--capture-id', 'bad-time'], {
        encoding: 'utf8',
        input: JSON.stringify(capture({ captured_at: '2026-02-30T20:48:00.123+09:00' }))
    })

    assert.equal(result.status, 1)
    assert.match(result.stderr, /captured_at/)
    assert.equal(fs.existsSync(path.join(directory, 'states')), false)
    assert.equal(fs.existsSync(path.join(directory, 'screenshots')), false)
})

test('fails closed instead of overwriting an existing capture id', () => {
    const directory = temporaryDirectory()
    run(directory, 'repeat', capture())
    const before = fs.readFileSync(path.join(directory, 'screenshots', 'repeat.png'))
    const result = spawnSync(process.execPath, [recorder, '--output-directory', directory, '--capture-id', 'repeat'], {
        encoding: 'utf8',
        input: JSON.stringify(capture())
    })

    assert.equal(result.status, 1)
    assert.match(result.stderr, /capture id already exists/)
    assert.deepEqual(fs.readFileSync(path.join(directory, 'screenshots', 'repeat.png')), before)
})
