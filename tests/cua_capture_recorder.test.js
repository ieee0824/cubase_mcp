'use strict'

const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { execFileSync, spawn, spawnSync } = require('node:child_process')
const test = require('node:test')

const root = path.resolve(__dirname, '..')
const recorder = path.join(root, 'scripts', 'record-cua-capture.js')
const temporaryDirectories = []
const onePixelPng = Buffer.from(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFgAI/ScL6qQAAAABJRU5ErkJggg==',
    'base64'
)

function temporaryDirectory() {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'cmcp-cua-capture-test-'))
    temporaryDirectories.push(directory)
    return directory
}

test.after(() => {
    for (const directory of temporaryDirectories) fs.rmSync(directory, { recursive: true, force: true })
})

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

test('rejects unrecognized image signatures before creating capture artifacts', () => {
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

test('records multi-megabyte screenshot bytes without regex stack exhaustion', () => {
    const directory = temporaryDirectory()
    // Synthetic signature-bearing bytes test persistence, not image decoding.
    const bytes = Buffer.alloc(8 * 1024 * 1024, 0x42)
    onePixelPng.copy(bytes)
    const result = JSON.parse(run(directory, 'large', capture({ screenshot_base64: bytes.toString('base64') })))
    assert.deepEqual(fs.readFileSync(path.join(directory, result.screenshot_path)), bytes)
})

test('rejects oversized stdin before EOF without creating artifacts', { timeout: 30000 }, async () => {
    const directory = temporaryDirectory()
    const child = spawn(process.execPath, [recorder, '--output-directory', directory, '--capture-id', 'large-input'])
    let stderr = ''
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk })
    child.stdout.resume()
    // Keep stdin open: a bounded reader must reject without waiting for EOF.
    const chunk = Buffer.alloc(1024 * 1024, 0x20)
    let written = 0
    function feed() {
        while (written < 129) {
            written += 1
            if (!child.stdin.write(chunk)) return
        }
    }
    child.stdin.on('drain', feed)
    child.stdin.on('error', error => {
        if (error.code !== 'EPIPE' && error.code !== 'ERR_STREAM_DESTROYED') throw error
    })
    const finished = new Promise((resolve, reject) => {
        child.once('error', reject)
        child.once('close', (code, signal) => resolve({ code, signal }))
    })
    const timer = setTimeout(() => child.kill(), 10000)
    try {
        feed()
        assert.deepEqual(await finished, { code: 1, signal: null })
        assert.match(stderr, /stdin exceeds the size limit/)
        assert.deepEqual(fs.readdirSync(directory), [])
    } finally {
        clearTimeout(timer)
        child.stdin.destroy()
    }
})

for (const invalidBase64 of ['AAAA!!!!', 'AAAA\nAAA', 'AA=A', 'AB==']) {
    test(`rejects noncanonical base64 ${JSON.stringify(invalidBase64)}`, () => {
        const directory = temporaryDirectory()
        const result = spawnSync(process.execPath, [recorder, '--output-directory', directory, '--capture-id', 'invalid'], {
            encoding: 'utf8', input: JSON.stringify(capture({ screenshot_base64: invalidBase64 }))
        })
        assert.equal(result.status, 1)
        assert.match(result.stderr, /canonical/)
        assert.deepEqual(fs.readdirSync(directory), [])
    })
}

for (const linkedDirectory of ['output', 'states', 'screenshots']) {
    test(`rejects a symbolic link at the ${linkedDirectory} directory`, () => {
        const directory = temporaryDirectory()
        const destination = temporaryDirectory()
        const link = path.join(directory, linkedDirectory)
        fs.symlinkSync(destination, link, process.platform === 'win32' ? 'junction' : 'dir')
        const result = spawnSync(process.execPath, [recorder, '--output-directory', linkedDirectory === 'output' ? link : directory, '--capture-id', 'linked'], {
            encoding: 'utf8', input: JSON.stringify(capture())
        })
        assert.equal(result.status, 1)
        assert.match(result.stderr, /non-symbolic-link directory/)
        assert.deepEqual(fs.readdirSync(destination), [])
    })
}

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
